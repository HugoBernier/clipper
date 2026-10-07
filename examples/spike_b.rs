//! Spike B : paquets H.264 du spike A → MP4 via IMFSinkWriter en passthrough.
//!
//! Simule une sauvegarde : coupe à la première keyframe ≥ 2 s, ramène ses ts à 0,
//! écrit `target/spike_b.mp4` (moov avant mdat), puis vérifie le fichier :
//! ordre des boîtes, décodage complet par MF, une image exportée en BMP.
//!
//! Prérequis : `cargo run --example spike_a` (produit .h264 + .idx). Jetable.

use std::fs;

use anyhow::{Context, Result, bail};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

const FPS: u32 = 60;
const SEC: i64 = 10_000_000; // 100 ns
const CUT_AFTER: i64 = 2 * SEC;
const OUT: &str = "target/spike_b.mp4";
const TMP: &str = "target/spike_b.mp4.tmp";
const FRAME_BMP: &str = "target/spike_b_frame.bmp";

struct Packet {
    ts: i64,
    key: bool,
    data: Vec<u8>,
}

fn main() -> Result<()> {
    // SAFETY: init COM/MF une fois, sur le thread principal.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup")?;
    }
    let packets = read_packets()?;
    let start = packets
        .iter()
        .position(|p| p.key && p.ts >= CUT_AFTER)
        .context("aucune keyframe après 2 s")?;
    let clip = &packets[start..];
    println!(
        "coupe à {:.2} s : {} paquets sur {}",
        clip[0].ts as f64 / SEC as f64,
        clip.len(),
        packets.len()
    );

    let header = sequence_header(&clip[0].data).context("SPS/PPS absents de la keyframe")?;
    let (w, h) = (1720, 720); // taille produite par le spike A sur l'écran 3440x1440
    write_mp4(clip, &header, w, h)?;
    println!("boîtes avant : {:?}", top_level_boxes(&fs::read(TMP)?));
    fs::write(OUT, faststart(fs::read(TMP)?)?)?;
    fs::remove_file(TMP)?;

    let size = fs::metadata(OUT)?.len();
    println!("MP4 : {OUT}, {} Ko", size / 1024);
    println!("boîtes : {:?}", top_level_boxes(&fs::read(OUT)?));
    verify_decode()?;
    Ok(())
}

fn read_packets() -> Result<Vec<Packet>> {
    let stream = fs::read("target/spike_a.h264").context("lance d'abord le spike A")?;
    let index = fs::read_to_string("target/spike_a.idx")?;
    let mut offset = 0;
    let mut packets = Vec::new();
    for line in index.lines() {
        let mut f = line.split(' ');
        let (Some(ts), Some(len), Some(key)) = (f.next(), f.next(), f.next()) else {
            bail!("ligne d'index invalide : {line}");
        };
        let len: usize = len.parse()?;
        packets.push(Packet {
            ts: ts.parse()?,
            key: key == "1",
            data: stream[offset..offset + len].to_vec(),
        });
        offset += len;
    }
    Ok(packets)
}

/// SPS (7) + PPS (8) avec leurs start codes : ce qu'attend MF_MT_MPEG_SEQUENCE_HEADER.
fn sequence_header(au: &[u8]) -> Option<Vec<u8>> {
    let starts: Vec<usize> = (0..au.len().saturating_sub(3))
        .filter(|&i| au[i..i + 3] == [0, 0, 1])
        .collect();
    let mut header = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(au.len(), |&e| {
            // Un start code sur 4 octets (00 00 00 01) appartient au NAL suivant.
            if e > 0 && au[e - 1] == 0 { e - 1 } else { e }
        });
        if matches!(au[s + 3] & 0x1f, 7 | 8) {
            header.extend_from_slice(&[0, 0, 0, 1]);
            header.extend_from_slice(&au[s + 3..end]);
        }
    }
    (!header.is_empty()).then_some(header)
}

fn write_mp4(clip: &[Packet], header: &[u8], w: u32, h: u32) -> Result<()> {
    let t0 = clip[0].ts;
    // SAFETY: API MF documentée ; chaque buffer est déverrouillé après copie.
    unsafe {
        // Pas de MF_MPEG4SINK_MOOV_BEFORE_MDAT : sa recopie décale le mdat d'1 Mio
        // (fichier illisible, constaté au spike B). Le faststart est fait après coup.
        // Le conteneur se déduit de l'extension, sauf si on le précise : utile pour le .tmp.
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("attributs nuls")?;
        attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        let writer = MFCreateSinkWriterFromURL(&HSTRING::from(TMP), None, &attrs)
            .context("MFCreateSinkWriterFromURL")?;

        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, (w as u64) << 32 | h as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, (FPS as u64) << 32 | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1 << 32 | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        t.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, header)?;

        let stream = writer.AddStream(&t).context("AddStream")?;
        // Même type en entrée et en sortie : pas d'encodeur, passthrough.
        writer
            .SetInputMediaType(stream, &t, None)
            .context("SetInputMediaType")?;
        writer.BeginWriting().context("BeginWriting")?;

        for p in clip {
            let buffer = MFCreateMemoryBuffer(p.data.len() as u32)?;
            let mut ptr = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(p.data.as_ptr(), ptr, p.data.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(p.data.len() as u32)?;

            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(p.ts - t0)?;
            sample.SetSampleDuration(SEC / FPS as i64)?;
            sample.SetUINT32(&MFSampleExtension_CleanPoint, p.key as u32)?;
            writer.WriteSample(stream, &sample).context("WriteSample")?;
        }
        writer.Finalize().context("Finalize")?;
    }
    Ok(())
}

/// Algorithme de qt-faststart (ffmpeg) : place `moov` devant `mdat` et décale
/// chaque offset de chunk (`stco`/`co64`) de la taille du `moov`.
fn faststart(file: Vec<u8>) -> Result<Vec<u8>> {
    let boxes = parse_boxes(&file, 0, file.len())?;
    let find = |kind: &[u8; 4]| boxes.iter().position(|b| &b.kind == kind);
    let (Some(moov), Some(mdat)) = (find(b"moov"), find(b"mdat")) else {
        bail!("moov ou mdat absent");
    };
    if moov < mdat {
        return Ok(file);
    }
    let shift = boxes[moov].len as u64;
    let mut moov_bytes = file[boxes[moov].range()].to_vec();
    patch_chunk_offsets(&mut moov_bytes, shift)?;

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
    let be32 = |i: usize| u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
    let mut boxes = Vec::new();
    let mut i = from;
    while i + 8 <= to {
        let (len, header) = match be32(i) {
            0 => (to - i, 8),
            1 => {
                let mut b = [0; 8];
                b.copy_from_slice(data.get(i + 8..i + 16).context("boîte 64 bits tronquée")?);
                (u64::from_be_bytes(b) as usize, 16)
            }
            n => (n as usize, 8),
        };
        if len < header || i + len > to {
            bail!("boîte invalide à l'offset {i}");
        }
        let kind = [data[i + 4], data[i + 5], data[i + 6], data[i + 7]];
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
                    // body : version/flags (4) + nombre d'entrées (4), puis les offsets.
                    let count = u32::from_be_bytes(moov[body + 4..body + 8].try_into()?) as usize;
                    for k in 0..count {
                        let at = body + 8 + k * size;
                        let field = moov.get_mut(at..at + size).context("table tronquée")?;
                        if wide {
                            let v = u64::from_be_bytes((&*field).try_into()?) + shift;
                            field.copy_from_slice(&v.to_be_bytes());
                        } else {
                            let v = u32::from_be_bytes((&*field).try_into()?) as u64 + shift;
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

/// Boîtes MP4 de premier niveau, dans l'ordre (faststart = moov avant mdat).
fn top_level_boxes(file: &[u8]) -> Vec<String> {
    let mut boxes = Vec::new();
    let mut i = 0;
    while i + 8 <= file.len() {
        let mut size =
            u32::from_be_bytes([file[i], file[i + 1], file[i + 2], file[i + 3]]) as usize;
        let kind = String::from_utf8_lossy(&file[i + 4..i + 8]).into_owned();
        if size == 1 && i + 16 <= file.len() {
            size = u64::from_be_bytes(file[i + 8..i + 16].try_into().unwrap_or_default()) as usize;
        }
        boxes.push(kind);
        if size < 8 {
            break;
        }
        i += size;
    }
    boxes
}

/// Relit le MP4 avec le décodeur de Windows : compte les images, exporte celle du milieu.
fn verify_decode() -> Result<()> {
    // SAFETY: API MF documentée ; buffers déverrouillés après lecture.
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("attributs nuls")?;
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(OUT), &attrs)
            .context("le MP4 ne s'ouvre pas")?;
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

        let native = reader.GetNativeMediaType(video, 0)?;
        let native_size = native.GetUINT64(&MF_MT_FRAME_SIZE)?;
        println!(
            "type relu : {}x{}, matrice YUV {}",
            native_size >> 32,
            native_size & 0xffff_ffff,
            native
                .GetUINT32(&MF_MT_YUV_MATRIX)
                .map_or("absente".into(), |m| m.to_string())
        );

        let rgb = MFCreateMediaType()?;
        rgb.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        rgb.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(video, None, &rgb)?;
        let current = reader.GetCurrentMediaType(video)?;
        let size = current.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (w, h) = ((size >> 32) as usize, (size & 0xffff_ffff) as usize);

        let mut frames = 0;
        let mut last_ts = 0;
        loop {
            let mut flags = 0;
            let mut ts = 0;
            let mut sample = None;
            reader.ReadSample(
                video,
                0,
                None,
                Some(&mut flags),
                Some(&mut ts),
                Some(&mut sample),
            )?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else { continue };
            if frames == 120 {
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut ptr = std::ptr::null_mut();
                let mut len = 0;
                buffer.Lock(&mut ptr, None, Some(&mut len))?;
                let pixels = std::slice::from_raw_parts(ptr, len as usize);
                // Le décodeur aligne les lignes : pas réel = taille / hauteur, pas w * 4.
                let stride = pixels.len() / h;
                let rows: Vec<u8> = pixels
                    .chunks(stride)
                    .take(h)
                    .flat_map(|row| &row[..w * 4])
                    .copied()
                    .collect();
                write_bmp(FRAME_BMP, w, h, &rows)?;
                buffer.Unlock()?;
            }
            frames += 1;
            last_ts = ts;
        }
        println!(
            "décodage : {frames} images, dernière à {:.2} s, image 120 → {FRAME_BMP}",
            last_ts as f64 / SEC as f64
        );
    }
    Ok(())
}

/// BMP 32 bits haut-en-bas (hauteur négative), juste pour regarder une image.
fn write_bmp(path: &str, w: usize, h: usize, bgra: &[u8]) -> Result<()> {
    let mut f = Vec::with_capacity(54 + bgra.len());
    f.extend_from_slice(b"BM");
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
    fs::write(path, f)?;
    Ok(())
}
