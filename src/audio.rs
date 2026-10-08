//! Audio : son du PC (loopback WASAPI) + micro → timelines QPC → mixeur → MFT AAC → `Ring`.
//!
//! WASAPI date chaque paquet en QPC (« device timestamps » d'OBS) : la timeline place
//! les échantillons sur la même horloge que la vidéo, comble les trous par du silence
//! (la loopback ne livre rien quand rien ne joue) et coupe les chevauchements.

use std::mem::ManuallyDrop;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use log::{debug, error, info, warn};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;

use crate::mf::{self, AudioFormat, SEC};
use crate::mix::{Mixer, downmix_to_mono};
use crate::ring::{Packet, Ring};

pub const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
/// 16 bits × 2 canaux.
const FRAME_BYTES: usize = 4;
/// Écart toléré entre le ts d'un paquet et la position attendue : la gigue d'horloge
/// est absorbée, au-delà on corrige (reste sous une image à 60 fps).
const TOLERANCE: i64 = SEC / 100;
/// Retard au-delà duquel l'absence de paquets est comblée par du silence : plus que la
/// latence de la loopback, pour ne pas devancer un paquet en route.
const LAG: i64 = SEC / 10;
const POLL: Duration = Duration::from_millis(10);

/// Position de l'audio émis, en échantillons depuis `origin` (ts QPC).
pub struct Timeline {
    origin: i64,
    written: u64,
}

/// Ce qu'il faut faire d'un paquet : silence à insérer avant, échantillons à sauter.
#[derive(Debug, PartialEq)]
pub struct Placement {
    pub silence: u64,
    pub skip: u64,
}

impl Timeline {
    pub fn new(origin: i64) -> Self {
        Self { origin, written: 0 }
    }

    /// ts du prochain échantillon émis.
    pub fn next_ts(&self) -> i64 {
        self.origin + (self.written * SEC as u64 / u64::from(RATE)) as i64
    }

    pub fn place(&mut self, ts: i64, frames: u64) -> Placement {
        let gap = ts - self.next_ts();
        let placement = if gap > TOLERANCE {
            Placement {
                silence: to_frames(gap),
                skip: 0,
            }
        } else if gap < -TOLERANCE {
            Placement {
                silence: 0,
                skip: to_frames(-gap).min(frames),
            }
        } else {
            Placement {
                silence: 0,
                skip: 0,
            }
        };
        self.written += placement.silence + frames - placement.skip;
        placement
    }

    /// Silence à émettre si rien n'est arrivé depuis plus de `LAG`.
    pub fn fill_until(&mut self, now: i64) -> u64 {
        let target = now - LAG;
        let next = self.next_ts();
        if target <= next {
            return 0;
        }
        let frames = to_frames(target - next);
        self.written += frames;
        frames
    }
}

fn to_frames(duration: i64) -> u64 {
    (duration as u64) * u64::from(RATE) / SEC as u64
}

/// Les objets COM de windows-rs ne sont pas `Send` ; l'encodeur n'est utilisé que
/// par le thread audio.
struct SendBox<T>(T);
unsafe impl<T> Send for SendBox<T> {}

/// Démarre capture, mixage et encodage dans un thread dédié ; renvoie le format AAC
/// une fois l'encodeur prêt. Une perte de périphérique est réessayée (silence
/// entre-temps), sans jamais bloquer l'autre source.
pub fn spawn(bitrate: u32, microphone: bool, ring: Arc<Mutex<Ring>>) -> Result<AudioFormat> {
    let (ready_tx, ready_rx) = channel();
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || {
            if let Err(e) = run(bitrate, microphone, &ring, &ready_tx) {
                error!("audio arrêté : {e:#}");
                let _ = ready_tx.send(Err(anyhow!("{e:#}")));
            }
        })?;
    ready_rx
        .recv()
        .context("thread audio mort pendant l'init")?
}

fn run(
    bitrate: u32,
    microphone: bool,
    ring: &Mutex<Ring>,
    ready: &Sender<Result<AudioFormat>>,
) -> Result<()> {
    mf::startup()?;
    let encoder = Encoder::new(bitrate)?;
    let _ = ready.send(Ok(encoder.format()?));
    // Origine commune : l'échantillon k de chaque source tombe au même instant.
    let origin = mf::now();
    let mut inputs = vec![Input::new(Source::System, 0, origin)];
    if microphone {
        inputs.push(Input::new(Source::Microphone, 1, origin));
    }
    let mut mixer = Mixer::new(inputs.len());
    let mut encoded: u64 = 0;
    loop {
        for input in &mut inputs {
            input.poll(&mut mixer);
        }
        let pcm = mixer.pull();
        if !pcm.is_empty() {
            encoder.encode(&pcm, origin + frames_duration(encoded), ring)?;
            encoded += (pcm.len() / CHANNELS as usize) as u64;
        }
        std::thread::sleep(POLL);
    }
}

fn frames_duration(frames: u64) -> i64 {
    (frames * SEC as u64 / u64::from(RATE)) as i64
}

#[derive(Clone, Copy)]
enum Source {
    /// Son du PC : loopback du périphérique de sortie par défaut.
    System,
    /// Micro de communication par défaut (celui de Discord).
    Microphone,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::System => "son du PC",
            Source::Microphone => "micro",
        }
    }

    fn open(self) -> Result<(Capture, String)> {
        let enumerator = wasapi::DeviceEnumerator::new()?;
        let device = match self {
            Source::System => enumerator.get_default_device(&wasapi::Direction::Render)?,
            Source::Microphone => enumerator.get_default_device_for_role(
                &wasapi::Direction::Capture,
                &wasapi::Role::Communications,
            )?,
        };
        let mut client = device.get_iaudioclient()?;
        // autoconvert : le moteur audio livre directement du PCM 16 bits 48 kHz stéréo,
        // quel que soit le format du périphérique (rééchantillonnage, up/downmix).
        let format =
            wasapi::WaveFormat::new(16, 16, &wasapi::SampleType::Int, RATE as usize, 2, None);
        // Direction::Capture sur un périphérique de rendu = mode loopback.
        client.initialize_client(
            &format,
            &wasapi::Direction::Capture,
            &wasapi::StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: 2 * LAG,
            },
        )?;
        let reader = client.get_audiocaptureclient()?;
        client.start_stream()?;
        let name = device.get_friendlyname()?;
        Ok((
            Capture {
                _client: client,
                reader,
            },
            name,
        ))
    }
}

struct Capture {
    /// Gardé en vie : le relâcher arrête le flux.
    _client: wasapi::AudioClient,
    reader: wasapi::AudioCaptureClient,
}

/// Une source et sa position : livre au mixeur un flux continu (silence quand rien
/// n'arrive ou que le périphérique manque) et rouvre le périphérique après une perte.
struct Input {
    source: Source,
    index: usize,
    timeline: Timeline,
    capture: Option<Capture>,
    retry_at: Instant,
    failing: bool,
}

impl Input {
    fn new(source: Source, index: usize, origin: i64) -> Self {
        Self {
            source,
            index,
            timeline: Timeline::new(origin),
            capture: None,
            retry_at: Instant::now(),
            failing: false,
        }
    }

    fn poll(&mut self, mixer: &mut Mixer) {
        if self.capture.is_none() && Instant::now() >= self.retry_at {
            match self.source.open() {
                Ok((capture, device)) => {
                    info!("capture {} : {device}", self.source.name());
                    self.capture = Some(capture);
                    self.failing = false;
                }
                Err(e) => self.fail(&e),
            }
        }
        if let Some(capture) = &self.capture
            && let Err(e) = read_packets(
                &capture.reader,
                &mut self.timeline,
                self.index,
                matches!(self.source, Source::Microphone),
                mixer,
            )
        {
            self.capture = None;
            self.fail(&e);
        }
        let silence = self.timeline.fill_until(mf::now());
        mixer.push_silence(self.index, silence as usize * CHANNELS as usize);
    }

    /// Un seul avertissement par panne, pas un par seconde.
    fn fail(&mut self, e: &anyhow::Error) {
        if !self.failing {
            warn!(
                "{} indisponible : {e:#} ; nouvel essai chaque seconde",
                self.source.name()
            );
        }
        self.failing = true;
        self.retry_at = Instant::now() + Duration::from_secs(1);
    }
}

/// Lit tous les paquets disponibles et les place sur la timeline de la source.
fn read_packets(
    reader: &wasapi::AudioCaptureClient,
    timeline: &mut Timeline,
    index: usize,
    mono: bool,
    mixer: &mut Mixer,
) -> Result<()> {
    let mut pcm = Vec::new();
    loop {
        let frames = reader.get_next_packet_size()?.unwrap_or(0) as usize;
        if frames == 0 {
            return Ok(());
        }
        pcm.resize(frames * FRAME_BYTES, 0);
        let (read, info) = reader.read_from_device(&mut pcm)?;
        let read = read as usize;
        let start_ts = timeline.next_ts();
        let placement = timeline.place(info.timestamp as i64, read as u64);
        debug!(
            "source {index} paquet ts={} retard={:.1} ms frames={read} silent={} {placement:?} next={start_ts}",
            info.timestamp,
            (mf::now() - info.timestamp as i64) as f64 / 1e4,
            info.flags.silent,
        );
        let channels = CHANNELS as usize;
        let skip = placement.skip as usize;
        mixer.push_silence(index, placement.silence as usize * channels);
        if info.flags.silent {
            mixer.push_silence(index, (read - skip) * channels);
        } else {
            let mut samples: Vec<i16> = pcm[skip * FRAME_BYTES..read * FRAME_BYTES]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b))
                .collect();
            if mono {
                downmix_to_mono(&mut samples);
            }
            mixer.push(index, &samples);
        }
    }
}

/// Encodeur AAC de Media Foundation (synchrone).
struct Encoder {
    mft: SendBox<IMFTransform>,
}

impl Encoder {
    fn new(bitrate: u32) -> Result<Self> {
        // SAFETY: API MF documentée ; le tableau d'IMFActivate est libéré par CoTaskMemFree.
        unsafe {
            let input = MFT_REGISTER_TYPE_INFO {
                guidMajorType: MFMediaType_Audio,
                guidSubtype: MFAudioFormat_PCM,
            };
            let output = MFT_REGISTER_TYPE_INFO {
                guidMajorType: MFMediaType_Audio,
                guidSubtype: MFAudioFormat_AAC,
            };
            let mut list = std::ptr::null_mut();
            let mut count = 0;
            MFTEnumEx(
                MFT_CATEGORY_AUDIO_ENCODER,
                MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
                Some(&input),
                Some(&output),
                &mut list,
                &mut count,
            )
            .context("MFTEnumEx AAC")?;
            if count == 0 {
                bail!("aucun encodeur AAC");
            }
            let activates = std::slice::from_raw_parts_mut(list, count as usize);
            let activate = activates[0].take().context("IMFActivate nul")?;
            for a in activates.iter_mut() {
                a.take();
            }
            CoTaskMemFree(Some(list as _));
            let mft: IMFTransform = activate.ActivateObject().context("ActivateObject AAC")?;

            let pcm = MFCreateMediaType()?;
            pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            pcm.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
            pcm.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS)?;
            pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, FRAME_BYTES as u32)?;
            pcm.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, RATE * FRAME_BYTES as u32)?;
            // L'encodeur AAC veut le type d'entrée avant celui de sortie.
            mft.SetInputType(0, &pcm, 0).context("AAC SetInputType")?;

            let aac = MFCreateMediaType()?;
            aac.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            aac.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            aac.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            aac.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
            aac.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS)?;
            // Valeurs acceptées : 12000, 16000, 20000, 24000 octets/s.
            aac.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bitrate / 8)?;
            aac.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?; // AAC brut, pour le MP4
            aac.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)?; // AAC-LC
            mft.SetOutputType(0, &aac, 0).context("AAC SetOutputType")?;

            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(Self { mft: SendBox(mft) })
        }
    }

    /// Le type de sortie réel (avec l'AudioSpecificConfig), sérialisé pour le MP4.
    fn format(&self) -> Result<AudioFormat> {
        // SAFETY: lecture du type courant de l'encodeur.
        let media_type = unsafe { self.mft.0.GetOutputCurrentType(0)? };
        AudioFormat::from_media_type(&media_type)
    }

    /// Encode `pcm` (stéréo entrelacé, premier échantillon à `ts`) et pousse les
    /// trames AAC produites.
    fn encode(&self, pcm: &[i16], ts: i64, ring: &Mutex<Ring>) -> Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let mft = &self.mft.0;
        // SAFETY: MFT synchrone : ProcessInput puis ProcessOutput jusqu'à NEED_MORE_INPUT ;
        // buffers déverrouillés après copie.
        unsafe {
            let bytes = std::mem::size_of_val(pcm);
            let buffer = MFCreateMemoryBuffer(bytes as u32)?;
            let mut ptr = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(pcm.as_ptr().cast::<u8>(), ptr, bytes);
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(ts)?;
            sample.SetSampleDuration(frames_duration((pcm.len() / CHANNELS as usize) as u64))?;
            mft.ProcessInput(0, &sample, 0)
                .context("AAC ProcessInput")?;

            let size = mft.GetOutputStreamInfo(0)?.cbSize.max(8192);
            loop {
                let out = MFCreateSample()?;
                out.AddBuffer(&MFCreateMemoryBuffer(size)?)?;
                let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
                    pSample: ManuallyDrop::new(Some(out.clone())),
                    ..Default::default()
                }];
                let mut status = 0;
                let result = mft.ProcessOutput(0, &mut buffers, &mut status);
                let [b] = &mut buffers;
                ManuallyDrop::drop(&mut b.pSample);
                ManuallyDrop::drop(&mut b.pEvents);
                match result {
                    Ok(()) => {}
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                    Err(e) => return Err(e).context("AAC ProcessOutput"),
                }
                let contiguous = out.ConvertToContiguousBuffer()?;
                let mut ptr = std::ptr::null_mut();
                let mut len = 0;
                contiguous.Lock(&mut ptr, None, Some(&mut len))?;
                let data: Arc<[u8]> = Arc::from(std::slice::from_raw_parts(ptr, len as usize));
                contiguous.Unlock()?;
                let packet = Packet {
                    ts: out.GetSampleTime()?,
                    key: true,
                    data,
                };
                ring.lock()
                    .map_err(|_| anyhow!("ring empoisonné"))?
                    .push_audio(packet);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 ms d'audio à 48 kHz.
    const PACKET: u64 = 480;
    const PACKET_TS: i64 = SEC / 100;

    #[test]
    fn contiguous_packets_need_no_correction() {
        let mut t = Timeline::new(0);
        for k in 0..1000 {
            let p = t.place(k * PACKET_TS, PACKET);
            assert_eq!(
                p,
                Placement {
                    silence: 0,
                    skip: 0
                }
            );
        }
        // 1000 × 10 ms : pas de dérive d'arrondi.
        assert_eq!(t.next_ts(), 10 * SEC);
    }

    #[test]
    fn small_jitter_is_absorbed() {
        let mut t = Timeline::new(0);
        t.place(0, PACKET);
        let p = t.place(PACKET_TS + SEC / 500, PACKET); // +2 ms
        assert_eq!(
            p,
            Placement {
                silence: 0,
                skip: 0
            }
        );
    }

    #[test]
    fn gap_is_filled_with_silence() {
        let mut t = Timeline::new(0);
        t.place(0, PACKET);
        let p = t.place(PACKET_TS + 2 * SEC, PACKET); // 2 s de rien
        assert_eq!(
            p,
            Placement {
                silence: 2 * u64::from(RATE),
                skip: 0
            }
        );
        assert_eq!(t.next_ts(), 2 * SEC + 2 * PACKET_TS);
    }

    #[test]
    fn overlap_is_skipped() {
        let mut t = Timeline::new(0);
        t.place(0, 4 * PACKET); // jusqu'à 40 ms
        let p = t.place(PACKET_TS, 4 * PACKET); // repart à 10 ms : 30 ms en trop
        assert_eq!(
            p,
            Placement {
                silence: 0,
                skip: 3 * PACKET
            }
        );
        assert_eq!(t.next_ts(), 5 * PACKET_TS);
    }

    #[test]
    fn packet_entirely_in_the_past_is_dropped() {
        let mut t = Timeline::new(0);
        t.place(0, 100 * PACKET); // jusqu'à 1 s
        let p = t.place(0, PACKET);
        assert_eq!(
            p,
            Placement {
                silence: 0,
                skip: PACKET
            }
        );
        assert_eq!(t.next_ts(), SEC);
    }

    #[test]
    fn idle_is_filled_only_beyond_lag() {
        let mut t = Timeline::new(0);
        assert_eq!(t.fill_until(LAG), 0);
        assert_eq!(t.fill_until(LAG + SEC), u64::from(RATE));
        assert_eq!(t.next_ts(), SEC);
        assert_eq!(t.fill_until(LAG + SEC), 0);
    }

    #[test]
    fn audio_resuming_after_idle_fill_stays_in_sync() {
        // Silence comblé jusqu'à now - LAG, puis un paquet daté de now - 30 ms arrive.
        let mut t = Timeline::new(0);
        let now = 5 * SEC;
        t.fill_until(now);
        let p = t.place(now - 3 * PACKET_TS, PACKET);
        assert_eq!(p.skip, 0);
        assert_eq!(t.next_ts(), now - 3 * PACKET_TS + PACKET_TS);
    }
}
