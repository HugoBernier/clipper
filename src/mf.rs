//! Helpers Media Foundation partagés par `video` et `save`, et l'horloge QPC.

use std::mem::ManuallyDrop;

use anyhow::{Context, Result};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_UI4};

pub const SEC: i64 = 10_000_000; // 100 ns

/// COM (MTA) + Media Foundation, à appeler dans chaque thread qui utilise MF.
pub fn startup() -> Result<()> {
    // SAFETY: COM MTA est réentrant ; MFStartup est compté par référence.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .context("CoInitializeEx")?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup")
    }
}

/// Horloge commune à tous les timestamps : QPC en unités de 100 ns.
pub fn now() -> i64 {
    let (mut counter, mut freq) = (0, 0);
    // SAFETY: ne peuvent pas échouer depuis Windows XP.
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    (i128::from(counter) * i128::from(SEC) / i128::from(freq)) as i64
}

/// Format vidéo de sortie, commun à l'encodeur et au MP4.
#[derive(Clone, Copy, Debug)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
}

impl VideoFormat {
    pub fn frame_duration(&self) -> i64 {
        SEC / i64::from(self.fps)
    }

    pub fn h264_type(&self) -> Result<IMFMediaType> {
        let t = self.video_type(&MFVideoFormat_H264)?;
        // SAFETY: attributs MF standards sur un type fraîchement créé.
        unsafe {
            t.SetUINT32(&MF_MT_AVG_BITRATE, self.bitrate)?;
            t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1 << 32 | 1)?;
            t.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
        }
        Ok(t)
    }

    pub fn nv12_type(&self) -> Result<IMFMediaType> {
        self.video_type(&MFVideoFormat_NV12)
    }

    fn video_type(&self, subtype: &windows::core::GUID) -> Result<IMFMediaType> {
        // SAFETY: attributs MF standards sur un type fraîchement créé.
        unsafe {
            let t = MFCreateMediaType()?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
            t.SetUINT64(
                &MF_MT_FRAME_SIZE,
                u64::from(self.width) << 32 | u64::from(self.height),
            )?;
            t.SetUINT64(&MF_MT_FRAME_RATE, u64::from(self.fps) << 32 | 1)?;
            t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            // BT.709 plage limitée : le standard HD, et ce que les lecteurs supposent.
            t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
            t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
            t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
            t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
            Ok(t)
        }
    }
}

/// VARIANT VT_UI4 pour `ICodecAPI::SetValue` (le VARIANT Win32 n'a pas de `From`).
pub fn var_u32(v: u32) -> VARIANT {
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
