//! Bibliothèque de clips : liste du dossier, durée, hauteur et cadence lues dans le
//! MP4, noms acceptés (logique pure, sans API Windows).
#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail, ensure};

use crate::mp4box;

/// Le `moov` d'un clip de 5 min à 60 i/s fait quelques centaines de Ko : au-delà, le
/// fichier n'est pas un clip.
const MAX_MOOV: u64 = 16 << 20;
/// Interdits par Windows dans un nom de fichier.
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Un clip tel que la fenêtre l'affiche.
#[derive(Debug)]
pub struct ClipInfo {
    pub name: String,
    /// Dernière modification, en ms depuis 1970 (lu par `Date` en JavaScript).
    pub modified_ms: u64,
    pub size: u64,
    /// En secondes ; absente si le fichier n'est pas lisible.
    pub duration: Option<f64>,
    pub video: Option<Video>,
}

/// Piste vidéo d'un clip : hauteur (720 pour « 720p ») et images par seconde.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Video {
    pub height: u32,
    pub fps: u32,
}

impl ClipInfo {
    /// Une ligne pour la page : nom, date, taille, durée, hauteur, images/s, séparés par
    /// des tabulations (interdites dans un nom de fichier Windows). Inconnu : vide.
    pub fn line(&self) -> String {
        let duration = self.duration.map(|d| d.to_string()).unwrap_or_default();
        let (height, fps) = self.video.map_or((String::new(), String::new()), |v| {
            (v.height.to_string(), v.fps.to_string())
        });
        format!(
            "{}\t{}\t{}\t{duration}\t{height}\t{fps}",
            self.name, self.modified_ms, self.size
        )
    }
}

/// Les clips du dossier, du plus récent au plus ancien. Un dossier absent est vide.
pub fn list(dir: &Path) -> Result<Vec<ClipInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut clips = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("lecture de {}", dir.display()))? {
        // Une entrée illisible (supprimée pendant la lecture, partage réseau) est sautée
        // plutôt que de cacher toute la bibliothèque.
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || !is_mp4(&path) {
            continue;
        }
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        let info = (!online_only(&metadata))
            .then(|| file_info(&path).ok())
            .flatten();
        clips.push(ClipInfo {
            name: entry.file_name().to_string_lossy().into_owned(),
            modified_ms,
            size: metadata.len(),
            duration: info.map(|(duration, _)| duration),
            video: info.and_then(|(_, video)| video),
        });
    }
    clips.sort_by_key(|c| std::cmp::Reverse(c.modified_ms));
    Ok(clips)
}

/// Fichier OneDrive « disponible en ligne uniquement » : le lire le téléchargerait en
/// entier. Sa durée et sa piste vidéo ne sont pas lues.
#[cfg(windows)]
fn online_only(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    /// FILE_ATTRIBUTE_OFFLINE, RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS.
    const ONLINE_ONLY: u32 = 0x1000 | 0x4_0000 | 0x40_0000;
    metadata.file_attributes() & ONLINE_ONLY != 0
}

#[cfg(not(windows))]
fn online_only(_: &std::fs::Metadata) -> bool {
    false
}

fn is_mp4(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
}

/// Durée et piste vidéo d'un MP4, lues dans `moov` : seuls les en-têtes de boîtes et
/// `moov` sont lus, où que soit `moov` (devant ou derrière les images).
pub fn file_info(path: &Path) -> Result<(f64, Option<Video>)> {
    let mut file = File::open(path).with_context(|| format!("ouverture de {}", path.display()))?;
    let len = file.metadata()?.len();
    let (start, size, header) = find_box(&mut file, 0, len, b"moov")?.context("moov absent")?;
    ensure!(size <= MAX_MOOV, "moov trop grand");
    let mut moov = vec![0u8; (size - header) as usize];
    file.seek(SeekFrom::Start(start + header))?;
    file.read_exact(&mut moov)?;
    let mvhd = child(&moov, 0..moov.len(), b"mvhd")?.context("mvhd absent")?;
    Ok((mvhd_duration(&moov[mvhd])?, moov_video(&moov)))
}

/// Hauteur et cadence de la première piste vidéo, d'après le contenu de `moov` ;
/// `None` si elle manque ou est illisible.
pub fn moov_video(moov: &[u8]) -> Option<Video> {
    mp4box::parse(moov, 0, moov.len())
        .ok()?
        .iter()
        .filter(|b| &b.kind == b"trak")
        .find_map(|trak| trak_video(moov, trak.start + trak.header..trak.start + trak.len).ok()?)
}

fn trak_video(data: &[u8], trak: Range<usize>) -> Result<Option<Video>> {
    let field = |body: &Range<usize>, at: usize| -> Result<u32> {
        let bytes = data
            .get(body.start + at..body.start + at + 4)
            .filter(|_| body.start + at + 4 <= body.end)
            .context("boîte tronquée")?;
        Ok(u32::from_be_bytes(bytes.try_into()?))
    };
    let mdia = child(data, trak.clone(), b"mdia")?.context("mdia absent")?;
    let hdlr = child(data, mdia.clone(), b"hdlr")?.context("hdlr absent")?;
    if field(&hdlr, 8)?.to_be_bytes() != *b"vide" {
        return Ok(None);
    }
    // Hauteur en virgule fixe 16.16 : dernier champ de tkhd, en version 0 comme 1.
    let tkhd = child(data, trak, b"tkhd")?.context("tkhd absent")?;
    ensure!(tkhd.len() >= 8, "tkhd tronqué");
    let height = field(&tkhd, tkhd.len() - 4)? >> 16;
    // mdhd place timescale et durée comme mvhd.
    let mdhd = child(data, mdia.clone(), b"mdhd")?.context("mdhd absent")?;
    let seconds = mvhd_duration(&data[mdhd])?;
    ensure!(seconds > 0.0, "piste vidéo vide");
    let minf = child(data, mdia, b"minf")?.context("minf absent")?;
    let stbl = child(data, minf, b"stbl")?.context("stbl absent")?;
    let stsz = child(data, stbl, b"stsz")?.context("stsz absent")?;
    let frames = field(&stsz, 8)?;
    Ok(Some(Video {
        height,
        fps: (f64::from(frames) / seconds).round() as u32,
    }))
}

/// Contenu (sans en-tête) de la première boîte `kind` parmi celles de `data[within]`.
fn child(data: &[u8], within: Range<usize>, kind: &[u8; 4]) -> Result<Option<Range<usize>>> {
    Ok(mp4box::parse(data, within.start, within.end)?
        .into_iter()
        .find(|b| &b.kind == kind)
        .map(|b| b.start + b.header..b.start + b.len))
}

/// Première boîte `kind` dans `[from, to)` : (début, taille, taille de l'en-tête).
fn find_box(
    file: &mut File,
    from: u64,
    to: u64,
    kind: &[u8; 4],
) -> Result<Option<(u64, u64, u64)>> {
    let mut pos = from;
    while pos < to {
        let mut head = [0u8; 16];
        file.seek(SeekFrom::Start(pos))?;
        let read = file.read(&mut head)?;
        let (size, header, found) = mp4box::header(&head[..read]).context("en-tête tronqué")?;
        let size = if size == 0 { to - pos } else { size };
        ensure!(
            size >= header as u64 && pos.checked_add(size).is_some_and(|end| end <= to),
            "boîte invalide à l'offset {pos}"
        );
        if &found == kind {
            return Ok(Some((pos, size, header as u64)));
        }
        pos += size;
    }
    Ok(None)
}

/// Durée d'après le contenu de `mvhd` ou `mdhd` (versions 0 et 1).
pub fn mvhd_duration(body: &[u8]) -> Result<f64> {
    let field = |at: usize, len: usize| body.get(at..at + len).context("mvhd tronqué");
    let (timescale, duration) = match body.first() {
        Some(0) => (
            field(12, 4)?,
            u64::from(u32::from_be_bytes(field(16, 4)?.try_into()?)),
        ),
        Some(1) => (field(20, 4)?, u64::from_be_bytes(field(24, 8)?.try_into()?)),
        _ => bail!("version de mvhd inconnue"),
    };
    let timescale = u32::from_be_bytes(timescale.try_into()?);
    ensure!(timescale > 0, "timescale nul");
    Ok(duration as f64 / f64::from(timescale))
}

/// Un nom venu de la page ne doit désigner qu'un clip du dossier : pas de `..`, de
/// séparateur, ni de flux NTFS (`clip.mp4:x`).
pub fn clip_path(dir: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    ensure!(
        path.file_name() == Some(OsStr::new(name))
            && !name.contains([':', '/', '\\'])
            && is_mp4(path),
        "nom de clip invalide : {name}"
    );
    Ok(dir.join(name))
}

/// Nom de fichier saisi pour un renommage : espaces retirés, `.mp4` ajouté si absent.
pub fn new_name(input: &str) -> Result<String> {
    let name = input.trim();
    ensure!(!name.is_empty(), "Nom vide");
    ensure!(
        !name
            .chars()
            .any(|c| c.is_control() || FORBIDDEN.contains(&c)),
        "Caractères interdits : {}",
        FORBIDDEN
            .iter()
            .map(char::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    );
    // Windows retire un point final, et réserve ces noms aux périphériques.
    ensure!(!name.ends_with('.'), "Point final interdit");
    let stem = name.split('.').next().unwrap_or(name);
    ensure!(
        !RESERVED.contains(&stem.to_ascii_lowercase().as_str()),
        "Nom réservé par Windows"
    );
    Ok(if is_mp4(Path::new(name)) {
        name.to_owned()
    } else {
        format!("{name}.mp4")
    })
}

/// Renomme un clip sans jamais en écraser un autre ; renvoie le nouveau nom.
pub fn rename(dir: &Path, old: &str, input: &str) -> Result<String> {
    let from = clip_path(dir, old)?;
    ensure!(from.is_file(), "Clip introuvable");
    let new = new_name(input)?;
    let to = clip_path(dir, &new)?;
    // `fs::rename` remplace une destination existante sous Windows. Seul cas permis :
    // la destination est ce même fichier (changement de casse, que NTFS ignore).
    if to.exists() && !same_file(&from, &to)? {
        bail!("Nom déjà pris");
    }
    std::fs::rename(&from, &to)
        .with_context(|| format!("renommage de {old}"))
        .context("Clip en cours d'utilisation")?;
    Ok(new)
}

/// Même fichier sur disque, quelle que soit la casse demandée : le chemin réel est
/// comparé, pas les noms (le repli de casse de NTFS n'est pas celui de Rust).
fn same_file(a: &Path, b: &Path) -> Result<bool> {
    let real =
        |p: &Path| std::fs::canonicalize(p).with_context(|| format!("chemin de {}", p.display()));
    Ok(real(a)? == real(b)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((8 + body.len()) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    /// mvhd v0 : version/flags, création, modification, timescale, durée.
    fn mvhd_v0(timescale: u32, duration: u32) -> Vec<u8> {
        let mut body = vec![0u8; 12];
        body.extend_from_slice(&timescale.to_be_bytes());
        body.extend_from_slice(&duration.to_be_bytes());
        body.extend_from_slice(&[0; 80]);
        bx(b"mvhd", &body)
    }

    fn mvhd_v1(timescale: u32, duration: u64) -> Vec<u8> {
        let mut body = vec![1, 0, 0, 0];
        body.extend_from_slice(&[0; 16]);
        body.extend_from_slice(&timescale.to_be_bytes());
        body.extend_from_slice(&duration.to_be_bytes());
        body.extend_from_slice(&[0; 80]);
        bx(b"mvhd", &body)
    }

    fn moov(children: &[Vec<u8>]) -> Vec<u8> {
        bx(b"moov", &children.concat())
    }

    fn body(mvhd: &[u8]) -> &[u8] {
        &mvhd[8..]
    }

    #[test]
    fn reads_mvhd_v0_duration() {
        assert_eq!(mvhd_duration(body(&mvhd_v0(1000, 30_620))).unwrap(), 30.62);
    }

    #[test]
    fn reads_mvhd_v1_duration() {
        assert_eq!(
            mvhd_duration(body(&mvhd_v1(90_000, 2_700_000))).unwrap(),
            30.0
        );
    }

    #[test]
    fn truncated_or_unknown_mvhd_is_an_error() {
        assert!(mvhd_duration(&[0; 10]).is_err());
        assert!(mvhd_duration(&[]).is_err());
        assert!(mvhd_duration(&[2; 120]).is_err());
        assert!(mvhd_duration(body(&mvhd_v0(0, 100))).is_err());
    }

    fn file_duration(path: &Path) -> Result<f64> {
        file_info(path).map(|(duration, _)| duration)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clipper-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_duration_skips_mdat_wherever_moov_is() {
        let dir = temp_dir("duration");
        let mdat = bx(b"mdat", &[0; 5000]);
        let ftyp = bx(b"ftyp", b"isom");
        let fast = [ftyp.clone(), moov(&[mvhd_v0(1000, 31_000)]), mdat.clone()].concat();
        // mvhd pas en premier, moov derrière les images.
        let slow = [
            ftyp.clone(),
            mdat,
            moov(&[bx(b"udta", &[1, 2, 3]), mvhd_v0(1000, 12_500)]),
        ]
        .concat();
        let no_mvhd = [ftyp.clone(), moov(&[bx(b"udta", &[])])].concat();
        std::fs::write(dir.join("no_mvhd.mp4"), no_mvhd).unwrap();
        assert!(file_duration(&dir.join("no_mvhd.mp4")).is_err());
        std::fs::write(dir.join("fast.mp4"), fast).unwrap();
        std::fs::write(dir.join("slow.mp4"), slow).unwrap();
        assert_eq!(file_duration(&dir.join("fast.mp4")).unwrap(), 31.0);
        assert_eq!(file_duration(&dir.join("slow.mp4")).unwrap(), 12.5);
        std::fs::write(dir.join("broken.mp4"), b"pas un mp4").unwrap();
        assert!(file_duration(&dir.join("broken.mp4")).is_err());
        let video = trak(b"vide", 1080, 15_360, 15_360 * 31, 930);
        let full = [ftyp, moov(&[mvhd_v0(1000, 31_000), video])].concat();
        std::fs::write(dir.join("full.mp4"), full).unwrap();
        let (duration, video) = file_info(&dir.join("full.mp4")).unwrap();
        assert_eq!(duration, 31.0);
        assert_eq!(
            video,
            Some(Video {
                height: 1080,
                fps: 30
            })
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn huge_box_size_is_an_error_not_an_overflow() {
        let dir = temp_dir("overflow");
        // Boîte 64 bits dont la taille ferait déborder position + taille.
        let mut file = bx(b"ftyp", b"isom");
        let at = file.len() as u64;
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(b"free");
        file.extend_from_slice(&(u64::MAX - at + 1).to_be_bytes());
        std::fs::write(dir.join("evil.mp4"), file).unwrap();
        assert!(file_duration(&dir.join("evil.mp4")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_only_mp4_newest_first() {
        let dir = temp_dir("list");
        let clip = [bx(b"ftyp", b"isom"), moov(&[mvhd_v0(1000, 30_000)])].concat();
        std::fs::write(dir.join("old.mp4"), &clip).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("new.mp4"), &clip).unwrap();
        std::fs::write(dir.join("writing.mp4.tmp"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"").unwrap();
        let clips = list(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let names: Vec<_> = clips.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["new.mp4", "old.mp4"]);
        assert_eq!(clips[0].size, clip.len() as u64);
        assert_eq!(clips[0].duration, Some(30.0));
    }

    #[test]
    fn missing_folder_is_an_empty_list() {
        let dir = std::env::temp_dir().join("clipper-lib-absent-dossier");
        assert!(list(&dir).unwrap().is_empty());
    }

    #[test]
    fn page_line_uses_tabs() {
        let clip = ClipInfo {
            name: "clip.mp4".into(),
            modified_ms: 1_760_000_000_000,
            size: 18_400_000,
            duration: Some(30.62),
            video: Some(Video {
                height: 720,
                fps: 60,
            }),
        };
        assert_eq!(
            clip.line(),
            "clip.mp4\t1760000000000\t18400000\t30.62\t720\t60"
        );
        let unknown = ClipInfo {
            duration: None,
            video: None,
            ..clip
        };
        assert_eq!(unknown.line(), "clip.mp4\t1760000000000\t18400000\t\t\t");
    }

    /// Piste minimale : tkhd (hauteur en 16.16 à la fin), mdia/mdhd, hdlr, minf/stbl/stsz.
    fn trak(handler: &[u8; 4], height: u32, timescale: u32, duration: u32, count: u32) -> Vec<u8> {
        let mut tkhd = vec![0u8; 76];
        tkhd.extend_from_slice(&(1720u32 << 16).to_be_bytes());
        tkhd.extend_from_slice(&(height << 16).to_be_bytes());
        let mut mdhd = vec![0u8; 12];
        mdhd.extend_from_slice(&timescale.to_be_bytes());
        mdhd.extend_from_slice(&duration.to_be_bytes());
        mdhd.extend_from_slice(&[0; 4]);
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(handler);
        hdlr.extend_from_slice(&[0; 13]);
        let mut stsz = vec![0u8; 8];
        stsz.extend_from_slice(&count.to_be_bytes());
        let stbl = bx(b"stbl", &bx(b"stsz", &stsz));
        let mdia = [bx(b"mdhd", &mdhd), bx(b"hdlr", &hdlr), bx(b"minf", &stbl)].concat();
        bx(b"trak", &[bx(b"tkhd", &tkhd), bx(b"mdia", &mdia)].concat())
    }

    #[test]
    fn reads_height_and_fps_of_the_video_track() {
        let sound = trak(b"soun", 0, 48_000, 48_000 * 30, 1406);
        let video = trak(b"vide", 720, 15_360, 15_360 * 30, 1800);
        let body = [mvhd_v0(1000, 30_000), sound, video].concat();
        assert_eq!(
            moov_video(&body),
            Some(Video {
                height: 720,
                fps: 60
            })
        );
        // 29,97 i/s s'affiche 30.
        let ntsc = trak(b"vide", 1440, 30_000, 30_000 * 30, 899);
        assert_eq!(moov_video(&ntsc).map(|v| v.fps), Some(30));
    }

    #[test]
    fn no_or_broken_video_track_is_none() {
        assert_eq!(moov_video(&trak(b"soun", 0, 48_000, 48_000, 47)), None);
        assert_eq!(moov_video(&trak(b"vide", 720, 15_360, 0, 1800)), None);
        assert_eq!(moov_video(&bx(b"trak", &bx(b"tkhd", &[0; 4]))), None);
        assert_eq!(moov_video(b"pas un moov"), None);
    }

    #[test]
    fn clip_names_stay_inside_the_folder() {
        let dir = Path::new(r"C:\Clips");
        assert_eq!(clip_path(dir, "clip.mp4").unwrap(), dir.join("clip.mp4"));
        for bad in [
            "",
            "..",
            r"..\clip.mp4",
            "a/clip.mp4",
            r"a\clip.mp4",
            "clip.mp4:x",
            "clipper.toml",
            "clip.mp4.tmp",
        ] {
            assert!(clip_path(dir, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rename_adds_the_extension_and_trims() {
        assert_eq!(new_name("  Gros clutch ").unwrap(), "Gros clutch.mp4");
        assert_eq!(new_name("ace.mp4").unwrap(), "ace.mp4");
        assert_eq!(new_name("ace.MP4").unwrap(), "ace.MP4");
    }

    #[test]
    fn rename_rejects_names_windows_refuses() {
        for bad in [
            "", "   ", "a/b", r"a\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "con",
            "fin.",
        ] {
            assert!(new_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rename_never_overwrites() {
        let dir = temp_dir("rename");
        std::fs::write(dir.join("a.mp4"), b"a").unwrap();
        std::fs::write(dir.join("b.mp4"), b"b").unwrap();
        assert!(rename(&dir, "a.mp4", "b").is_err());
        assert_eq!(std::fs::read(dir.join("b.mp4")).unwrap(), b"b");
        assert_eq!(rename(&dir, "a.mp4", "ace").unwrap(), "ace.mp4");
        assert!(dir.join("ace.mp4").exists() && !dir.join("a.mp4").exists());
        // Dossier sensible à la casse : « A.mp4 » est un autre clip que « a.mp4 », il ne
        // doit pas être écrasé. Sinon (NTFS par défaut), c'est le même fichier et
        // changer la casse est permis.
        std::fs::write(dir.join("A.mp4"), b"autre").unwrap();
        std::fs::write(dir.join("a.mp4"), b"a").unwrap();
        // Sensible à la casse si la seconde écriture n'a pas remplacé la première.
        let distinct = std::fs::read(dir.join("A.mp4")).unwrap() == b"autre";
        let result = rename(&dir, "a.mp4", "A");
        if distinct {
            assert!(result.is_err());
            assert_eq!(std::fs::read(dir.join("A.mp4")).unwrap(), b"autre");
        } else {
            assert_eq!(result.unwrap(), "A.mp4");
        }
        // Changer seulement la casse, accents compris : pas une collision.
        std::fs::write(dir.join("été.mp4"), b"e").unwrap();
        assert_eq!(rename(&dir, "été.mp4", "Été").unwrap(), "Été.mp4");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
