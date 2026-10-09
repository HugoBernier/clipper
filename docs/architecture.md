# Architecture

Légende : **[V]** vérifié en ligne le 2026-10-07 · **[S]** supposé, à confirmer par un spike.

**Débit** (depuis l'itération 9) : fixé par le niveau de qualité, pas par une taille cible. `config::video_bitrate` = bits par pixel du niveau × largeur × hauteur × 60 × (fps / 60)^0,6. L'exposant suit les débits conseillés par YouTube (1440p60 ≈ 1,5 × 1440p30). « Moyenne » en 1720×720 à 60 i/s redonne les 4,25 Mb/s de la 0.1, qui visait 19 Mo pour 30 s (limite Discord gratuite : 20 Mo [V], [Dexerto](https://www.dexerto.com/entertainment/discord-raises-free-upload-limit-to-20mb-after-lowering-it-two-years-ago-3398251/)). La taille estimée d'un clip, affichée dans la fenêtre, ajoute 10 % de dépassement de l'encodeur et 1 s de GOP.

**Taille de sortie** : hauteur choisie (720 par défaut), largeur au ratio de l'écran, arrondie au pair (écran 3440×1440 → 1720×720). On n'étire jamais l'image.

## 1. Crates

| Crate | Version | Rôle | Verdict |
|---|---|---|---|
| `windows` | 0.62.2 [V] | WGC, D3D11, Media Foundation, RegisterHotKey | Indispensable |
| ~~`windows-capture`~~ | 2.0.1 [V] | Capture WGC | **Écarté (spike A)** : il crée son propre device D3D11 sans `VIDEO_SUPPORT` ni protection multithread, et on ne peut pas lui en fournir un. WGC est appelé directement via `windows` (~40 lignes). |
| `wasapi` | 0.25.0 [V] | Loopback | Prendre. La position QPC est [S] |
| `serde` + `toml` | toml 1.1.6 [V] | Config | Prendre |
| `anyhow` | 1.0.104 [V] | Erreurs | Prendre |
| `log` + `simplelog` | 0.12.2 [V] | Log dans un fichier | Prendre |
| `webview2-com` | 0.39.1 [V] | Fenêtre : liaison officielle de WebView2 (moteur d'Edge présent dans Windows) ; loader lié en statique, aucune DLL à livrer | Prendre (itération 9) |
| ~~`mp4`~~ | 0.14.0, figé depuis 2023 [V] | Muxer | Inutile : le passthrough IMFSinkWriter fonctionne (spike B) |

Encodage : MFT Media Foundation appelés directement (H.264 AMF matériel + AAC Microsoft).

## 2. Arborescence

```
Cargo.toml
clipper.toml.example
src/
  main.rs    # windows_subsystem, logger, boucle de messages, App : réglages appliqués à chaud
  config.rs  # Config (serde, défauts, validation, set/pairs) + débit et taille estimée (pur)
  ring.rs    # Packet, Ring, éviction, snapshot coupé à la keyframe (pur, TDD)
  mf.rs      # helpers MF partagés : MFStartup, media types, MFT async
  video.rs   # WGC → VideoProcessor (scale + NV12, GPU) → MFT H.264
  audio.rs   # son du PC (loopback) + micro WASAPI → timelines → mixeur → MFT AAC
  mix.rs     # mixage pur de sources PCM alignées sur la même origine (TDD)
  tray.rs    # icône de notification, menu, démarrage avec Windows (dossier Démarrage)
  ui.rs      # fenêtre Win32 + WebView2 ; ui.html : la page (réglages)
build.rs     # compile assets/clipper.rc (icône + version) dans l'exe
assets/      # clipper.svg (maquette d'icône), clipper.ico, clipper.rc
installer/   # clipper.iss (Inno Setup)
  save.rs    # snapshot → SinkWriter passthrough → .tmp → faststart (pur) → rename
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
- **Fenêtre principale** invisible (message-only) : elle reçoit le raccourci (`RegisterHotKey` lié à elle) et les demandes de l'icône et de la page (`WM_MICROPHONE`, `WM_UI`). Des messages de thread seraient jetés par les boucles modales (menu de l'icône, sélecteur de dossier), qui distribuent en revanche ceux des fenêtres : un appui pendant qu'un menu est ouvert n'est pas perdu. L'état du thread principal (`App`) est atteint depuis sa procédure (`thread_local` + `try_borrow_mut`) ; aucune boucle modale ne s'ouvre pendant qu'il est emprunté. Pendant la capture d'un nouveau raccourci, l'actuel est désenregistré (sinon Windows le prendrait avant la page) et remis à la demande suivante ou à la fermeture.
- La résolution de sortie ne suit pas l'écran : l'encodeur n'est réinitialisé que si l'utilisateur change la résolution, les fps ou la qualité (buffer vidé, audio ininterrompu).
- **Fenêtre** (itération 9) : sur le thread principal. Le moteur WebView2 est créé de façon asynchrone à l'ouverture (pas de boucle de messages imbriquée : un appui sur le raccourci n'est pas perdu) et détruit à la fermeture. La page envoie des lignes texte (`get`, `set\nclé=valeur`, `pick_folder`, `startup\ntrue`), mises en file puis signalées par `WM_UI` ; `App` les applique et répond par l'état complet (`state` puis `clé=valeur`). Pas de JSON : `serde_json` n'est pas une crate autorisée, et ce format suffit. Toute navigation après la page initiale est annulée (un fichier déposé sur la fenêtre ne remplace pas la page qui pilote Clipper).

## 4. Types

```rust
struct Packet { ts: i64 /*100 ns QPC*/, key: bool, data: Arc<[u8]> }
struct Ring   { packets: VecDeque<Packet>, keep: i64 }  // vidéo ; l'audio arrive à l'itération 2
struct VideoFormat { width, height, fps, bitrate }       // mf.rs, commun encodeur et MP4
```

- Pas de `dur` par paquet (CFR : `1/fps`) ni d'en-tête stocké : SPS/PPS sont répétés dans chaque IDR (AMF : AUD, SPS, PPS, IDR) et `save` les extrait de la première keyframe.
- `Ring::snapshot(end, dur)` prend la dernière keyframe dont `ts ≤ end − dur`, puis l'audio à partir de ce `ts`.
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
| MP4 | H.264 + AAC, `moov` avant `mdat` (« faststart ») par **réécriture façon `qt-faststart`** : on déplace `moov` et on décale les offsets `stco`/`co64` de sa taille. Fonction pure, testée en TDD dans `save.rs`. | Recette web standard. `MF_MPEG4SINK_MOOV_BEFORE_MDAT` est **écarté** : sa recopie décale le `mdat` d'1 Mio et rend le fichier illisible (spike B). |
| Audio | Timestamps du périphérique (`GetBuffer` → position QPC), silence inséré quand la loopback ne livre rien. `autoconvert` de WASAPI : PCM 16 bits 48 kHz stéréo livré par Windows, sans conversion à coder. | Équivalent du réglage « Use device timestamps » d'OBS. Vérifié : les ts WASAPI sont l'instant de rendu (0 à 10 ms dans le futur). |

### Pièges Media Foundation rencontrés

- SinkWriter vers un `.tmp` : préciser `MF_TRANSCODE_CONTAINERTYPE = MPEG4`, sinon le conteneur est déduit de l'extension (`MF_E_NOT_FOUND`).
- Passthrough H.264 : même type en entrée et en sortie, avec `MF_MT_MPEG_SEQUENCE_HEADER` = SPS + PPS (start codes) extraits de la première keyframe. `MFSampleExtension_CleanPoint` sur les keyframes.
- Les ts sont ramenés à 0 sur la keyframe de coupe.
- Lecture en RGB32 : les lignes sont alignées, donc le pas réel est `taille / hauteur`, pas `largeur × 4`.

## 6. Bonnes pratiques

- **Erreurs** : `anyhow` + `.context()` sur chaque appel COM.
- **Logs** : `%LOCALAPPDATA%\clipper\clipper.log`, recréé à chaque démarrage, en heure locale ; `panic::set_hook` y écrit les panics. En debug, aussi sur la sortie d'erreur. `CLIPPER_DEBUG=1` : chaque paquet audio (ts, retard, placement).
- **`unsafe`** : seulement dans `mf`, `video`, `audio`, `save` et `RegisterHotKey`. `#![forbid(unsafe_code)]` dans `ring` et `config`, un commentaire `// SAFETY:` par bloc.
- **Console** : `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
- **Retour utilisateur** : `MessageBeep`, un son pour le succès et un autre pour l'échec.
- **Arrêt** : écriture en `.tmp` puis rename, donc un kill est sans danger.
- **Démarrage auto** : un raccourci dans `shell:startup`.
- **Config** : le fichier est à côté de l'exe ; s'il manque, on écrit les valeurs par défaut.
- **Démarrage avec Windows** : raccourci `Clipper.lnk` dans le dossier Démarrage (`shell:startup`) + `Explorer\StartupApproved\StartupFolder\Clipper.lnk` = `02…`. Méthode documentée par Microsoft pour les applications de bureau, comme Telegram et Ollama ; validée au redémarrage le 2026-10-08. La clé `Run` (méthode d'Electron) aurait aussi convenu : ses échecs apparents venaient de l'environnement de test (voir `CLAUDE.md`, « Tester comme un vrai utilisateur »), pas de Windows. Une seule instance à la fois (mutex nommé `Local\Clipper`).
- **Changement de qualité** : `video::Video` (format + drapeau d'arrêt + thread) ; `stop()` puis `spawn()` au nouveau format, `Ring::clear()` entre les deux. Le menu de l'icône poste `WM_QUALITY` au thread principal, propriétaire du pipeline. **Libérer un pipeline** : un MFT asynchrone exige `IMFShutdown::Shutdown` (et `IMFActivate::ShutdownObject`), et le gestionnaire `FrameArrived` de WGC doit être retiré (`RemoveFrameArrived`) ; sans ça, ~240 Mo et ~1 700 handles fuyaient à chaque changement.
- **Clips** : `output_dir` est relatif au dossier Vidéos de Windows (`SHGetKnownFolderPath`, suit un dossier déplacé vers OneDrive) ; défaut `Vidéos\Clipper`.
- **Changement de résolution de la source** : le pool WGC est recréé à la nouvelle taille et le VideoProcessor aussi ; la sortie garde sa taille, l'image est centrée sans déformation (bandes noires), donc l'encodeur n'est jamais réinitialisé.
- **Outillage** : `clippy -D warnings`, `rustfmt`, `lto = true`, `panic = "abort"`.

## 7. Tests

**TDD (logique pure)** :
- `ring` : éviction, aucune keyframe assez ancienne, buffer court, buffer vide, audio aligné sur la keyframe.
- `faststart` : moov déjà devant (inchangé), moov après mdat (déplacé, offsets `stco` décalés), `co64`, boîte 64 bits, débordement 32 bits → erreur, fichier tronqué → erreur.
- `sequence_header` : extrait SPS + PPS d'une unité d'accès (start codes sur 3 et 4 octets).
- `video_bitrate(qualité, largeur, hauteur, fps)` : débit de la 0.1 retrouvé, ordre des niveaux, proportionnel aux pixels, fps × 2 ≈ débit × 1,5, plancher ; `estimated_mb`.
- `Config::set` / `pairs` : chaque réglage, refus sans effet, aller-retour ; relecture d'un `clipper.toml` 0.1 (`target_mb` ignoré puis retiré).
- `mix::apply_volume` : 0 %, 100 %, 50 %, 200 % saturé ; `Ring::set_keep`.
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
