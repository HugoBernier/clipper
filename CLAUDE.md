# Clipper

Replay buffer minimal pour Windows, en Rust : au raccourci, sauvegarde les 30 dernières secondes de jeu (vidéo et son) dans un MP4 envoyable sur Discord.

- Le quoi et le pourquoi, avec la liste YAGNI et l'avancement : [docs/scope.md](docs/scope.md)
- Le comment (crates, modules, threads, types, tests) : [docs/architecture.md](docs/architecture.md)

## Principes (dans cet ordre en cas de conflit)

1. **YAGNI / KISS** : on n'implémente que l'itération en cours de `docs/scope.md`. Rien « au cas où ». Si une idée sort du scope, on la propose sans la coder.
2. **XP** : petites étapes. On fait un spike d'abord sur le plus gros risque, et on écrit le test avant le code pour toute la logique pure.
3. **SOLID, surtout le S** : un module, une responsabilité. **Pas de trait** sauf si un test ou une deuxième implémentation réelle l'exige.
4. **DRY** : on factorise à partir de deux usages réels, pas avant.

En cas de doute, le moins de code gagne.

## Règles de code

- **Toolchain** : Rust stable, edition 2024, cible `x86_64-pc-windows-msvc`.
- **Crates autorisées** : `windows`, `wasapi`, `serde`, `toml`, `anyhow`, `log`, `simplelog` ; en build-dependency, `embed-resource` (icône et version dans l'exe). Toute autre crate se demande d'abord.
- **Erreurs** : `anyhow::Result` partout, et `.context("…")` sur chaque appel COM ou Win32. Pas de `unwrap()` hors tests.
- **`unsafe`** : seulement dans `mf.rs`, `video.rs`, `audio.rs`, `save.rs` et `main.rs` pour la boucle de messages et le bip (`RegisterHotKey`, `GetMessageW`, `MessageBeep`). Chaque bloc porte un commentaire `// SAFETY:`. `ring.rs`, `config.rs` et `mix.rs` ont `#![forbid(unsafe_code)]`. `tray.rs` (icône, menu, registre) peut aussi contenir de l'`unsafe`.
- **Pas de console en release** : `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
- **Anti-cheat** : aucune injection, aucun `SetWindowsHookEx`. On passe uniquement par WGC et `RegisterHotKey`.
- **Horloge** : tous les timestamps sont en unités de 100 ns, dérivés de QPC.
- **Commentaires** : seulement pour le *pourquoi* non évident (contraintes d'API, pièges de Media Foundation).

## Commandes

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Spikes : `cargo run --example <nom>` (fichiers dans `examples/`, jetables, ils ne sont pas réutilisés par `src/`).

## Installeur et version installée

- Construire : `cargo build --release`, puis `"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\clipper.iss` → `target\installer\ClipperSetup-<version>.exe`. La version vient de `Cargo.toml` (via les infos de version de l'exe).
- Installer ou mettre à jour : lancer le setup (il ferme le Clipper en cours). Silencieux : `/VERYSILENT /SUPPRESSMSGBOXES /TASKS=startup`. Ne pas lancer le setup avec `Start-Process -Wait` : il attend aussi Clipper, lancé en fin d'installation.
- Installé par utilisateur dans `%LOCALAPPDATA%\Programs\Clipper` (exe + `clipper.toml`), désinstallable depuis « Applications installées ».
- Démarrage auto : valeur `Clipper` de `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (case du setup ou du menu de l'icône).
- Log : `%LOCALAPPDATA%\clipper\clipper.log` (`clipper-debug.log` pour une build debug). Clips : `Vidéos\Clipper`.
- Icône : maquette `assets/clipper.svg` ; après modification, régénérer `assets/clipper.ico` : `magick -background none -density 384 assets/clipper.svg -define icon:auto-resize=256,64,48,40,32,24,20,16 assets/clipper.ico`.

## Avant de dire « fini »

Lance `cargo fmt`, `cargo clippy -- -D warnings` et `cargo test`, qui doivent tous passer. Vérifie aussi le critère « Fini quand » de l'itération dans `docs/scope.md`. Ce qui n'a pas pu être testé à la main (en jeu, dans Discord) se signale explicitement.
