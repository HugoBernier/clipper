# Clipper

Clipper garde en mémoire les 30 dernières secondes de votre écran et de votre son. Un raccourci les enregistre dans un MP4, prêt à partager.

- Encodage par la carte graphique, pas par le processeur.
- Son du PC et micro dans le même clip, synchronisés.
- Durée, résolution, images/s et qualité réglables, avec la taille estimée d'un clip.
- Pensé pour les anti-triches (EAC, BattlEye) : aucune injection, aucun hook clavier.

## Installation

Téléchargez `ClipperSetup-<version>.exe` depuis [Releases](../../releases), puis lancez-le. Pas besoin de droits administrateur.

Au premier lancement, Windows peut afficher un avertissement, car l'installeur n'est pas encore signé : cliquez sur *Informations complémentaires*, puis *Exécuter quand même*.

Il faut Windows 10 ou 11 et une carte graphique AMD, NVIDIA ou Intel récente.

## Utilisation

| Pour… | Faites… |
|---|---|
| Enregistrer les 30 dernières secondes | **Alt+F10** (un son confirme) |
| Revoir, copier, glisser, renommer ou supprimer un clip | Clic sur l'icône près de l'horloge → *Clips* |
| Changer les réglages | Clic sur l'icône → *Réglages* |
| Ouvrir le dossier des clips | `Vidéos\Clipper`, ou bouton *Ouvrir le dossier* |
| Couper le micro, démarrer avec Windows, quitter | Clic droit sur l'icône |

## Sécurité et vie privée

- **Rien ne quitte votre PC.** Le code de Clipper ne contacte aucun serveur, et sa fenêtre ne peut afficher que sa propre page.
- **Aucun clip sans votre raccourci.** Les dernières secondes restent en mémoire et sont remplacées en continu ; seul le raccourci les enregistre.
- **Ce qui est capturé :** l'écran principal, le son du PC et, si activé, le micro. Windows affiche alors en permanence l'icône « micro utilisé ».
- **Ce qui est écrit sur le disque :** vos clips (`Vidéos\Clipper`), les réglages (`clipper.toml`, à côté de l'exe) et un journal technique sans image ni son (`%LOCALAPPDATA%\clipper\clipper.log`).
- **Désinstallation :** depuis *Applications installées*. Elle retire l'app, ses raccourcis et le démarrage auto ; vos clips restent.

Signaler une faille : voir [SECURITY.md](SECURITY.md). Signature des releases : voir [CODE_SIGNING.md](CODE_SIGNING.md).

## Comment c'est fait

Clipper est écrit en Rust avec l'aide de Claude Code (IA). Chaque changement passe par une pull request, relue et testée avant d'être fusionnée.

- **Tests automatiques** sur la logique qui ne dépend pas de Windows : buffer, mixage audio, lecture des MP4, réglages.
- **CI bloquante** à chaque modification : formatage, `clippy` sans aucun avertissement, tests.
- **Code `unsafe` limité** aux fichiers qui appellent Windows, chacun justifié par un commentaire `SAFETY`. Il est interdit dans les modules de logique pure.
- **Peu de dépendances** : `windows`, `wasapi`, `webview2-com` (accès à Windows), `serde`, `toml` (réglages), `anyhow`, `log`, `simplelog` (erreurs et journal) ; `embed-resource` pour l'icône de l'exe.
- **Releases construites par la CI** depuis ce dépôt, jamais à la main.

## Réglages

Tout se règle dans la fenêtre, et chaque changement s'applique immédiatement, sans relancer Clipper.

Les mêmes réglages sont enregistrés dans `clipper.toml`, à côté de l'exe (`%LOCALAPPDATA%\Programs\Clipper`). Seule une modification faite à la main dans ce fichier demande de relancer Clipper.

| Clé | Défaut | Rôle |
|---|---|---|
| `clip_seconds` | `30` | Durée d'un clip (10 à 300 s) |
| `height` | `720` | Hauteur de la vidéo (720, 1080, 1440) ; la largeur suit l'écran |
| `fps` | `60` | Images par seconde (30 ou 60) |
| `quality` | `"medium"` | `low`, `medium`, `high`, `very_high` |
| `hotkey` | `"Alt+F10"` | Raccourci (modificateurs + F1–F24, A–Z ou 0–9) |
| `output_dir` | `"Clipper"` | Dossier des clips, dans Vidéos ou chemin absolu |
| `microphone` | `true` | Ajouter le micro au clip |
| `microphone_volume` | `100` | Volume du micro, en % (0 à 200) |
| `microphone_device` | `""` | Micro choisi ; vide : celui de Windows par défaut |

## Développement

Rust stable (edition 2024), cible `x86_64-pc-windows-msvc`.

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

- Règles du projet : [`CLAUDE.md`](CLAUDE.md)
- Périmètre et avancement : [`docs/scope.md`](docs/scope.md)
- Architecture : [`docs/architecture.md`](docs/architecture.md)
- Apparence de l'interface : [`docs/design-system`](docs/design-system/README.md)

Commits au format [Conventional Commits](https://www.conventionalcommits.org/fr/). [release-please](https://github.com/googleapis/release-please) en déduit la version et le [`CHANGELOG.md`](CHANGELOG.md) ; fusionner sa PR publie la release.

## Licence

[MIT](LICENSE).
