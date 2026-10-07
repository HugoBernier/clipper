# Architecture

Légende : **[V]** vérifié en ligne le 2026-10-07 · **[S]** supposé, à confirmer par un spike.

**Limite Discord** : 20 Mo pour les comptes gratuits depuis le 14/08/2026 [V] ([Dexerto](https://www.dexerto.com/entertainment/discord-raises-free-upload-limit-to-20mb-after-lowering-it-two-years-ago-3398251/)). Cible configurable `target_mb = 19`. Pour 30 s, ça donne ~4,5 Mb/s de vidéo, soit du 720p60 confortable ou du 1080p30.

**Taille de sortie** : hauteur fixe 720, largeur au ratio de l'écran, arrondie au pair (écran 3440×1440 → 1720×720). On n'étire jamais l'image.

## 1. Crates

| Crate | Version | Rôle | Verdict |
|---|---|---|---|
| `windows` | 0.62.2 [V] | WGC, D3D11, Media Foundation, RegisterHotKey | Indispensable |
| ~~`windows-capture`~~ | 2.0.1 [V] | Capture WGC | **Écarté (spike A)** : il crée son propre device D3D11 sans `VIDEO_SUPPORT` ni protection multithread, et on ne peut pas lui en fournir un. WGC est appelé directement via `windows` (~40 lignes). |
| `wasapi` | 0.25.0 [V] | Loopback | Prendre. La position QPC est [S] |
| `serde` + `toml` | toml 1.1.6 [V] | Config | Prendre |
| `anyhow` | 1.0.104 [V] | Erreurs | Prendre |
| `log` + `simplelog` | 0.12.2 [V] | Log dans un fichier | Prendre |
| ~~`mp4`~~ | 0.14.0, figé depuis 2023 [V] | Muxer | Plan B seulement. Muxing via IMFSinkWriter en passthrough [S] |

Encodage : MFT Media Foundation appelés directement (H.264 AMF matériel + AAC Microsoft).

## 2. Arborescence

```
Cargo.toml
clipper.toml.example
src/
  main.rs    # windows_subsystem, logger, config, threads, boucle WM_HOTKEY
  config.rs  # Config (serde, valeurs par défaut) + calcul du bitrate (pur)
  ring.rs    # Packet, Ring, éviction, snapshot coupé à la keyframe (pur, TDD)
  mf.rs      # helpers MF partagés : MFStartup, media types, MFT async
  video.rs   # WGC → VideoProcessor (scale + NV12, GPU) → MFT H.264
  audio.rs   # loopback WASAPI → bouchage des silences, f32→i16 → MFT AAC
  save.rs    # snapshot → SinkWriter passthrough → .tmp puis rename
```

## 3. Flux et threads

```
 [callback WGC]                      [thread audio]
 image (si l'écran change)           paquet loopback (f32, ts QPC)
   │ scale+NV12 BT.709 (GPU)           │ silences, f32→i16
   ▼ dernière image (Mutex)            │
 [thread vidéo, horloge 60 fps]        │
   │ dernière image, ts = t0 + n/60    │
   ▼ MFT H.264 AMF                     ▼ MFT AAC
   Packet ──┐                   ┌── Packet
            ▼                   ▼
       Arc<Mutex<Ring>>  (video + audio VecDeque)
            ▲ lock court : clone d'Arc<[u8]>
 [thread principal] GetMessage → WM_HOTKEY → snapshot()
            │ spawn
            ▼
 [thread save] SinkWriter → clip_YYYYMMDD_HHMMSS.mp4.tmp → rename → MessageBeep
```

- Un Mutex plutôt que des canaux : moins de code, et la section critique se limite à des clones d'`Arc`.
- La sauvegarde n'encode rien et ne bloque donc jamais la capture.
- La résolution de sortie est fixe : l'encodeur n'est jamais réinitialisé.

## 4. Types

```rust
struct Packet { ts: i64 /*100 ns QPC*/, dur: i64, keyframe: bool, data: Arc<[u8]> }
struct Ring   { video: VecDeque<Packet>, audio: VecDeque<Packet>, keep: i64 }
struct Clip   { video: Vec<Packet>, audio: Vec<Packet>, video_hdr: Arc<[u8]>, audio_hdr: Arc<[u8]> }
```

- `Ring::snapshot(now, dur)` prend la dernière keyframe dont `ts ≤ now − dur`, puis l'audio à partir de ce `ts`.
- L'éviction garde `dur + 1 GOP` en mémoire.
- **Cadence fixe (CFR) 60 fps**, comme OBS : WGC ne livre une image que si l'écran change, donc une horloge prend la dernière image convertie toutes les 1/60 s et la répète si rien de neuf n'est arrivé. `ts = t0 + n/60`. Le GOP de 60 donne alors exactement une keyframe par seconde. Vérifié au spike A.
- **Sauvegarde** : comme OBS, on note l'instant de l'appui et on attend que l'encodeur ait sorti un paquet de `ts ≥` cet instant avant le snapshot, à cause de la latence de l'encodeur.
- **Zéro trait.** Un `Encoder` ou un `Sink` n'aurait qu'une implémentation et aucun test qui en dépend.

## 5. Recommandations de la communauté retenues

| Sujet | Choix | Source / preuve |
|---|---|---|
| Buffer | Deque de paquets encodés, purge par l'avant jusqu'à une keyframe, garde ≥ 2 keyframes, sauvegarde différée jusqu'à l'instant de l'appui | OBS `replay_buffer_purge` / `replay_buffer_save` ([obs-ffmpeg-mux.c](https://github.com/obsproject/obs-studio/blob/master/plugins/obs-ffmpeg/obs-ffmpeg-mux.c)) |
| Cadence | CFR 60 fps, répétition de la dernière image | Comportement d'OBS ([forum OBS](https://obsproject.com/forum/threads/how-does-obs-work-with-differernt-frame-rates-to-output-a-constant-frame-rate.106156)) ; guides Discord : CFR préféré |
| Encodeur | CBR, GOP 1 s, **B-frames 0**, preset qualité (`QualityVsSpeed` 100), low latency off, profil High | Défauts OBS pour AMF ([wiki OBS AMF](https://github.com/obsproject/obs-studio/wiki/amf-hw-encoder-options-and-information)). Les B-frames à 0 donnent aussi `pts = dts`. |
| Pré-analyse AMF | Off | Elle consomme le moteur graphique du GPU, donc des FPS en jeu (même wiki). Non exposée via ICodecAPI de toute façon. |
| Débit | VBV = 1 s (`BufferSize` = `MaxBitRate` = débit) + **marge de 10 %** dans le calcul | Mesuré au spike A : CBR AMF à +6-7 % avec VBV, +13 % sans |
| Couleurs | NV12 4:2:0, **BT.709 plage limitée**, réglé sur le VideoProcessor et étiqueté dans les types MF | Le VideoProcessor sort du BT.601 par défaut ([doc D3D11](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11videocontext-videoprocessorsetoutputcolorspace)) |
| Mise à l'échelle | VideoProcessor du driver. 3440→1720 est un ratio ×2 exact, donc peu de risque d'artefacts. | À juger à l'œil au spike B |
| MP4 | H.264 + AAC, `moov` avant `mdat` (« faststart ») via `MF_MPEG4SINK_MOOV_BEFORE_MDAT` | Recette web standard ([doc MF](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-mpeg4sink-moov-before-mdat)) ; à vérifier : le SinkWriter transmet-il l'attribut ? |
| Audio | Timestamps du périphérique (`GetBuffer` → position QPC), silence inséré quand la loopback ne livre rien | Équivalent du réglage « Use device timestamps » d'OBS |

## 6. Bonnes pratiques

- **Erreurs** : `anyhow` + `.context()` sur chaque appel COM.
- **Logs** : `%LOCALAPPDATA%\clipper\clipper.log`, tronqué au démarrage ; `panic::set_hook` écrit les panics dans le log.
- **`unsafe`** : seulement dans `mf`, `video`, `audio`, `save` et `RegisterHotKey`. `#![forbid(unsafe_code)]` dans `ring` et `config`, un commentaire `// SAFETY:` par bloc.
- **Console** : `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
- **Retour utilisateur** : `MessageBeep`, un son pour le succès et un autre pour l'échec.
- **Arrêt** : écriture en `.tmp` puis rename, donc un kill est sans danger.
- **Démarrage auto** : un raccourci dans `shell:startup`.
- **Config** : le fichier est à côté de l'exe ; s'il manque, on écrit les valeurs par défaut.
- **Outillage** : `clippy -D warnings`, `rustfmt`, `lto = true`, `panic = "abort"`.

## 7. Tests

**TDD (logique pure)** :
- `ring` : éviction, aucune keyframe assez ancienne, buffer court, buffer vide, audio aligné sur la keyframe.
- `video_bitrate(target_mb, dur, gop, audio_bps, marge = 0.10)` : cas normal, résultat négatif ou trop bas.
- Parsing de la config, y compris le raccourci (`"Ctrl+Alt+F10"`).
- Audio : f32→i16 avec saturation, nombre d'échantillons de silence à insérer.

**Manuel** :
- plein écran exclusif contre borderless ;
- changement de résolution ;
- HDR ;
- veille de l'écran ;
- Win+L ;
- débranchement du casque ;
- 30 s sans aucun son ;
- double appui sur la hotkey ;
- disque plein ;
- jeu lancé en admin ;
- taille réelle du clip contre la cible.

## 8. Itérations et hors scope

Voir [scope.md](scope.md).
