# Clipper

Replay buffer minimal pour Windows : Clipper garde en mémoire les 30 dernières secondes de l'écran et du son, et les enregistre dans un MP4 prêt pour Discord quand on appuie sur un raccourci.

- Encodage matériel H.264 (GPU), AAC pour le son du PC et le micro, synchronisés sur la même horloge.
- Clip sous la limite d'upload de Discord (19 Mo par défaut).
- Sans injection ni hook clavier : compatible avec les anti-cheats (EAC, BattlEye).
- Icône près de l'horloge, démarrage avec Windows, installeur sans droits admin.

## Installation

Télécharger `ClipperSetup-<version>.exe` depuis la page [Releases](../../releases) et le lancer. Windows SmartScreen peut afficher un avertissement (l'exe n'est pas signé) : *Informations complémentaires* → *Exécuter quand même*.

Prérequis : Windows 10 1903 ou plus récent, GPU avec encodeur H.264 matériel (AMD, NVIDIA ou Intel).

## Utilisation

| Action | Comment |
|---|---|
| Enregistrer les 30 dernières secondes | **Alt+F10** (un son confirme) |
| Retrouver les clips | `Vidéos\Clipper`, ou clic droit sur l'icône → *Ouvrir le dossier des clips* |
| Démarrer avec Windows | Clic droit sur l'icône → *Démarrer avec Windows* |
| Quitter | Clic droit sur l'icône → *Quitter* |

## Réglages

Fichier `clipper.toml`, à côté de l'exe (`%LOCALAPPDATA%\Programs\Clipper`), créé au premier lancement. Relancer Clipper après une modification.

| Clé | Défaut | Rôle |
|---|---|---|
| `clip_seconds` | `30` | Durée d'un clip, en secondes |
| `height` | `720` | Hauteur de sortie ; la largeur suit le ratio de l'écran |
| `fps` | `60` | Images par seconde |
| `target_mb` | `19.0` | Taille maximale d'un clip, en Mo |
| `hotkey` | `"Alt+F10"` | Raccourci (ex. `"Ctrl+Shift+S"`, touches F1–F24, A–Z, 0–9) |
| `output_dir` | `"Clipper"` | Dossier des clips, relatif au dossier Vidéos ou absolu |
| `microphone` | `true` | Mixer le micro de communication au son du PC |

Log : `%LOCALAPPDATA%\clipper\clipper.log`.

## Développement

Rust stable (edition 2024), cible `x86_64-pc-windows-msvc`.

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

- Règles du projet (principes, crates autorisées, commits) : [`CLAUDE.md`](CLAUDE.md)
- Périmètre, itérations et avancement : [`docs/scope.md`](docs/scope.md)
- Architecture et choix techniques : [`docs/architecture.md`](docs/architecture.md)

### Commits et releases

Commits au format [Conventional Commits](https://www.conventionalcommits.org/fr/) (`feat(audio): …`, `fix(tray): …`). À chaque push sur `main`, [release-please](https://github.com/googleapis/release-please) tient à jour une *Release PR* qui fixe la prochaine version [SemVer](https://semver.org/lang/fr/) et le [`CHANGELOG.md`](CHANGELOG.md). La fusionner publie la release ; la CI y attache l'installeur.
