//! Vidéo : WGC (écran principal) → VideoProcessor (BGRA → NV12 BT.709, GPU)
//! → horloge CFR → MFT H.264 matériel (async) → `Ring`.

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use log::{error, info, warn};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{E_FAIL, E_POINTER, HMODULE, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
use windows::core::{IInspectable, Interface, Ref};

use crate::mf::{self, SEC, VideoFormat};
use crate::ring::{Packet, Ring};

/// Les objets COM de windows-rs ne sont pas `Send` ; le device est
/// multithread-protected et chaque objet n'est utilisé que sous un Mutex.
struct SendBox<T>(T);
unsafe impl<T> Send for SendBox<T> {}

impl<T> SendBox<T> {
    /// Passer par une méthode fait capturer la boîte entière (donc `Send`) par une
    /// closure, et non son champ seul (capture par champ de l'édition 2024).
    fn get(&self) -> &T {
        &self.0
    }
}

/// Dernière image convertie, partagée entre la capture et l'horloge d'encodage.
type Latest = Arc<Mutex<SendBox<Option<ID3D11Texture2D>>>>;

/// Pipeline vidéo en cours : son format, et de quoi l'arrêter (changement de qualité).
pub struct Video {
    pub format: VideoFormat,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Video {
    /// Arrête capture et encodage, et attend la fin du thread.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Démarre la capture et l'encodage dans un thread dédié. Renvoie une fois
/// l'initialisation réussie. Si l'encodage échoue plus tard, le thread `main_thread`
/// reçoit WM_QUIT.
pub fn spawn(
    height: u32,
    fps: u32,
    bitrate: u32,
    ring: Arc<Mutex<Ring>>,
    main_thread: u32,
) -> Result<Video> {
    let (ready_tx, ready_rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread = std::thread::Builder::new()
        .name("video".into())
        .spawn(move || {
            if let Err(e) = run(height, fps, bitrate, &ring, &thread_stop, &ready_tx) {
                error!("vidéo arrêtée : {e:#}");
                let _ = ready_tx.send(Err(anyhow!("{e:#}")));
                // SAFETY: simple envoi de message au thread principal.
                unsafe {
                    let _ = PostThreadMessageW(main_thread, WM_QUIT, WPARAM(1), LPARAM(0));
                }
            }
        })?;
    let format = ready_rx
        .recv()
        .context("thread vidéo mort pendant l'init")??;
    Ok(Video {
        format,
        stop,
        thread: Some(thread),
    })
}

fn run(
    height: u32,
    fps: u32,
    bitrate: u32,
    ring: &Mutex<Ring>,
    stop: &AtomicBool,
    ready: &Sender<Result<VideoFormat>>,
) -> Result<()> {
    mf::startup()?;
    let (device, context) = create_device()?;
    let item = primary_monitor()?;
    let src = item.Size()?;
    // Hauteur fixe, largeur au ratio de l'écran (3440x1440 → 1720x720), paire pour NV12.
    let width = (src.Width as u32 * height / src.Height as u32) & !1;
    let format = VideoFormat {
        width,
        height,
        fps,
        bitrate,
    };
    let (encoder, name, activate) = create_encoder(&device, &format)?;
    info!(
        "source {}x{} → {width}x{height} @ {fps} fps, {} kb/s, encodeur {name}",
        src.Width,
        src.Height,
        bitrate / 1000
    );

    let latest: Latest = Arc::new(Mutex::new(SendBox(None)));
    let _capture = start_capture(&device, &context, &item, &format, latest.clone())?;
    let _ = ready.send(Ok(format));
    let result = encode_loop(&encoder, &format, &latest, ring, stop);
    shutdown_encoder(&encoder, &activate);
    result
}

/// Un MFT asynchrone garde des références circulaires (file d'événements, device GPU,
/// textures en attente) tant qu'on n'appelle pas `IMFShutdown::Shutdown` : sans ça,
/// chaque changement de qualité fuyait ~240 Mo et ~1 700 handles.
fn shutdown_encoder(mft: &IMFTransform, activate: &IMFActivate) {
    // SAFETY: fin de vie documentée d'un MFT asynchrone ; plus aucun appel ensuite.
    unsafe {
        let _ = mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        if let Ok(shutdown) = mft.cast::<IMFShutdown>() {
            let _ = shutdown.Shutdown();
        }
        let _ = activate.ShutdownObject();
    }
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

/// Renvoie aussi l'`IMFActivate` qui a créé l'encodeur : il faut l'arrêter à la fin.
fn create_encoder(
    device: &ID3D11Device,
    format: &VideoFormat,
) -> Result<(IMFTransform, String, IMFActivate)> {
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

        // Réglages alignés sur OBS (AMF) : CBR, keyframe toutes les 1 s, pas de
        // B-frames (pts = dts), preset qualité, VBV d'1 s pour borner les dépassements.
        // Le mode de débit se règle avant les types.
        let codec: ICodecAPI = mft.cast()?;
        let settings = [
            (
                "RateControlMode",
                CODECAPI_AVEncCommonRateControlMode,
                eAVEncCommonRateControlMode_CBR.0 as u32,
            ),
            ("GOPSize", CODECAPI_AVEncMPVGOPSize, format.fps),
            ("BFrames", CODECAPI_AVEncMPVDefaultBPictureCount, 0),
            ("QualityVsSpeed", CODECAPI_AVEncCommonQualityVsSpeed, 100),
            ("BufferSize", CODECAPI_AVEncCommonBufferSize, format.bitrate),
            ("MaxBitRate", CODECAPI_AVEncCommonMaxBitRate, format.bitrate),
        ];
        for (name, api, value) in settings {
            if let Err(e) = codec.SetValue(&api, &mf::var_u32(value)) {
                warn!("réglage encodeur {name} refusé : {e}");
            }
        }

        mft.SetOutputType(0, &format.h264_type()?, 0)
            .context("SetOutputType")?;
        mft.SetInputType(0, &format.nv12_type()?, 0)
            .context("SetInputType")?;
        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        Ok((mft, friendly, activate))
    }
}

/// Garde la session WGC en vie ; la fermer arrête la capture.
struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    frame_arrived: i64,
}

impl Drop for Capture {
    fn drop(&mut self) {
        // Le gestionnaire retient le convertisseur, le device et la dernière image :
        // le désabonner avant de fermer, sinon tout reste en mémoire.
        let _ = self.pool.RemoveFrameArrived(self.frame_arrived);
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

/// WGC n'envoie une image que quand l'écran change : chaque image reçue est convertie
/// puis déposée dans `latest`, que l'horloge d'encodage reprend à fréquence fixe.
fn start_capture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    item: &GraphicsCaptureItem,
    format: &VideoFormat,
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
            warn!("bordure jaune de capture non désactivable : {e}");
        }

        let converter = Converter::new(device, context, size, format)?;
        let state = Mutex::new(SendBox((converter, size)));
        let (device, context, format) =
            (SendBox(device.clone()), SendBox(context.clone()), *format);
        let winrt_device = SendBox(winrt_device);
        let frame_arrived = pool.FrameArrived(&TypedEventHandler::new(
            move |pool: Ref<Direct3D11CaptureFramePool>, _: Ref<IInspectable>| {
                let pool = pool.ok()?;
                let frame = pool.TryGetNextFrame()?;
                let mut state = state
                    .lock()
                    .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
                let SendBox((converter, size)) = &mut *state;
                // Changement de résolution (jeu en plein écran exclusif, réglage
                // Windows) : le pool garde sa taille de départ, on le recrée.
                let content = frame.ContentSize()?;
                if content != *size {
                    frame.Close()?;
                    info!("résolution source {}x{}", content.Width, content.Height);
                    pool.Recreate(
                        winrt_device.get(),
                        DirectXPixelFormat::B8G8R8A8UIntNormalized,
                        2,
                        content,
                    )?;
                    *converter = Converter::new(device.get(), context.get(), content, &format)
                        .map_err(|e| windows::core::Error::new(E_FAIL, format!("{e:#}")))?;
                    *size = content;
                    return Ok(());
                }
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                let texture: ID3D11Texture2D = access.GetInterface()?;
                let nv12 = converter.convert(&texture)?;
                frame.Close()?;
                latest
                    .lock()
                    .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?
                    .0 = Some(nv12);
                Ok(())
            },
        ))?;
        session.StartCapture()?;
        Ok(Capture {
            pool,
            session,
            frame_arrived,
        })
    }
}

/// BGRA (taille écran) → NV12 BT.709 plage limitée, à la taille de sortie, sur le GPU.
struct Converter {
    device: ID3D11Device,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    width: u32,
    height: u32,
}

impl Converter {
    fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        src: SizeInt32,
        format: &VideoFormat,
    ) -> Result<Self> {
        let (src_w, src_h) = (src.Width as u32, src.Height as u32);
        // SAFETY: création et réglage d'un VideoProcessor D3D11 documentés.
        unsafe {
            let video_device: ID3D11VideoDevice = device.cast()?;
            let video_context: ID3D11VideoContext = context.cast()?;
            let rate = DXGI_RATIONAL {
                Numerator: format.fps,
                Denominator: 1,
            };
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: src_w,
                InputHeight: src_h,
                OutputFrameRate: rate,
                OutputWidth: format.width,
                OutputHeight: format.height,
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
            // Proportions conservées : bandes noires plutôt qu'une image étirée.
            let (x, y, w, h) = fit(src_w, src_h, format.width, format.height);
            let dest = RECT {
                left: x as i32,
                top: y as i32,
                right: (x + w) as i32,
                bottom: (y + h) as i32,
            };
            video_context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dest));
            let black = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    YCbCr: D3D11_VIDEO_COLOR_YCbCrA {
                        Y: 16.0 / 255.0,
                        Cb: 0.5,
                        Cr: 0.5,
                        A: 1.0,
                    },
                },
            };
            video_context.VideoProcessorSetOutputBackgroundColor(&processor, true, &black);
            Ok(Self {
                device: device.clone(),
                video_device,
                video_context,
                enumerator,
                processor,
                width: format.width,
                height: format.height,
            })
        }
    }

    unsafe fn convert(&self, src: &ID3D11Texture2D) -> windows::core::Result<ID3D11Texture2D> {
        unsafe {
            // Une texture neuve par image : l'encodeur async peut encore lire la précédente.
            let desc = D3D11_TEXTURE2D_DESC {
                Width: self.width,
                Height: self.height,
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

/// Horloge à fréquence fixe (comme OBS) : l'image n part à start + n/fps avec
/// ts = t0 + n/fps ; si WGC n'a rien envoyé de neuf, la dernière image est répétée.
fn encode_loop(
    mft: &IMFTransform,
    format: &VideoFormat,
    latest: &Latest,
    ring: &Mutex<Ring>,
    stop: &AtomicBool,
) -> Result<()> {
    let fps = i64::from(format.fps);
    let start = Instant::now();
    let t0 = mf::now();
    let mut n: i64 = 0;
    // SAFETY: modèle async MF : on ne nourrit/vide l'encodeur qu'en réponse à ses événements.
    unsafe {
        let events: IMFMediaEventGenerator = mft.cast()?;
        loop {
            let event = events.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0))?;
            let kind = event.GetType()?;
            if kind == METransformNeedInput.0 as u32 {
                // Vérifié à chaque image (60 fois par seconde) : arrêt quasi immédiat.
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let due = start + Duration::from_nanos((n * 1_000_000_000 / fps) as u64);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                let texture = wait_first_frame(latest)?;
                let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &texture, 0, false)?;
                let sample = MFCreateSample()?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(t0 + n * SEC / fps)?;
                sample.SetSampleDuration(format.frame_duration())?;
                mft.ProcessInput(0, &sample, 0).context("ProcessInput")?;
                n += 1;
            } else if kind == METransformHaveOutput.0 as u32 {
                let Some(sample) = process_output(mft)? else {
                    continue;
                };
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut ptr = std::ptr::null_mut();
                let mut len = 0;
                buffer.Lock(&mut ptr, None, Some(&mut len))?;
                let data: Arc<[u8]> = Arc::from(std::slice::from_raw_parts(ptr, len as usize));
                buffer.Unlock()?;
                let packet = Packet {
                    ts: sample.GetSampleTime()?,
                    key: has_idr(&data),
                    data,
                };
                ring.lock()
                    .map_err(|_| anyhow!("ring empoisonné"))?
                    .push_video(packet);
            }
        }
    }
}

fn wait_first_frame(latest: &Latest) -> Result<ID3D11Texture2D> {
    loop {
        let guard = latest.lock().map_err(|_| anyhow!("mutex empoisonné"))?;
        if let Some(t) = guard.0.clone() {
            return Ok(t);
        }
        drop(guard);
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
                warn!("type de sortie de l'encodeur renégocié");
                return Ok(None);
            }
            Err(e) => return Err(e).context("ProcessOutput"),
        }
        let [buffer] = &mut buffers;
        ManuallyDrop::drop(&mut buffer.pEvents);
        Ok(ManuallyDrop::take(&mut buffer.pSample))
    }
}

/// Cherche une NAL IDR (type 5) dans une unité d'accès Annex B.
fn has_idr(data: &[u8]) -> bool {
    data.windows(4)
        .any(|w| w[..3] == [0, 0, 1] && w[3] & 0x1f == 5)
}

/// Rectangle (x, y, largeur, hauteur) de `src` mis à l'échelle dans `dst` sans
/// déformation, centré, dimensions paires (NV12).
fn fit(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    let (w, h) = if u64::from(src_w) * u64::from(dst_h) > u64::from(dst_w) * u64::from(src_h) {
        (
            dst_w,
            (u64::from(dst_w) * u64::from(src_h) / u64::from(src_w)) as u32 & !1,
        )
    } else {
        (
            (u64::from(dst_h) * u64::from(src_w) / u64::from(src_h)) as u32 & !1,
            dst_h,
        )
    };
    (((dst_w - w) / 2) & !1, ((dst_h - h) / 2) & !1, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_aspect_fills_output() {
        assert_eq!(fit(3440, 1440, 1720, 720), (0, 0, 1720, 720));
    }

    #[test]
    fn narrower_source_is_pillarboxed() {
        // 16:9 dans 21:9 : 1280x720 centré, bandes de 220 px.
        assert_eq!(fit(1920, 1080, 1720, 720), (220, 0, 1280, 720));
    }

    #[test]
    fn wider_source_is_letterboxed() {
        // 32:9 dans 21:9 : pleine largeur, hauteur 482 (paire), centré.
        assert_eq!(fit(5120, 1440, 1720, 720), (0, 118, 1720, 482));
    }
}
