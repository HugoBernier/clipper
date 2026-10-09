//! Configuration (`clipper.toml` à côté de l'exe) et calculs dérivés (logique pure).
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

/// Mesuré au spike A : le CBR AMF dépasse la cible de 6-7 % même avec VBV d'1 s.
const BITRATE_MARGIN: f64 = 0.10;
const MIN_VIDEO_BPS: u32 = 500_000;
/// Bits par pixel en « Moyenne » à 60 i/s : redonne le débit de la 0.1 (4,25 Mb/s en
/// 1720×720, l'écran 21:9 de référence, quand la taille visait 19 Mo pour 30 s).
const MEDIUM_BITS_PER_PIXEL: f64 = 0.0572;
/// Deux images voisines se ressemblent d'autant plus que les fps montent : le débit
/// croît moins vite qu'eux (débits conseillés par YouTube : 1440p60 ≈ 1,5 × 1440p30).
const FPS_EXPONENT: f64 = 0.6;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Durée d'un clip, en secondes (10 à 300).
    pub clip_seconds: u32,
    /// Hauteur de sortie ; la largeur suit le ratio de l'écran.
    pub height: u32,
    pub fps: u32,
    pub quality: Quality,
    /// Ex. "Alt+F10", "Ctrl+Shift+S".
    pub hotkey: String,
    /// Relatif au dossier Vidéos de Windows, ou absolu.
    pub output_dir: PathBuf,
    /// Mixer le micro de communication (celui de Discord) au son du PC.
    pub microphone: bool,
    /// Volume du micro dans le clip, en % (0 à 200).
    pub microphone_volume: u32,
    /// Taille cible de la 0.1 : lue pour accepter un ancien fichier, jamais réécrite.
    #[serde(skip_serializing)]
    target_mb: Option<f64>,
}

/// Niveau de qualité : fixe le débit, donc la netteté ; la taille suit la durée.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Low,
    #[default]
    Medium,
    High,
    VeryHigh,
}

impl Quality {
    const ALL: [Quality; 4] = [
        Quality::Low,
        Quality::Medium,
        Quality::High,
        Quality::VeryHigh,
    ];

    /// Nom dans `clipper.toml` et dans la fenêtre de réglages.
    fn as_str(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Medium => "medium",
            Quality::High => "high",
            Quality::VeryHigh => "very_high",
        }
    }

    fn factor(self) -> f64 {
        match self {
            Quality::Low => 0.5,
            Quality::Medium => 1.0,
            Quality::High => 1.5,
            Quality::VeryHigh => 2.5,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            clip_seconds: 30,
            height: 720,
            fps: 60,
            quality: Quality::default(),
            hotkey: "Alt+F10".into(),
            output_dir: "Clipper".into(),
            microphone: true,
            microphone_volume: 100,
            target_mb: None,
        }
    }
}

impl Config {
    /// Lit `path`, ou l'écrit avec les valeurs par défaut s'il n'existe pas.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let config = Self::default();
            config.save(path)?;
            return Ok(config);
        }
        let text = std::fs::read_to_string(path)?;
        let mut config: Self =
            toml::from_str(&text).with_context(|| format!("lecture de {}", path.display()))?;
        // La 0.1 acceptait toute durée > 0 : on la ramène dans les bornes plutôt que de
        // refuser de démarrer.
        config.clip_seconds = config.clip_seconds.clamp(10, 300);
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            (10..=300).contains(&self.clip_seconds),
            "clip_seconds doit être entre 10 et 300"
        );
        ensure!(self.fps > 0, "fps doit être > 0");
        ensure!(
            self.height >= 2 && self.height.is_multiple_of(2),
            "height doit être pair"
        );
        ensure!(
            self.microphone_volume <= 200,
            "microphone_volume doit être entre 0 et 200"
        );
        Ok(())
    }

    /// Change un réglage (`clé`, valeur texte, venus de la fenêtre) ; refusé et sans
    /// effet si la config qui en résulte n'est pas valide.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let mut next = self.clone();
        let number = || {
            value
                .parse::<u32>()
                .with_context(|| format!("{key} : nombre attendu, reçu « {value} »"))
        };
        match key {
            "clip_seconds" => next.clip_seconds = number()?,
            "height" => next.height = number()?,
            "fps" => next.fps = number()?,
            "microphone_volume" => next.microphone_volume = number()?,
            "quality" => {
                next.quality = *Quality::ALL
                    .iter()
                    .find(|q| q.as_str() == value)
                    .with_context(|| format!("qualité inconnue : {value}"))?;
            }
            "microphone" => {
                next.microphone = value
                    .parse()
                    .with_context(|| format!("microphone : true ou false, reçu « {value} »"))?;
            }
            "hotkey" => {
                parse_hotkey(value)?;
                next.hotkey = value.into();
            }
            "output_dir" => {
                ensure!(!value.is_empty(), "dossier des clips vide");
                next.output_dir = value.into();
            }
            _ => bail!("réglage inconnu : {key}"),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Les réglages en `(clé, valeur)`, au format qu'accepte `set`.
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("clip_seconds", self.clip_seconds.to_string()),
            ("height", self.height.to_string()),
            ("fps", self.fps.to_string()),
            ("quality", self.quality.as_str().into()),
            ("hotkey", self.hotkey.clone()),
            ("output_dir", self.output_dir.display().to_string()),
            ("microphone", self.microphone.to_string()),
            ("microphone_volume", self.microphone_volume.to_string()),
        ]
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, toml::to_string_pretty(self)?)
            .with_context(|| format!("écriture de {}", path.display()))
    }
}

/// Débit vidéo d'une sortie `width`×`height` à `fps` pour un niveau de qualité.
pub fn video_bitrate(quality: Quality, width: u32, height: u32, fps: u32) -> u32 {
    let pixels = f64::from(width) * f64::from(height) * 60.0;
    let bps = MEDIUM_BITS_PER_PIXEL
        * quality.factor()
        * pixels
        * (f64::from(fps) / 60.0).powf(FPS_EXPONENT);
    (bps as u32).max(MIN_VIDEO_BPS)
}

/// Taille d'un clip à prévoir, en Mo (10^6 octets) : dépassement de l'encodeur compris,
/// et 1 s de plus au pire (le clip démarre à la keyframe qui précède).
pub fn estimated_mb(video_bps: u32, audio_bps: u32, clip_seconds: u32) -> f64 {
    let bps = f64::from(video_bps) * (1.0 + BITRATE_MARGIN) + f64::from(audio_bps);
    bps * f64::from(clip_seconds + 1) / 8e6
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
    // Lettre ou chiffre seuls : RegisterHotKey les prendrait à toutes les applications.
    let function_key = (0x70..=0x87).contains(&vk);
    ensure!(
        modifiers != 0 || function_key,
        "raccourci « {text} » : une lettre ou un chiffre demandent Ctrl, Alt, Shift ou Win"
    );
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

    /// Sortie 720p de l'écran 21:9 de référence (3440×1440).
    const UW_720: (u32, u32) = (1720, 720);

    #[test]
    fn medium_720p60_keeps_the_previous_bitrate() {
        // 0.1.x : 19 Mo × 8 × 0,9 / 31 s − 160 kb/s d'audio ≈ 4,25 Mb/s.
        let bps = video_bitrate(Quality::Medium, UW_720.0, UW_720.1, 60);
        assert_eq!(bps / 10_000, 425);
    }

    #[test]
    fn bitrate_follows_quality() {
        let at = |q| video_bitrate(q, 1920, 1080, 60);
        assert!(at(Quality::Low) < at(Quality::Medium));
        assert!(at(Quality::Medium) < at(Quality::High));
        assert!(at(Quality::High) < at(Quality::VeryHigh));
    }

    #[test]
    fn bitrate_follows_pixels() {
        let p720 = video_bitrate(Quality::Medium, 1280, 720, 60);
        let p1440 = video_bitrate(Quality::Medium, 2560, 1440, 60);
        assert_eq!(p1440 / 1000, p720 * 4 / 1000);
    }

    #[test]
    fn doubling_fps_costs_about_half_more() {
        // Comme les débits conseillés par YouTube : 1440p60 ≈ 1,5 × 1440p30.
        let p30 = f64::from(video_bitrate(Quality::Medium, 2560, 1440, 30));
        let p60 = f64::from(video_bitrate(Quality::Medium, 2560, 1440, 60));
        assert!((1.45..1.6).contains(&(p60 / p30)), "{}", p60 / p30);
    }

    #[test]
    fn bitrate_has_a_floor() {
        assert_eq!(video_bitrate(Quality::Low, 64, 36, 30), MIN_VIDEO_BPS);
    }

    #[test]
    fn estimate_covers_encoder_overshoot_and_one_gop() {
        // (4 Mb/s × 1,1 + 160 kb/s) × 31 s / 8 = 17,67 Mo
        let mb = estimated_mb(4_000_000, 160_000, 30);
        assert!((mb - 17.67).abs() < 0.01, "{mb}");
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
        // Une lettre ou un chiffre seuls seraient pris à toutes les applications.
        for bad in [
            "Alt", "Alt+F25", "Alt+F0", "Ctrl+A+B", "Alt+Tab", "", "A", "3", "shift",
        ] {
            assert!(parse_hotkey(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn function_key_alone_is_allowed() {
        assert_eq!(
            parse_hotkey("F9").unwrap(),
            Hotkey {
                modifiers: 0,
                vk: 0x78
            }
        );
    }

    #[test]
    fn out_of_range_duration_from_an_old_file_is_clamped() {
        // La 0.1 acceptait toute durée > 0 : un ancien fichier ne doit pas empêcher
        // Clipper de démarrer.
        for (seconds, kept) in [(5, 10), (600, 300)] {
            let path = temp_path(&format!("clamp-{seconds}"));
            std::fs::write(&path, format!("clip_seconds = {seconds}")).unwrap();
            let config = Config::load_or_create(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert_eq!(config.clip_seconds, kept);
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
        assert_eq!(config.quality, Quality::Medium);
        assert!(config.microphone);
        assert_eq!(config.microphone_volume, 100);
    }

    #[test]
    fn microphone_can_be_disabled() {
        let config: Config = toml::from_str("microphone = false").unwrap();
        assert!(!config.microphone);
    }

    #[test]
    fn quality_is_written_in_snake_case() {
        let config: Config = toml::from_str(r#"quality = "very_high""#).unwrap();
        assert_eq!(config.quality, Quality::VeryHigh);
        assert!(
            toml::to_string(&config)
                .unwrap()
                .contains(r#"quality = "very_high""#)
        );
    }

    #[test]
    fn window_and_file_use_the_same_quality_names() {
        for q in Quality::ALL {
            let text = toml::to_string(&Config {
                quality: q,
                ..Config::default()
            })
            .unwrap();
            assert!(
                text.contains(&format!(r#"quality = "{}""#, q.as_str())),
                "{text}"
            );
        }
    }

    #[test]
    fn reads_a_0_1_config_and_drops_target_mb() {
        let old = r#"
clip_seconds = 30
height = 1440
fps = 60
target_mb = 48.0
hotkey = "Ctrl+Shift+S"
output_dir = "Clipper"
microphone = true
"#;
        let path = temp_path("old");
        std::fs::write(&path, old).unwrap();
        let config = Config::load_or_create(&path).unwrap();
        assert_eq!((config.height, config.fps), (1440, 60));
        assert_eq!(config.hotkey, "Ctrl+Shift+S");
        assert_eq!(config.quality, Quality::Medium);
        config.save(&path).unwrap();
        let rewritten = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(!rewritten.contains("target_mb"), "{rewritten}");
    }

    #[test]
    fn duration_must_stay_between_10_s_and_5_min() {
        for (seconds, ok) in [(9, false), (10, true), (300, true), (301, false)] {
            let config = Config {
                clip_seconds: seconds,
                ..Config::default()
            };
            assert_eq!(config.validate().is_ok(), ok, "{seconds} s");
        }
    }

    #[test]
    fn microphone_volume_must_stay_between_0_and_200() {
        for (volume, ok) in [(0, true), (200, true), (201, false)] {
            let config = Config {
                microphone_volume: volume,
                ..Config::default()
            };
            assert_eq!(config.validate().is_ok(), ok, "{volume} %");
        }
    }

    #[test]
    fn odd_height_or_zero_fps_is_an_error() {
        let odd = Config {
            height: 721,
            ..Config::default()
        };
        let still = Config {
            fps: 0,
            ..Config::default()
        };
        assert!(odd.validate().is_err());
        assert!(still.validate().is_err());
        assert!(Config::default().validate().is_ok());
    }

    #[test]
    fn creates_default_file_then_reads_it_back() {
        let path = temp_path("new");
        let _ = std::fs::remove_file(&path);
        let created = Config::load_or_create(&path).unwrap();
        let read = Config::load_or_create(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(created.hotkey, read.hotkey);
        assert_eq!(created.quality, read.quality);
    }

    #[test]
    fn set_changes_one_setting() {
        let mut config = Config::default();
        config.set("height", "1440").unwrap();
        config.set("fps", "144").unwrap();
        config.set("quality", "very_high").unwrap();
        config.set("clip_seconds", "120").unwrap();
        config.set("microphone", "false").unwrap();
        config.set("microphone_volume", "150").unwrap();
        config.set("hotkey", "Ctrl+Shift+S").unwrap();
        config.set("output_dir", r"D:\Clips").unwrap();
        assert_eq!((config.height, config.fps), (1440, 144));
        assert_eq!(config.quality, Quality::VeryHigh);
        assert_eq!(config.clip_seconds, 120);
        assert!(!config.microphone);
        assert_eq!(config.microphone_volume, 150);
        assert_eq!(config.hotkey, "Ctrl+Shift+S");
        assert_eq!(config.output_dir, PathBuf::from(r"D:\Clips"));
    }

    #[test]
    fn invalid_set_leaves_the_config_unchanged() {
        let mut config = Config::default();
        for (key, value) in [
            ("clip_seconds", "5"),
            ("clip_seconds", "trente"),
            ("microphone_volume", "300"),
            ("quality", "ultra"),
            ("hotkey", "Alt+Tab"),
            ("microphone", "oui"),
            ("output_dir", ""),
            ("couleur", "bleu"),
        ] {
            assert!(config.set(key, value).is_err(), "{key}={value}");
        }
        assert_eq!(config.pairs(), Config::default().pairs());
    }

    #[test]
    fn pairs_round_trip_through_set() {
        let mut config = Config::default();
        config.set("quality", "high").unwrap();
        config.set("output_dir", r"C:\Vidéos\Clips").unwrap();
        let mut copy = Config::default();
        for (key, value) in config.pairs() {
            copy.set(key, &value).unwrap();
        }
        assert_eq!(copy.pairs(), config.pairs());
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("clipper-{name}-{}.toml", std::process::id()))
    }
}
