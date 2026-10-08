# Scope

## Le besoin

Au raccourci, un MP4 des **30 dernières secondes** (image + son du jeu), qui s'envoie tel quel sur Discord. Rien n'est affiché, rien de tiers n'est installé.

## Contraintes

| Contrainte | Valeur |
|---|---|
| OS | Windows 11 |
| GPU | AMD RX 6800 XT, driver seul (pas d'Adrenalin) |
| Taille du clip | ≤ 19 Mo pour 30 s (limite Discord gratuite : 20 Mo) |
| Codec | H.264 matériel (AMF) + AAC, conteneur MP4 |
| Anti-cheat | EAC (Hunt: Showdown), BattlEye (R6 Siege) : aucune injection, aucun hook clavier bas niveau |
| Démarrage | automatique avec Windows, sans fenêtre |

## Dans le MVP

- Capture de l'écran principal (Windows Graphics Capture).
- Son système en loopback (WASAPI).
- Buffer circulaire de paquets encodés en mémoire.
- Raccourci global (`RegisterHotKey`) qui écrit le MP4.
- Fichier de config TOML à côté de l'exe.
- Log dans un fichier, et un bip pour signaler le succès ou l'échec.

## Hors MVP (YAGNI) — ne pas implémenter sans demande explicite

- commande vocale ;
- upload ;
- notifications Windows ;
- capture d'une fenêtre ou de plusieurs écrans ;
- HDR ;
- HEVC/AV1 ;
- fallback logiciel ;
- rotation des logs ;
- hotkey « quitter » ;
- ffmpeg ;
- plusieurs durées de clip.

## Définition de « réussi »

Sur 2 semaines de jeu réel, **zéro appui sur le raccourci sans clip correct**. Un clip correct :
- commence sur une keyframe ;
- dure 30 à 31 s ;
- pèse moins de 19 Mo ;
- a un son synchronisé à moins d'une frame près ;
- se lit dans Discord.

D'ici là, OBS reste l'outil du quotidien.

## Itérations

| # | Étape | Fini quand |
|---|---|---|
| A | Spike : WGC → MFT H.264 AMF → `.h264` brut 10 s | Lisible dans VLC, CPU < 5 %, une keyframe par seconde. **Fait le 2026-10-07** : `AMDh264Encoder`, 1720×720, CFR 60 fps (600 images en 10 s), keyframes à 0,0 / 1,0 … 9,0 s, 4,8 Mb/s pour une cible de 4,5 (+6-7 %), CPU 1,2 % sur le bureau. Lecture validée au spike B. CPU en jeu : à confirmer pendant l'itération 3. |
| B | Spike : paquets → MP4 via SinkWriter passthrough | Le MP4 se lit dans Discord (sinon, plan B : crate `mp4`). **Fait le 2026-10-07** : coupe à la keyframe de 2 s, ts ramenés à 0, 480 images décodées par Windows, BT.709 relu, faststart maison OK, image contrôlée visuellement. Lecture dans Discord confirmée le 2026-10-08. |
| 1 | Ring + hotkey + config, vidéo seule | 1 h stable en mémoire, clip de 30 à 31 s sous la cible. **Code fait le 2026-10-07** : 24 tests, clip de 30,62 s, 15,2 Mo (vidéo seule, cible 19), faststart OK, ~98 Mo de RAM. Test d'endurance : stable sur 39 min (87-93 Mo privés, handles constants, ~0,75 % CPU), interrompu par la fin de session ; la soirée de l'itération 3 couvre la suite. Discord OK. |
| 2 | Audio + AAC + synchro QPC | Décalage < 1 frame, pas de dérive après un silence. **Mesuré le 2026-10-07** (stimulus flash + bip, instants QPC) : le son est placé à l'instant QPC où Windows le joue, l'image 16 à 32 ms après l'événement à l'écran (≤ ~1 image propre à Clipper). Silences comblés exactement, sans dérive. **Validé en jeu le 2026-10-07** : environ 1 image de retard, invisible. |
| 3 | Prod : sous-système Windows, log, bips, démarrage auto | Survit à un reboot et à une soirée de Hunt et R6. **Code fait le 2026-10-08** : pas de console, log fichier en heure locale, bips, clips dans Vidéos\Clipper, changement de résolution géré (bandes noires), double appui ignoré (testé), 37 tests. Installé avec démarrage auto le 2026-10-08 ; clips avec son validés par l'utilisateur. **Redémarrage validé le 2026-10-08** (raccourci du dossier Démarrage, visible et activé dans le Gestionnaire des tâches ; case du menu testée). **Reste à faire** : soirée de jeu et cas limites en jeu. |
| 4 | Micro mixé au son du PC (voir ci-dessous) | Voix audible et synchro (< 1 image) avec le jeu ; micro absent ou débranché → clip quand même, avec le son du PC ; pas de dérive sur 1 h. **Code fait le 2026-10-08** : micro de communication capté (Scarlett Solo, paquets de 10 ms, ~8 ms de latence, aucune correction), mixeur pur (6 tests), clip mixé vidéo/audio alignés, 45 tests. **Reste à faire** : voix + tir en jeu, débrancher/rebrancher le micro, 1 h. **Corrigé le 2026-10-08** : voix seulement à gauche (la Scarlett Solo expose 2 canaux : micro à gauche, entrée instrument à droite) → micro converti en mono sur les deux canaux, comme le « Downmix to Mono » d'OBS ; la voix perd ~6 dB. |
| 5 | Icône dans la zone de notification (voir ci-dessous) | Icône visible près de l'horloge ; menu : ouvrir le dossier des clips, « Démarrer avec Windows » (coché selon l'état réel), quitter ; l'icône revient si l'Explorateur redémarre. **Fait le 2026-10-08** : icône, info-bulle, menu testés par l'utilisateur ; aucun bouton dans la barre des tâches ; 48 tests. |
| 6 | Installeur Windows (voir ci-dessous) | `ClipperSetup.exe` installe sans droits admin, crée l'entrée du menu Démarrer, propose le démarrage avec Windows et lance Clipper ; Clipper apparaît dans « Applications installées » avec son icône ; la désinstallation retire exe, raccourcis et démarrage auto, garde les clips ; réinstaller par-dessus une version qui tourne fonctionne. **Fait le 2026-10-08** : setup de 2,6 Mo ; testé en silencieux : installation par-dessus le Clipper en cours (fermé puis relancé), entrée « Clipper 0.1.0 » avec icône, menu Démarrer, démarrage auto ; désinstallation propre, clips conservés. Icône et version compilées dans l'exe (maquette SVG). **Reste à faire** : l'assistant graphique vu par l'utilisateur, le design définitif de l'icône. |
| 7 | Processus de release (voir ci-dessous) | Conventional Commits appliqués ; CI verte sur `main` ; une Release PR release-please propose la version SemVer et le `CHANGELOG.md` ; sa fusion publie une release GitHub avec le setup construit par la CI **Fait le 2026-10-08** : dépôt privé `HugoBernier/clipper`, CI verte, release `v0.1.1` publiée par release-please avec `ClipperSetup-0.1.1.exe`. |
| 8 | Qualité réglable facilement (voir ci-dessous) | Choisir un préréglage de qualité depuis le menu de l'icône, appliqué sans relancer Clipper ; le clip suivant respecte la résolution, les fps et la taille cible du préréglage **Fait le 2026-10-08** : sous-menu « Qualité » (3 préréglages, l'actif coché), bascule à chaud testée (720p60 ↔ 1440p60, clip suivant en 3440×1440, 34 Mo sous 48), audio ininterrompu. Mémoire stable sur 30 allers-retours (92 → 104 Mo) après correction d'une fuite (`IMFShutdown` de l'encodeur async, désabonnement WGC) ; reste ~3 handles par changement, attribués au pilote. 55 tests. |
| 9 | Interface graphique (voir ci-dessous) | Une fenêtre claire, ouverte depuis l'icône : réglages (qualité, micro, raccourci, dossier, démarrage) et derniers clips à partager ; design fourni par Claude Design |

## Itération 4 : micro

**Pourquoi** : entendre sa propre voix dans les clips (réactions, callouts). Aujourd'hui seul le son du PC est capturé ; la voix des potes en vocal y est, la sienne non.

**Ce qui est dans l'itération**
- Capture du micro par défaut de Windows (WASAPI, `autoconvert` → 48 kHz 16 bits stéréo, comme la loopback).
- **Mixage dans une seule piste AAC** : Discord ne lit que la première piste audio d'un MP4.
- Chaque source garde sa `Timeline` (réutilisée : deux usages réels, factorisation justifiée) ; un **mixeur pur** aligne les deux flux par index d'échantillon sur l'horloge QPC, additionne avec saturation et ne livre à l'encodeur que la partie couverte par les deux sources (ou comblée de silence après `LAG`).
- Micro absent, refusé ou débranché : on continue avec le son du PC seul (silence côté micro), et on réessaie toutes les secondes comme pour la loopback.

**Tests (TDD, mixeur)** : deux flux alignés, un flux en retard, une source muette ou absente, saturation à ±32767, pas de dérive d'index sur 1 h simulée.

**Validation manuelle** : parler en tirant (voix et son du jeu synchro), débrancher puis rebrancher le micro, 1 h sans dérive.

**Conséquences à connaître**
- Windows affichera en permanence l'icône « micro utilisé » dans la barre des tâches. Rien n'est écrit sur disque sans appui sur le raccourci : les 30 s restent en mémoire.
- Avec des enceintes (sans casque), le micro capte aussi le son du jeu : écho léger dans le clip. Avec un casque, pas de souci.

**Décisions (2026-10-08)**
1. Micro activé par défaut ; `microphone = false` dans `clipper.toml` pour le couper.
2. Pas de réglage de volume pour l'instant : on juge sur les vrais clips.
3. Micro de communication par défaut (celui de Discord).

## Itération 5 : icône dans la zone de notification

**Pourquoi** : sans fenêtre ni icône, Clipper est invisible et ne se quitte que par le Gestionnaire des tâches. Un outil Windows d'arrière-plan vit près de l'horloge (comme Discord, Medal, ShadowPlay).

**Ce qui est dans l'itération**
- Icône de l'application (ressource de l'exe, itération 6) et info-bulle « Clipper — Alt+F10 ».
- Menu au clic droit : **Ouvrir le dossier des clips**, **Démarrer avec Windows** (case à cocher), **Quitter**.
- Démarrage avec Windows par un raccourci dans le dossier Démarrage + son approbation `StartupApproved\StartupFolder` (méthode documentée par Microsoft ; voir `docs/architecture.md`).
- Quitter attend la fin d'une sauvegarde en cours.
- Une fenêtre cachée reçoit les messages de l'icône ; elle n'apparaît ni à l'écran ni dans la barre des tâches.

**Hors itération** : icône de l'exe dans l'Explorateur (demande une ressource compilée, donc une crate ou un script de build), notification Windows à chaque clip (le bip suffit).

## Itération 6 : installeur Windows

**Pourquoi** : installer, mettre à jour et désinstaller Clipper comme n'importe quel logiciel, sans copier d'exe à la main.

**Ce qui est dans l'itération**
- **Inno Setup** (gratuit, y compris en commercial) : script `installer/clipper.iss`, sortie `target/installer/ClipperSetup-<version>.exe`.
- Installation **par utilisateur**, sans droits admin, dans `%LOCALAPPDATA%\Programs\Clipper` (comme Discord, VS Code) : la config reste modifiable à côté de l'exe.
- Menu Démarrer, case « Démarrer avec Windows » (même raccourci du dossier Démarrage que le menu de l'icône), lancement en fin d'installation, désinstalleur dans « Applications installées ».
- Une mise à jour ferme le Clipper en cours avant de remplacer l'exe.
- **Icône de l'application** : maquette SVG (`assets/clipper.svg`, à remplacer par un design définitif), convertie en `.ico` multi-tailles et compilée dans l'exe (crate de build `embed-resource`). L'icône de notification utilise la même ressource. Informations de version (nom, version) dans l'exe.

**Hors itération** : signature de code (certificat payant ; sans lui, SmartScreen avertit au premier lancement du setup), mise à jour automatique, installation pour tous les utilisateurs.

## Itération 7 : processus de release

**Pourquoi** : qualité de travail. Historique lisible, versions qui ont un sens, releases reproductibles.

**Ce qui est dans l'itération** (outils standard de la communauté)
- **Conventional Commits** (règle dans `CLAUDE.md` depuis le 2026-10-08).
- **SemVer** : la version de `Cargo.toml` est la seule source (elle remonte dans l'exe et le setup). `0.x` tant que le format de config peut changer ; `0.1.0` = l'état du 2026-10-08 avant Conventional Commits (tag `v0.1.0`).
- **[release-please](https://github.com/googleapis/release-please)** (GitHub Action, type `rust`) : à chaque push sur `main`, une *Release PR* fixe la version suivante dans `Cargo.toml`/`Cargo.lock` et génère `CHANGELOG.md` (format *Keep a Changelog*) depuis les commits. La fusionner crée le tag `vX.Y.Z` et la release GitHub. Les commits antérieurs à `v0.1.0` sont ignorés (`bootstrap-sha`).
- **CI** (`.github/workflows/ci.yml`, runner Windows) : `fmt --check`, `clippy -D warnings`, tests, à chaque push et PR.
- **Release** (`.github/workflows/release.yml`) : quand une release est créée, compilation, tests, setup Inno Setup joint à la release.
- Dépôt GitHub **privé** `clipper` ; `README.md` à la racine.

**Décisions (2026-10-08)** : dépôt `clipper` privé ; changelog généré (release-please plutôt que git-cliff : rien à installer en local, version et changelog dans la même PR).

## Itération 8 : qualité réglable facilement

**Pourquoi** : la qualité (résolution, fps, taille cible) doit se changer sans éditer `clipper.toml` ni relancer Clipper.

**Ce qui est dans l'itération**
- **Préréglages** (`config::PRESETS`) : un préréglage n'est qu'un trio hauteur / fps / taille cible, écrit tel quel dans `clipper.toml` (pas de nouveau champ ; une config modifiée à la main ne correspond à aucun préréglage et rien n'est coché). « Discord gratuit » (720p60, 19 Mo), « Discord gratuit, plus net » (1080p30, 19 Mo), « Discord Nitro Basic » (1440p60, 48 Mo).
- **Sous-menu « Qualité »** dans le menu de l'icône, préréglage actif coché.
- **Application à chaud** : le thread principal arrête le pipeline vidéo (drapeau vérifié à chaque image), vide le buffer, le relance au nouveau format ; l'audio continue.

## Itération 9 : interface graphique

**Pourquoi** : un vrai logiciel, réglable et utilisable sans fichier de config ; partager un clip en un geste.

**Ce qui est dans l'itération** (contenu exact selon la maquette Claude Design)
- Fenêtre ouverte depuis l'icône (clic gauche ou menu) : réglages (qualité, micro et son volume, raccourci, dossier des clips, démarrage avec Windows).
- Liste des derniers clips : lire, ouvrir le dossier, partager (copier le fichier pour le coller dans Discord, glisser-déposer).
- Les réglages s'appliquent sans relancer Clipper (s'appuie sur l'itération 8).

**Questions ouvertes**
1. Technologie : la maquette Claude Design sera en HTML/CSS ; l'afficher dans une WebView2 (le moteur Edge intégré à Windows, crate à valider) permet de la reprendre presque telle quelle. Une interface native Win32 obligerait à la redessiner. À trancher quand la maquette existe.
2. Que veut dire « partager » exactement : copier le fichier, copier un lien (demanderait un upload, hors scope), glisser vers Discord ?

Sortie de secours : si le spike A dépasse 3 soirées, on passe l'encodage et le mux à ffmpeg, et on garde WGC et WASAPI en natif.
