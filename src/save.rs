//! Sauvegarde d'un clip : paquets → MP4 (SinkWriter en passthrough) → faststart → rename.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::HSTRING;

use crate::audio;
use crate::mf::{AudioFormat, SEC, VideoFormat};
use crate::ring::{Clip, Packet};

/// Durée d'une trame AAC : 1024 échantillons.
const AAC_FRAME: i64 = 1024 * SEC / audio::RATE as i64;

/// Écrit `clip` dans `dir/clip_AAAAMMJJ_HHMMSS.mp4` et renvoie le chemin.
pub fn save(
    clip: &Clip,
    video: &VideoFormat,
    audio: Option<&AudioFormat>,
    dir: &Path,
) -> Result<PathBuf> {
    let first = clip.video.first().context("clip vide")?;
    let header = sequence_header(&first.data).context("SPS/PPS absents de la keyframe")?;
    std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    let path = dir.join(file_name());
    let tmp = path.with_extension("mp4.tmp");

    write_mp4(clip, &header, video, audio, &tmp)?;
    let bytes = std::fs::read(&tmp)?;
    std::fs::write(&tmp, faststart(bytes)?)?;
    // Écrire puis renommer : un kill en cours de route ne laisse jamais un .mp4 cassé.
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

fn file_name() -> String {
    // SAFETY: GetLocalTime n'a pas de précondition.
    let t = unsafe { GetLocalTime() };
    format!(
        "clip_{:04}{:02}{:02}_{:02}{:02}{:02}.mp4",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

fn write_mp4(
    clip: &Clip,
    header: &[u8],
    video: &VideoFormat,
    audio: Option<&AudioFormat>,
    path: &Path,
) -> Result<()> {
    // Les deux pistes partent de la première image : même horloge QPC, même origine.
    let t0 = clip.video[0].ts;
    let frame = video.frame_duration();
    // SAFETY: API MF documentée ; chaque buffer est déverrouillé après copie.
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("attributs nuls")?;
        // Le conteneur se déduit sinon de l'extension, inconnue pour .tmp.
        // Pas de MF_MPEG4SINK_MOOV_BEFORE_MDAT : sa recopie décale le mdat d'1 Mio.
        attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        let writer = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attrs)
            .context("MFCreateSinkWriterFromURL")?;

        let video_type = video.h264_type()?;
        video_type.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, header)?;
        // Même type en entrée et en sortie : passthrough, aucun réencodage.
        let add = |t: &IMFMediaType| -> Result<u32> {
            let stream = writer.AddStream(t).context("AddStream")?;
            writer
                .SetInputMediaType(stream, t, None)
                .context("SetInputMediaType")?;
            Ok(stream)
        };
        let video_stream = add(&video_type)?;
        let audio_stream = audio.map(|a| add(&a.media_type()?)).transpose()?;
        writer.BeginWriting().context("BeginWriting")?;

        // Écriture dans l'ordre des ts, pistes entrelacées.
        let audio_packets = if audio_stream.is_some() {
            &clip.audio[..]
        } else {
            &[]
        };
        let (mut v, mut a) = (
            clip.video.iter().peekable(),
            audio_packets.iter().peekable(),
        );
        loop {
            let take_video = match (v.peek(), a.peek()) {
                (Some(pv), Some(pa)) => pv.ts <= pa.ts,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let (p, stream, duration) = match (take_video, audio_stream) {
                (true, _) => (v.next(), video_stream, frame),
                (false, Some(s)) => (a.next(), s, AAC_FRAME),
                (false, None) => break,
            };
            let Some(p) = p else { break };
            writer
                .WriteSample(stream, &sample(p, p.ts - t0, duration)?)
                .context("WriteSample")?;
        }
        writer.Finalize().context("Finalize")?;
    }
    Ok(())
}

fn sample(p: &Packet, ts: i64, duration: i64) -> Result<IMFSample> {
    // SAFETY: buffer MF de la bonne taille, déverrouillé après copie.
    unsafe {
        let buffer = MFCreateMemoryBuffer(p.data.len() as u32)?;
        let mut ptr = std::ptr::null_mut();
        buffer.Lock(&mut ptr, None, None)?;
        std::ptr::copy_nonoverlapping(p.data.as_ptr(), ptr, p.data.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(p.data.len() as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(ts)?;
        sample.SetSampleDuration(duration)?;
        sample.SetUINT32(&MFSampleExtension_CleanPoint, u32::from(p.key))?;
        Ok(sample)
    }
}

/// SPS (7) + PPS (8) d'une unité d'accès Annex B, avec start codes : le format de
/// `MF_MT_MPEG_SEQUENCE_HEADER`.
fn sequence_header(au: &[u8]) -> Option<Vec<u8>> {
    let starts: Vec<usize> = (0..au.len().saturating_sub(3))
        .filter(|&i| au[i..i + 3] == [0, 0, 1])
        .collect();
    let mut header = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(au.len(), |&e| {
            // Le zéro de tête d'un start code sur 4 octets appartient au NAL suivant.
            if au[e - 1] == 0 { e - 1 } else { e }
        });
        if matches!(au[s + 3] & 0x1f, 7 | 8) {
            header.extend_from_slice(&[0, 0, 0, 1]);
            header.extend_from_slice(&au[s + 3..end]);
        }
    }
    (!header.is_empty()).then_some(header)
}

/// Algorithme de qt-faststart (ffmpeg) : place `moov` devant `mdat` et décale chaque
/// offset de chunk (`stco`/`co64`) de la taille du `moov`.
fn faststart(file: Vec<u8>) -> Result<Vec<u8>> {
    let boxes = parse_boxes(&file, 0, file.len())?;
    let find = |kind: &[u8; 4]| boxes.iter().position(|b| &b.kind == kind);
    let (Some(moov), Some(mdat)) = (find(b"moov"), find(b"mdat")) else {
        bail!("moov ou mdat absent");
    };
    if moov < mdat {
        return Ok(file);
    }
    let mut moov_bytes = file[boxes[moov].range()].to_vec();
    patch_chunk_offsets(&mut moov_bytes, boxes[moov].len as u64)?;

    let mut out = Vec::with_capacity(file.len());
    for (i, b) in boxes.iter().enumerate() {
        if i == mdat {
            out.extend_from_slice(&moov_bytes);
        }
        if i != moov {
            out.extend_from_slice(&file[b.range()]);
        }
    }
    Ok(out)
}

struct Mp4Box {
    kind: [u8; 4],
    start: usize,
    len: usize,
    header: usize,
}

impl Mp4Box {
    fn range(&self) -> std::ops::Range<usize> {
        self.start..self.start + self.len
    }
}

fn parse_boxes(data: &[u8], from: usize, to: usize) -> Result<Vec<Mp4Box>> {
    let mut boxes = Vec::new();
    let mut i = from;
    while i < to {
        let head = data.get(i..i + 8).context("en-tête de boîte tronqué")?;
        let (len, header) = match u32::from_be_bytes([head[0], head[1], head[2], head[3]]) {
            0 => (to - i, 8),
            1 => {
                let wide = data.get(i + 8..i + 16).context("boîte 64 bits tronquée")?;
                (u64::from_be_bytes(wide.try_into()?) as usize, 16)
            }
            n => (n as usize, 8),
        };
        if len < header || i + len > to {
            bail!("boîte invalide à l'offset {i}");
        }
        let kind = [head[4], head[5], head[6], head[7]];
        boxes.push(Mp4Box {
            kind,
            start: i,
            len,
            header,
        });
        i += len;
    }
    Ok(boxes)
}

/// Descend moov/trak/mdia/minf/stbl et décale les entrées de stco (32 bits) et co64.
fn patch_chunk_offsets(moov: &mut [u8], shift: u64) -> Result<()> {
    let mut stack = vec![(8, moov.len())];
    while let Some((from, to)) = stack.pop() {
        for b in parse_boxes(moov, from, to)? {
            let body = b.start + b.header;
            match &b.kind {
                b"trak" | b"mdia" | b"minf" | b"stbl" => stack.push((body, b.start + b.len)),
                b"stco" | b"co64" => {
                    let wide = &b.kind == b"co64";
                    let size = if wide { 8 } else { 4 };
                    // Corps : version/flags (4), nombre d'entrées (4), puis les offsets.
                    let count = moov.get(body + 4..body + 8).context("table tronquée")?;
                    let count = u32::from_be_bytes(count.try_into()?) as usize;
                    for k in 0..count {
                        let at = body + 8 + k * size;
                        let field = moov.get_mut(at..at + size).context("table tronquée")?;
                        if wide {
                            let v = u64::from_be_bytes((&*field).try_into()?) + shift;
                            field.copy_from_slice(&v.to_be_bytes());
                        } else {
                            let v = u64::from(u32::from_be_bytes((&*field).try_into()?)) + shift;
                            let v = u32::try_from(v).context("offset > 4 Go : co64 requis")?;
                            field.copy_from_slice(&v.to_be_bytes());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    fn table(kind: &[u8; 4], offsets: &[u64]) -> Vec<u8> {
        let mut body = vec![0; 4];
        body.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for &o in offsets {
            if kind == b"co64" {
                body.extend_from_slice(&o.to_be_bytes());
            } else {
                body.extend_from_slice(&(o as u32).to_be_bytes());
            }
        }
        bx(kind, &body)
    }

    fn moov_with(table: Vec<u8>) -> Vec<u8> {
        let stbl = bx(b"stbl", &table);
        let minf = bx(b"minf", &stbl);
        let mdia = bx(b"mdia", &minf);
        let trak = bx(b"trak", &mdia);
        bx(b"moov", &[bx(b"mvhd", &[0; 4]), trak].concat())
    }

    /// ftyp + mdat (deux chunks) + moov dont la table pointe sur les chunks.
    fn mp4(kind: &[u8; 4]) -> (Vec<u8>, usize) {
        let ftyp = bx(b"ftyp", b"isom");
        let mdat = bx(b"mdat", b"AAAABBBB");
        let chunk = (ftyp.len() + 8) as u64;
        let moov = moov_with(table(kind, &[chunk, chunk + 4]));
        let moov_len = moov.len();
        ([ftyp, mdat, moov].concat(), moov_len)
    }

    fn chunk_at(file: &[u8], table_kind: &[u8; 4], k: usize) -> Vec<u8> {
        let at = file.windows(4).position(|w| w == table_kind).unwrap() + 4 + 8;
        let offset = if table_kind == b"co64" {
            u64::from_be_bytes(file[at + 8 * k..at + 8 * k + 8].try_into().unwrap())
        } else {
            u64::from(u32::from_be_bytes(
                file[at + 4 * k..at + 4 * k + 4].try_into().unwrap(),
            ))
        } as usize;
        file[offset..offset + 4].to_vec()
    }

    fn kinds(file: &[u8]) -> Vec<[u8; 4]> {
        parse_boxes(file, 0, file.len())
            .unwrap()
            .iter()
            .map(|b| b.kind)
            .collect()
    }

    #[test]
    fn moves_moov_before_mdat_and_fixes_stco() {
        let (file, moov_len) = mp4(b"stco");
        let out = faststart(file.clone()).unwrap();
        assert_eq!(out.len(), file.len());
        assert_eq!(kinds(&out), [*b"ftyp", *b"moov", *b"mdat"]);
        assert_eq!(chunk_at(&out, b"stco", 0), b"AAAA");
        assert_eq!(chunk_at(&out, b"stco", 1), b"BBBB");
        assert!(moov_len > 0);
    }

    #[test]
    fn fixes_co64() {
        let (file, _) = mp4(b"co64");
        let out = faststart(file).unwrap();
        assert_eq!(chunk_at(&out, b"co64", 0), b"AAAA");
        assert_eq!(chunk_at(&out, b"co64", 1), b"BBBB");
    }

    #[test]
    fn already_faststart_is_unchanged() {
        let (file, _) = mp4(b"stco");
        let once = faststart(file).unwrap();
        assert_eq!(faststart(once.clone()).unwrap(), once);
    }

    #[test]
    fn stco_overflow_is_an_error() {
        let file = [
            bx(b"mdat", b""),
            moov_with(table(b"stco", &[u64::from(u32::MAX)])),
        ]
        .concat();
        assert!(faststart(file).is_err());
    }

    #[test]
    fn truncated_file_is_an_error() {
        let (mut file, _) = mp4(b"stco");
        file.truncate(file.len() - 3);
        assert!(faststart(file).is_err());
    }

    #[test]
    fn missing_moov_is_an_error() {
        assert!(faststart(bx(b"mdat", b"x")).is_err());
    }

    #[test]
    fn extracts_sps_and_pps_with_3_and_4_byte_start_codes() {
        // AUD (3 octets), SPS (4 octets), PPS (3 octets), IDR
        let au = [
            &[0, 0, 1, 0x09, 0xF0][..],
            &[0, 0, 0, 1, 0x67, 0xAA, 0xBB],
            &[0, 0, 1, 0x68, 0xCC],
            &[0, 0, 0, 1, 0x65, 0x88],
        ]
        .concat();
        assert_eq!(
            sequence_header(&au).unwrap(),
            [0, 0, 0, 1, 0x67, 0xAA, 0xBB, 0, 0, 0, 1, 0x68, 0xCC]
        );
    }

    #[test]
    fn no_sps_means_no_header() {
        assert!(sequence_header(&[0, 0, 1, 0x41, 0x9A]).is_none());
    }
}
