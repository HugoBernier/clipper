# Clipper

Replay buffer minimal pour Windows, en Rust : au raccourci, sauvegarde les 30 dernières secondes de jeu (vidéo et son) dans un MP4 envoyable sur Discord.

- Le quoi et le pourquoi, avec la liste YAGNI et l'avancement : [docs/scope.md](docs/scope.md)
- Le comment (crates, modules, threads, types, tests) : [docs/architecture.md](docs/architecture.md)
- L'apparence (couleurs, composants, textes de l'interface) : [docs/design-system/README.md](docs/design-system/README.md). Le code suit le design system, pas l'inverse.

## Principes (dans cet ordre en cas de conflit)

1. **YAGNI / KISS** : on n'implémente que l'itération en cours de `docs/scope.md`. Rien « au cas où ». Si une idée sort du scope, on la propose sans la coder.
2. **XP** : petites étapes. On fait un spike d'abord sur le plus gros risque, et on écrit le test avant le code pour toute la logique pure.
3. **SOLID, surtout le S** : un module, une responsabilité. **Pas de trait** sauf si un test ou une deuxième implémentation réelle l'exige.
4. **DRY** : on factorise à partir de deux usages réels, pas avant.

En cas de doute, le moins de code gagne.

## Règles de code

- **Toolchain** : Rust stable, edition 2024, cible `x86_64-pc-windows-msvc`.
- **Crates autorisées** : `windows`, `wasapi`, `serde`, `toml`, `anyhow`, `log`, `simplelog`, `webview2-com` (fenêtre, itérations 9 à 11) ; en build-dependency, `embed-resource` (icône et version dans l'exe). Toute autre crate se demande d'abord.
- **Erreurs** : `anyhow::Result` partout, et `.context("…")` sur chaque appel COM ou Win32. Pas de `unwrap()` hors tests.
- **`unsafe`** : seulement dans `mf.rs`, `video.rs`, `audio.rs`, `save.rs`, `ui.rs` (fenêtre WebView2, sélecteur de dossier), `shell.rs` (presse-papiers, glisser, corbeille, Explorateur) et `main.rs` pour l'initialisation, la fenêtre du raccourci, la boucle de messages, le bip et l'erreur d'arrêt (`OleInitialize`, `SetProcessDpiAwarenessContext`, `CreateWindowExW`, `RegisterHotKey`, `GetMessageW`, `MessageBeep`, `MessageBoxW`). Chaque bloc porte un commentaire `// SAFETY:`. `ring.rs`, `config.rs`, `mix.rs`, `mp4box.rs` et `library.rs` ont `#![forbid(unsafe_code)]`. `tray.rs` (icône, menu, registre) peut aussi contenir de l'`unsafe`.
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

Spikes : `cargo run --example <nom>` (fichiers dans `examples/`, jetables : supprimés une fois leur risque levé, jamais réutilisés par `src/`).

Mesure de synchro A/V : lancer `tools/sync_stimulus.ps1` pendant que Clipper tourne, sauvegarder un clip, puis `cargo run --example sync_check -- <clip.mp4>`.

## Installeur et version installée

- Construire : `cargo build --release`, puis `"%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\clipper.iss` → `target\installer\ClipperSetup-<version>.exe`. La version vient de `Cargo.toml` (via les infos de version de l'exe).
- Installer ou mettre à jour : lancer le setup (il ferme le Clipper en cours). Silencieux : `/VERYSILENT /SUPPRESSMSGBOXES /TASKS=startup`. Ne pas lancer le setup avec `Start-Process -Wait` : il attend aussi Clipper, lancé en fin d'installation.
- Installé par utilisateur dans `%LOCALAPPDATA%\Programs\Clipper` (exe + `clipper.toml`), désinstallable depuis « Applications installées ».
- Démarrage auto : raccourci `Clipper.lnk` dans `shell:startup` + approbation `StartupApproved\StartupFolder` (case du setup ou du menu de l'icône). Pas la clé `Run` : voir `docs/architecture.md`.
- Log : `%LOCALAPPDATA%\clipper\clipper.log` (`clipper-debug.log` pour une build debug). Clips : `Vidéos\Clipper`.
- Icône : maquette `assets/clipper.svg` ; après modification, régénérer `assets/clipper.ico` : `magick -background none -density 384 assets/clipper.svg -define icon:auto-resize=256,64,48,40,32,24,20,16 assets/clipper.ico`.

## Commits et versions

- **Conventional Commits** : `type(portée): description` en français, à l'impératif, sans majuscule ni point final. Types : `feat`, `fix`, `docs`, `refactor`, `test`, `build`, `ci`, `chore`. Portées usuelles : `video`, `audio`, `mix`, `ring`, `save`, `tray`, `config`, `installer`. Changement cassant : `!` après le type et un pied `BREAKING CHANGE:`.
- **SemVer** : la version de `Cargo.toml` est la seule source ; elle remonte dans l'exe et le setup. Ne pas la modifier à la main : release-please la fixe dans sa *Release PR* (avec `CHANGELOG.md`), et fusionner cette PR publie la release, setup compris (`.github/workflows/release.yml`).
- **Branches et PR** : jamais de push direct sur `main`. Une branche par changement, nommée `type/description-courte` avec les mêmes types (`feat/fenetre-reglages`, `docs/fenetre-clipper`), puis une PR vers `main` dont la CI doit passer. Le titre de la PR suit Conventional Commits : fusion en *squash*, c'est lui que release-please lit sur `main`.

## Tester comme un vrai utilisateur

Claude Code tourne dans l'application Claude, empaquetée **MSIX** : tout processus qu'il lance (PowerShell, l'installeur, Clipper) voit et écrit une copie **virtualisée** de `HKCU` et d'`AppData` (`%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\…`). L'Explorateur, le Gestionnaire des tâches et un Clipper lancé normalement voient, eux, les vraies valeurs. Conséquences constatées le 2026-10-08 : une valeur `Run` écrite par Claude Code n'existe pas pour Windows, et le log lu par Claude Code peut être une copie figée.

- Pour lire l'état réel (registre, log), passer par un script lancé avec `explorer.exe <fichier.bat>` qui écrit son résultat **hors d'AppData** (ex. `C:\Users\<nom>\…`).
- Démarrage avec Windows, installation, menu de l'icône : la validation finale se fait par l'utilisateur (setup lancé par double-clic, redémarrage).

## Avant de dire « fini »

Lance `cargo fmt`, `cargo clippy -- -D warnings` et `cargo test`, qui doivent tous passer. Vérifie aussi le critère « Fini quand » de l'itération dans `docs/scope.md`. Ce qui n'a pas pu être testé à la main (en jeu, dans Discord) se signale explicitement.
