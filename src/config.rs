//! Configuration (`clipper.toml` à côté de l'exe) et calculs dérivés (logique pure).
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

/// Mesuré au spike A : le CBR AMF dépasse la cible de 6-7 % même avec VBV d'1 s.
const BITRATE_MARGIN: f64 = 0.10;
const MIN_VIDEO_BPS: u32 = 500_000;

#[derive(Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Durée d'un clip, en secondes.
    pub clip_seconds: u32,
    /// Hauteur de sortie ; la largeur suit le ratio de l'écran.
    pub height: u32,
    pub fps: u32,
    /// Taille maximale d'un clip, en Mo (10^6 octets). Limite Discord gratuite : 20.
    pub target_mb: f64,
    /// Ex. "Alt+F10", "Ctrl+Shift+S".
    pub hotkey: String,
    /// Relatif au dossier Vidéos de Windows, ou absolu.
    pub output_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            clip_seconds: 30,
            height: 720,
            fps: 60,
            target_mb: 19.0,
            hotkey: "Alt+F10".into(),
            output_dir: "Clipper".into(),
        }
    }
}

impl Config {
    /// Lit `path`, ou l'écrit avec les valeurs par défaut s'il n'existe pas.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let config = Self::default();
            std::fs::write(path, toml::to_string_pretty(&config)?)
                .with_context(|| format!("écriture de {}", path.display()))?;
            return Ok(config);
        }
        let text = std::fs::read_to_string(path)?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("lecture de {}", path.display()))?;
        ensure!(config.clip_seconds > 0, "clip_seconds doit être > 0");
        ensure!(config.fps > 0, "fps doit être > 0");
        ensure!(
            config.height >= 2 && config.height.is_multiple_of(2),
            "height doit être pair"
        );
        Ok(config)
    }

    /// Débit vidéo pour qu'un clip (+ 1 s de GOP au pire) tienne sous `target_mb`.
    pub fn video_bitrate(&self, audio_bps: u32) -> Result<u32> {
        video_bitrate(self.target_mb, self.clip_seconds, 1, audio_bps)
    }
}

pub fn video_bitrate(target_mb: f64, clip_s: u32, gop_s: u32, audio_bps: u32) -> Result<u32> {
    let total_bps = target_mb * 1e6 * 8.0 * (1.0 - BITRATE_MARGIN) / f64::from(clip_s + gop_s);
    let video_bps = total_bps - f64::from(audio_bps);
    ensure!(
        video_bps >= f64::from(MIN_VIDEO_BPS),
        "target_mb trop petit : {:.0} kb/s de vidéo (minimum {})",
        video_bps / 1000.0,
        MIN_VIDEO_BPS / 1000
    );
    Ok(video_bps as u32)
}

/// Modificateurs au format de `RegisterHotKey` (MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN).
#[derive(Debug, PartialEq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

pub fn parse_hotkey(text: &str) -> Result<Hotkey> {
    let mut modifiers = 0;
    let mut vk = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "alt" => modifiers |= 0x1,
            "ctrl" | "control" => modifiers |= 0x2,
            "shift" => modifiers |= 0x4,
            "win" => modifiers |= 0x8,
            key => {
                if vk.is_some() {
                    bail!("raccourci « {text} » : une seule touche hors modificateurs");
                }
                vk = Some(virtual_key(key).with_context(|| format!("touche inconnue : {part}"))?);
            }
        }
    }
    let vk = vk.with_context(|| format!("raccourci « {text} » : touche manquante"))?;
    Ok(Hotkey { modifiers, vk })
}

/// F1–F24, A–Z, 0–9 : les codes VK Windows correspondants.
fn virtual_key(key: &str) -> Option<u32> {
    let bytes = key.as_bytes();
    match bytes {
        [c] if c.is_ascii_alphanumeric() => Some(u32::from(c.to_ascii_uppercase())),
        [b'f', ..] => match key[1..].parse::<u32>() {
            Ok(n @ 1..=24) => Some(0x70 + n - 1),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bitrate_fits_discord() {
        // 19 Mo × 8 × 0,9 / 31 s − 160 kb/s d'audio ≈ 4,25 Mb/s
        let bps = Config::default().video_bitrate(160_000).unwrap();
        assert_eq!(bps / 1000, 4252);
    }

    #[test]
    fn bitrate_too_low_is_an_error() {
        assert!(video_bitrate(1.0, 30, 1, 160_000).is_err());
    }

    #[test]
    fn parses_hotkeys() {
        assert_eq!(
            parse_hotkey("Alt+F10").unwrap(),
            Hotkey {
                modifiers: 0x1,
                vk: 0x79
            }
        );
        assert_eq!(
            parse_hotkey("ctrl + shift + s").unwrap(),
            Hotkey {
                modifiers: 0x6,
                vk: b'S' as u32
            }
        );
        assert_eq!(
            parse_hotkey("Win+9").unwrap(),
            Hotkey {
                modifiers: 0x8,
                vk: b'9' as u32
            }
        );
        assert_eq!(
            parse_hotkey("F24").unwrap(),
            Hotkey {
                modifiers: 0,
                vk: 0x87
            }
        );
    }

    #[test]
    fn rejects_bad_hotkeys() {
        for bad in ["Alt", "Alt+F25", "Alt+F0", "Ctrl+A+B", "Alt+Tab", ""] {
            assert!(parse_hotkey(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn unknown_field_is_an_error() {
        assert!(toml::from_str::<Config>("clip_second = 30").is_err());
    }

    #[test]
    fn missing_fields_take_defaults() {
        let config: Config = toml::from_str("clip_seconds = 20").unwrap();
        assert_eq!(config.clip_seconds, 20);
        assert_eq!(config.fps, 60);
    }

    #[test]
    fn creates_default_file_then_reads_it_back() {
        let path = std::env::temp_dir().join(format!("clipper-test-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let created = Config::load_or_create(&path).unwrap();
        let read = Config::load_or_create(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(created.hotkey, read.hotkey);
    }
}
