//! Mesure de synchro A/V d'un clip : `cargo run --example sync_check -- <clip.mp4>`.
//!
//! Décode le clip avec Media Foundation, repère les fronts montants de luminosité
//! (flash à l'écran) et de niveau sonore (bip), puis apparie chaque flash au bip le
//! plus proche et affiche l'écart. À utiliser avec un stimulus flash + bip simultanés.

use anyhow::{Context, Result};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

const SEC: f64 = 10_000_000.0;
/// Écart minimal entre deux événements, pour ne compter qu'un front par flash/bip.
const HOLD: f64 = 0.3;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage : sync_check <clip.mp4>")?;
    // SAFETY: init COM/MF une fois.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
    }
    let flashes = video_edges(&path)?;
    let beeps = audio_edges(&path)?;
    println!("flashs : {}", fmt(&flashes));
    println!("bips   : {}", fmt(&beeps));
    for f in &flashes {
        if let Some(b) = beeps
            .iter()
            .min_by(|a, b| (*a - f).abs().total_cmp(&(*b - f).abs()))
        {
            println!(
                "flash {f:6.3} s ↔ bip {b:6.3} s : son {:+5.1} ms",
                (b - f) * 1000.0
            );
        }
    }
    Ok(())
}

fn fmt(times: &[f64]) -> String {
    times
        .iter()
        .map(|t| format!("{t:.3}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn reader(path: &str, stream: u32, subtype: &windows::core::GUID) -> Result<IMFSourceReader> {
    // SAFETY: API MF documentée.
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("attributs")?;
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path), &attrs)?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(stream, true)?;
        let t = MFCreateMediaType()?;
        let major = if stream == MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32 {
            MFMediaType_Video
        } else {
            MFMediaType_Audio
        };
        t.SetGUID(&MF_MT_MAJOR_TYPE, &major)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        if major == MFMediaType_Audio {
            t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        }
        reader.SetCurrentMediaType(stream, None, &t)?;
        Ok(reader)
    }
}

/// Appelle `f(ts, octets)` pour chaque échantillon décodé.
fn each_sample(reader: &IMFSourceReader, stream: u32, mut f: impl FnMut(f64, &[u8])) -> Result<()> {
    // SAFETY: buffers verrouillés le temps de la lecture.
    unsafe {
        loop {
            let (mut flags, mut ts, mut sample) = (0, 0, None);
            reader.ReadSample(
                stream,
                0,
                None,
                Some(&mut flags),
                Some(&mut ts),
                Some(&mut sample),
            )?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(());
            }
            let Some(sample) = sample else { continue };
            let buffer = sample.ConvertToContiguousBuffer()?;
            let (mut ptr, mut len) = (std::ptr::null_mut(), 0);
            buffer.Lock(&mut ptr, None, Some(&mut len))?;
            f(
                ts as f64 / SEC,
                std::slice::from_raw_parts(ptr, len as usize),
            );
            buffer.Unlock()?;
        }
    }
}

/// Fronts montants de luminosité au centre de l'image (là où le stimulus flashe).
fn video_edges(path: &str) -> Result<Vec<f64>> {
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let r = reader(path, stream, &MFVideoFormat_RGB32)?;
    // SAFETY: lecture du type courant.
    let size = unsafe {
        r.GetCurrentMediaType(stream)?
            .GetUINT64(&MF_MT_FRAME_SIZE)?
    };
    let (w, h) = ((size >> 32) as usize, (size & 0xffff_ffff) as usize);
    let mut levels = Vec::new();
    each_sample(&r, stream, |ts, px| {
        // Lignes alignées : pas réel = taille / hauteur. Carré central de h/4 de côté.
        let stride = px.len() / h;
        let half = h / 8;
        let (mut sum, mut n) = (0u64, 0u64);
        for y in h / 2 - half..h / 2 + half {
            for x in w / 2 - half..w / 2 + half {
                let p = &px[y * stride + x * 4..][..3];
                sum += u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]);
                n += 3;
            }
        }
        levels.push((ts, sum as f64 / n as f64));
        if let Some(at) = std::env::var("SYNC_DUMP")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            && (ts - at).abs() < 0.009
        {
            let rows: Vec<u8> = px
                .chunks(stride)
                .take(h)
                .flat_map(|r| &r[..w * 4])
                .copied()
                .collect();
            let _ = dump_bmp("target/sync_frame.bmp", w, h, &rows);
        }
    })?;
    Ok(edges(&levels))
}

/// Fronts montants de niveau sonore (crête par fenêtre de 1 ms, canal gauche).
fn audio_edges(path: &str) -> Result<Vec<f64>> {
    let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    let r = reader(path, stream, &MFAudioFormat_PCM)?;
    let mut levels = Vec::new();
    each_sample(&r, stream, |ts, pcm| {
        for (k, window) in pcm.chunks(48 * 4).enumerate() {
            let peak = window
                .as_chunks::<4>()
                .0
                .iter()
                .map(|f| i16::from_le_bytes([f[0], f[1]]).unsigned_abs())
                .max()
                .unwrap_or(0);
            levels.push((ts + k as f64 / 1000.0, f64::from(peak)));
        }
    })?;
    // SYNC_PROFILE=<s> : niveau crête par ms sur 150 ms à partir de cet instant.
    if let Some(at) = std::env::var("SYNC_PROFILE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        let profile: Vec<String> = levels
            .iter()
            .filter(|l| l.0 >= at && l.0 < at + 0.15)
            .map(|l| format!("{:.0}", l.1))
            .collect();
        println!("profil audio : {}", profile.join(" "));
    }
    Ok(edges(&levels))
}

/// Instants où le niveau franchit le milieu entre min et max, en montant ; affiche
/// aussi la durée de chaque événement (jusqu'au retour sous le milieu).
fn edges(levels: &[(f64, f64)]) -> Vec<f64> {
    let lo = levels.iter().map(|l| l.1).fold(f64::MAX, f64::min);
    let hi = levels.iter().map(|l| l.1).fold(f64::MIN, f64::max);
    let mid = (lo + hi) / 2.0;
    let mut out: Vec<f64> = Vec::new();
    let mut lengths = Vec::new();
    for (k, w) in levels.windows(2).enumerate() {
        if w[0].1 < mid && w[1].1 >= mid && out.last().is_none_or(|t| w[1].0 - t > HOLD) {
            out.push(w[1].0);
            let end = levels[k + 1..]
                .iter()
                .find(|l| l.1 < mid)
                .map_or(w[1].0, |l| l.0);
            lengths.push(format!("{:.0}", (end - w[1].0) * 1000.0));
        }
    }
    println!("durées (ms) : {}", lengths.join(" "));
    out
}

fn dump_bmp(path: &str, w: usize, h: usize, bgra: &[u8]) -> std::io::Result<()> {
    let mut f = b"BM".to_vec();
    f.extend_from_slice(&(54 + bgra.len() as u32).to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&(w as i32).to_le_bytes());
    f.extend_from_slice(&(-(h as i32)).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&[0; 24]);
    f.extend_from_slice(bgra);
    std::fs::write(path, f)
}
