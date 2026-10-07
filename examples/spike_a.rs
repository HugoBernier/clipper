//! Spike A : WGC (écran principal) → VideoProcessor (BGRA → NV12 BT.709, GPU)
//! → MFT H.264 matériel (async, CFR 60 fps) → target/spike_a.h264 (10 s).
//!
//! Jetable : sert à lever le risque « encodeur matériel AMD depuis Rust ».

use std::fs::File;
use std::io::Write;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{E_FAIL, E_POINTER, FILETIME, HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_UI4};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface, Ref};

const H: u32 = 720;
const FPS: u32 = 60;
const BITRATE: u32 = 4_500_000;
const SECONDS: u32 = 10;
const SEC: i64 = 10_000_000; // 100 ns
const OUT: &str = "target/spike_a.h264";
/// Une ligne par paquet : `ts len keyframe`, lue par le spike B.
const INDEX: &str = "target/spike_a.idx";

/// Les objets COM de windows-rs ne sont pas `Send` ; ici le device est
/// multithread-protected et chaque objet n'est utilisé que sous un Mutex.
struct SendBox<T>(T);
unsafe impl<T> Send for SendBox<T> {}

/// Dernière image convertie, partagée entre la capture et l'horloge d'encodage.
type Latest = Arc<Mutex<SendBox<Option<ID3D11Texture2D>>>>;

static CAPTURED: AtomicU32 = AtomicU32::new(0);

fn main() -> Result<()> {
    // SAFETY: init COM/MF une fois, sur le thread principal.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup")?;
    }
    let (device, context) = create_device()?;
    let item = primary_monitor()?;
    let src = item.Size()?;
    // Hauteur fixe, largeur au ratio de l'écran (3440x1440 → 1720x720), paire pour NV12.
    let w = (src.Width as u32 * H / src.Height as u32) & !1;
    println!("source : {}x{} → sortie {w}x{H}", src.Width, src.Height);
    let (encoder, name) = create_encoder(&device, w)?;
    println!("encodeur : {name}");

    let latest: Latest = Arc::new(Mutex::new(SendBox(None)));
    let capture = start_capture(&device, &context, &item, w, latest.clone())?;

    let wall = Instant::now();
    let cpu0 = process_cpu_100ns();
    let stats = encode_loop(&encoder, &latest)?;
    let cpu = (process_cpu_100ns() - cpu0) as f64 / SEC as f64;
    let elapsed = wall.elapsed().as_secs_f64();
    drop(capture);

    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let kbps = stats.bytes as f64 * 8.0 / SECONDS as f64 / 1000.0;
    println!(
        "images WGC reçues : {} ; images encodées : {} (dont {} répétées)",
        CAPTURED.load(Ordering::Relaxed),
        stats.frames_in,
        stats.repeated
    );
    println!("paquets encodés   : {}", stats.packets_out);
    println!(
        "taille            : {} Ko → {kbps:.0} kb/s (cible {})",
        stats.bytes / 1024,
        BITRATE / 1000
    );
    println!("keyframes (s)     : {:?}", stats.keyframes);
    println!(
        "CPU process       : {:.1} % de la machine ({cpu:.2} s CPU / {elapsed:.1} s)",
        cpu / elapsed / cores * 100.0
    );
    println!("sortie            : {OUT}");
    Ok(())
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    // SAFETY: appels D3D11 standards, pointeurs de sortie valides.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            // VIDEO_SUPPORT : requis par le VideoProcessor et l'encodeur MF.
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .context("D3D11CreateDevice")?;
        let device: ID3D11Device = device.context("device nul")?;
        // L'encodeur MF utilise le device depuis ses propres threads.
        let _previous = device
            .cast::<ID3D11Multithread>()?
            .SetMultithreadProtected(true);
        Ok((device, context.context("context nul")?))
    }
}

/// BT.709, plage limitée (16-235) : le standard HD, et ce que les lecteurs supposent.
unsafe fn tag_bt709(t: &IMFMediaType) -> windows::core::Result<()> {
    unsafe {
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
    }
}

fn create_encoder(device: &ID3D11Device, w: u32) -> Result<(IMFTransform, String)> {
    // SAFETY: API MF documentée ; le tableau d'IMFActivate est libéré par CoTaskMemFree.
    unsafe {
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut list = std::ptr::null_mut();
        let mut count = 0;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut list,
            &mut count,
        )
        .context("MFTEnumEx")?;
        if count == 0 {
            bail!("aucun encodeur H.264 matériel");
        }
        let activates = std::slice::from_raw_parts_mut(list, count as usize);
        let activate = activates[0].take().context("IMFActivate nul")?;
        for a in activates.iter_mut() {
            a.take();
        }
        CoTaskMemFree(Some(list as _));

        let mut name = windows::core::PWSTR::null();
        let mut len = 0;
        activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut name, &mut len)?;
        let friendly = name.to_string()?;
        CoTaskMemFree(Some(name.0 as _));

        let mft: IMFTransform = activate.ActivateObject().context("ActivateObject")?;
        mft.GetAttributes()?
            .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;

        let mut token = 0;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.context("device manager nul")?;
        manager.ResetDevice(device, token)?;
        mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
            .context("SET_D3D_MANAGER")?;

        // Réglages alignés sur OBS (AMF) : CBR pour une taille prévisible, keyframe
        // toutes les 1 s, pas de B-frames (défaut OBS ; pts = dts, buffer plus simple),
        // preset qualité. Le mode de débit se règle avant les types.
        let codec: ICodecAPI = mft.cast()?;
        let settings = [
            (
                "RateControlMode",
                CODECAPI_AVEncCommonRateControlMode,
                eAVEncCommonRateControlMode_CBR.0 as u32,
            ),
            ("GOPSize", CODECAPI_AVEncMPVGOPSize, FPS),
            ("BFrames", CODECAPI_AVEncMPVDefaultBPictureCount, 0),
            ("QualityVsSpeed", CODECAPI_AVEncCommonQualityVsSpeed, 100),
            // Tampon VBV/HRD d'1 s : borne les dépassements du CBR (taille Discord).
            ("BufferSize", CODECAPI_AVEncCommonBufferSize, BITRATE),
            ("MaxBitRate", CODECAPI_AVEncCommonMaxBitRate, BITRATE),
        ];
        for (name, api, value) in settings {
            match codec.SetValue(&api, &var_u32(value)) {
                Ok(()) => println!("réglage {name} = {value}"),
                Err(e) => println!("réglage {name} refusé : {e}"),
            }
        }

        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, BITRATE)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, (w as u64) << 32 | H as u64)?;
        out.SetUINT64(&MF_MT_FRAME_RATE, (FPS as u64) << 32 | 1)?;
        out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1 << 32 | 1)?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
        tag_bt709(&out)?;
        mft.SetOutputType(0, &out, 0).context("SetOutputType")?;

        let inp = MFCreateMediaType()?;
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        inp.SetUINT64(&MF_MT_FRAME_SIZE, (w as u64) << 32 | H as u64)?;
        inp.SetUINT64(&MF_MT_FRAME_RATE, (FPS as u64) << 32 | 1)?;
        inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        tag_bt709(&inp)?;
        mft.SetInputType(0, &inp, 0).context("SetInputType")?;

        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        Ok((mft, friendly))
    }
}

/// Garde la session WGC en vie ; la fermer arrête la capture.
struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn primary_monitor() -> Result<GraphicsCaptureItem> {
    // SAFETY: interop WinRT documentée.
    unsafe {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        interop
            .CreateForMonitor(monitor)
            .context("CreateForMonitor")
    }
}

/// WGC n'envoie une image que quand l'écran change : on convertit chaque image
/// reçue et on la dépose dans `latest`. L'horloge d'encodage la reprend à 60 fps.
fn start_capture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    item: &GraphicsCaptureItem,
    w: u32,
    latest: Latest,
) -> Result<Capture> {
    // SAFETY: interop WinRT/D3D11 documentée.
    unsafe {
        let size = item.Size()?;
        let winrt_device: IDirect3DDevice =
            CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)?.cast()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(item)?;
        if let Err(e) = session.SetIsBorderRequired(false) {
            println!("bordure jaune non désactivable : {e}");
        }

        let converter = SendBox(Converter::new(
            device,
            context,
            size.Width as u32,
            size.Height as u32,
            w,
        )?);
        pool.FrameArrived(&TypedEventHandler::new(
            move |pool: Ref<Direct3D11CaptureFramePool>, _: Ref<IInspectable>| {
                let frame = pool.ok()?.TryGetNextFrame()?;
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                let texture: ID3D11Texture2D = access.GetInterface()?;
                let nv12 = converter.0.convert(&texture)?;
                frame.Close()?;
                latest
                    .lock()
                    .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?
                    .0 = Some(nv12);
                CAPTURED.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        ))?;
        session.StartCapture()?;
        Ok(Capture { pool, session })
    }
}

/// BGRA (taille écran) → NV12 BT.709 plage limitée, à la taille de sortie, sur le GPU.
struct Converter {
    device: ID3D11Device,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    w: u32,
}

impl Converter {
    unsafe fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        src_w: u32,
        src_h: u32,
        w: u32,
    ) -> Result<Self> {
        unsafe {
            let video_device: ID3D11VideoDevice = device.cast()?;
            let video_context: ID3D11VideoContext = context.cast()?;
            let rate = DXGI_RATIONAL {
                Numerator: FPS,
                Denominator: 1,
            };
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: src_w,
                InputHeight: src_h,
                OutputFrameRate: rate,
                OutputWidth: w,
                OutputHeight: H,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumerator = video_device.CreateVideoProcessorEnumerator(&desc)?;
            let processor = video_device.CreateVideoProcessor(&enumerator, 0)?;
            // Sans ça, le VideoProcessor sort du BT.601 (valeur par défaut).
            let ctx1: ID3D11VideoContext1 = video_context.cast()?;
            ctx1.VideoProcessorSetStreamColorSpace1(
                &processor,
                0,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
            ctx1.VideoProcessorSetOutputColorSpace1(
                &processor,
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
            );
            Ok(Self {
                device: device.clone(),
                video_device,
                video_context,
                enumerator,
                processor,
                w,
            })
        }
    }

    unsafe fn convert(&self, src: &ID3D11Texture2D) -> windows::core::Result<ID3D11Texture2D> {
        unsafe {
            // Une texture neuve par image : l'encodeur async peut encore lire la précédente.
            let desc = D3D11_TEXTURE2D_DESC {
                Width: self.w,
                Height: H,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };
            let mut nv12 = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut nv12))?;
            let nv12 = nv12.ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))?;

            let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut out_view = None;
            self.video_device.CreateVideoProcessorOutputView(
                &nv12,
                &self.enumerator,
                &out_desc,
                Some(&mut out_view),
            )?;

            let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
            };
            let mut in_view = None;
            self.video_device.CreateVideoProcessorInputView(
                src,
                &self.enumerator,
                &in_desc,
                Some(&mut in_view),
            )?;

            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(in_view),
                ..Default::default()
            };
            let result = self.video_context.VideoProcessorBlt(
                &self.processor,
                out_view
                    .as_ref()
                    .ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))?,
                0,
                std::slice::from_ref(&stream),
            );
            ManuallyDrop::drop(&mut stream.pInputSurface);
            result?;
            Ok(nv12)
        }
    }
}

#[derive(Default)]
struct Stats {
    frames_in: u32,
    repeated: u32,
    packets_out: u32,
    bytes: usize,
    keyframes: Vec<f64>,
}

/// Horloge à fréquence fixe (comme OBS) : l'image n part à start + n/FPS avec
/// ts = n/FPS ; si WGC n'a rien envoyé de neuf, la dernière image est répétée.
fn encode_loop(mft: &IMFTransform, latest: &Latest) -> Result<Stats> {
    let mut file = File::create(OUT)?;
    let mut index = File::create(INDEX)?;
    let mut stats = Stats::default();
    let mut draining = false;
    let mut n: u32 = 0;
    let mut previous: Option<ID3D11Texture2D> = None;
    let start = Instant::now();
    // SAFETY: modèle async MF : on ne nourrit/vide l'encodeur qu'en réponse à ses événements.
    unsafe {
        let events: IMFMediaEventGenerator = mft.cast()?;
        loop {
            let event = events.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0))?;
            let kind = event.GetType()?;
            if kind == METransformNeedInput.0 as u32 {
                if draining {
                    continue;
                }
                if n == SECONDS * FPS {
                    mft.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)?;
                    draining = true;
                    continue;
                }
                let due = start + Duration::from_nanos(n as u64 * 1_000_000_000 / FPS as u64);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                let texture = wait_first_frame(latest)?;
                if previous.as_ref() == Some(&texture) {
                    stats.repeated += 1;
                }
                let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &texture, 0, false)?;
                let sample = MFCreateSample()?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(n as i64 * SEC / FPS as i64)?;
                sample.SetSampleDuration(SEC / FPS as i64)?;
                mft.ProcessInput(0, &sample, 0).context("ProcessInput")?;
                previous = Some(texture);
                stats.frames_in += 1;
                n += 1;
            } else if kind == METransformHaveOutput.0 as u32 {
                let Some(sample) = process_output(mft)? else {
                    continue;
                };
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut ptr = std::ptr::null_mut();
                let mut len = 0;
                buffer.Lock(&mut ptr, None, Some(&mut len))?;
                let data = std::slice::from_raw_parts(ptr, len as usize);
                let ts = sample.GetSampleTime()?;
                let key = has_idr(data);
                if key {
                    stats
                        .keyframes
                        .push((ts as f64 / SEC as f64 * 100.0).round() / 100.0);
                }
                writeln!(index, "{ts} {} {}", data.len(), key as u8)?;
                file.write_all(data)?;
                stats.bytes += data.len();
                buffer.Unlock()?;
                stats.packets_out += 1;
            } else if kind == METransformDrainComplete.0 as u32 {
                break;
            }
        }
    }
    Ok(stats)
}

fn wait_first_frame(latest: &Latest) -> Result<ID3D11Texture2D> {
    loop {
        if let Some(t) = latest
            .lock()
            .map_err(|_| anyhow::anyhow!("mutex empoisonné"))?
            .0
            .clone()
        {
            return Ok(t);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

unsafe fn process_output(mft: &IMFTransform) -> Result<Option<IMFSample>> {
    unsafe {
        let mut buffers = [MFT_OUTPUT_DATA_BUFFER::default()];
        let mut status = 0;
        match mft.ProcessOutput(0, &mut buffers, &mut status) {
            Ok(()) => {}
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                let t = mft.GetOutputAvailableType(0, 0)?;
                mft.SetOutputType(0, &t, 0)?;
                println!("changement de type de sortie renégocié");
                return Ok(None);
            }
            Err(e) => return Err(e).context("ProcessOutput"),
        }
        let [buffer] = &mut buffers;
        ManuallyDrop::drop(&mut buffer.pEvents);
        Ok(ManuallyDrop::take(&mut buffer.pSample))
    }
}

fn var_u32(v: u32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_UI4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { ulVal: v },
            }),
        },
    }
}

/// Cherche une NAL IDR (type 5) dans un flux Annex B.
fn has_idr(data: &[u8]) -> bool {
    data.windows(4)
        .any(|w| w[..3] == [0, 0, 1] && w[3] & 0x1f == 5)
}

fn process_cpu_100ns() -> i64 {
    let (mut c, mut e, mut k, mut u) = Default::default();
    // SAFETY: GetProcessTimes sur le process courant, pointeurs valides.
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
    }
    let ft = |f: FILETIME| ((f.dwHighDateTime as i64) << 32) | f.dwLowDateTime as i64;
    ft(k) + ft(u)
}
