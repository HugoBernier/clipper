# Architecture

Légende : **[V]** vérifié en ligne le 2026-10-07 · **[S]** supposé, à confirmer par un spike.

**Limite Discord** : 20 Mo pour les comptes gratuits depuis le 14/08/2026 [V] ([Dexerto](https://www.dexerto.com/entertainment/discord-raises-free-upload-limit-to-20mb-after-lowering-it-two-years-ago-3398251/)). Cible configurable `target_mb = 19`. Pour 30 s, ça donne ~4,5 Mb/s de vidéo, soit du 720p60 confortable ou du 1080p30.

## 1. Crates

| Crate | Version | Rôle | Verdict |
|---|---|---|---|
| `windows` | 0.62.2 [V] | WGC, D3D11, Media Foundation, RegisterHotKey | Indispensable |
| `windows-capture` | 2.0.1 [V] | Capture WGC (`Frame::as_raw_texture()`) | Capture seulement. Son encodeur n'expose pas les paquets et ne capture pas l'audio : inutilisable pour le buffer. |
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
 [thread capture WGC]                [thread audio]
 frame (D3D11, ts QPC)               paquet loopback (f32, ts QPC)
   │ scale+NV12 (GPU)                  │ silences, f32→i16
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
- **Zéro trait.** Un `Encoder` ou un `Sink` n'aurait qu'une implémentation et aucun test qui en dépend.

## 5. Bonnes pratiques

- **Erreurs** : `anyhow` + `.context()` sur chaque appel COM.
- **Logs** : `%LOCALAPPDATA%\clipper\clipper.log`, tronqué au démarrage ; `panic::set_hook` écrit les panics dans le log.
- **`unsafe`** : seulement dans `mf`, `video`, `audio`, `save` et `RegisterHotKey`. `#![forbid(unsafe_code)]` dans `ring` et `config`, un commentaire `// SAFETY:` par bloc.
- **Console** : `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
- **Retour utilisateur** : `MessageBeep`, un son pour le succès et un autre pour l'échec.
- **Arrêt** : écriture en `.tmp` puis rename, donc un kill est sans danger.
- **Démarrage auto** : un raccourci dans `shell:startup`.
- **Config** : le fichier est à côté de l'exe ; s'il manque, on écrit les valeurs par défaut.
- **Outillage** : `clippy -D warnings`, `rustfmt`, `lto = true`, `panic = "abort"`.

## 6. Tests

**TDD (logique pure)** :
- `ring` : éviction, aucune keyframe assez ancienne, buffer court, buffer vide, audio aligné sur la keyframe.
- `video_bitrate` : cas normal, résultat négatif ou trop bas.
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

## 7. Itérations et hors scope

Voir [scope.md](scope.md).
