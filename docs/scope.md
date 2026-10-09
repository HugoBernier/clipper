# Scope

## Le besoin

Au raccourci, un MP4 des **30 dernières secondes** (image + son du jeu), qui s'envoie tel quel sur Discord. Rien n'est affiché, rien de tiers n'est installé.

Depuis l'itération 9 : la durée, la résolution, les fps et la qualité sont des choix de l'utilisateur. L'app sert les clips, elle ne raisonne plus en limites Discord ; elle affiche la taille estimée et laisse l'utilisateur décider. Une fenêtre (ouverte depuis l'icône) sert à régler Clipper et à revoir et partager les clips. En jeu, rien ne change : pas de fenêtre, la même empreinte.

## Contraintes

| Contrainte | Valeur |
|---|---|
| OS | Windows 11 |
| GPU | AMD RX 6800 XT, driver seul (pas d'Adrenalin) |
| Taille du clip | MVP : ≤ 19 Mo pour 30 s (limite Discord gratuite : 20 Mo). Depuis l'itération 9 : choisie par l'utilisateur (qualité × durée), estimée à l'écran |
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
- dans le lecteur : découpe (trim), vitesse de lecture, image par image ;
- compresseur vidéo (réencoder un clip pour qu'il tienne sous une taille) ;
- plafond de taille en Mo (la qualité s'adapterait à la durée) ;
- volume du son du PC (seul le micro est réglable) ;
- déplacer les anciens clips quand on change de dossier ;
- préréglages de qualité (retirés à l'itération 9) ;
- thème clair, choix du thème.

Les lignes « dans le lecteur… » à « déplacer les anciens clips… » ont été proposées le 2026-10-09 : à demander quand le besoin se présente. Les vignettes, d'abord ici, entrent à l'itération 11b (design system validé le 2026-10-09).

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
| 6 | Installeur Windows (voir ci-dessous) | `ClipperSetup.exe` installe sans droits admin, crée l'entrée du menu Démarrer, propose le démarrage avec Windows et lance Clipper ; Clipper apparaît dans « Applications installées » avec son icône ; la désinstallation retire exe, raccourcis et démarrage auto, garde les clips ; réinstaller par-dessus une version qui tourne fonctionne. **Fait le 2026-10-08** : setup de 2,6 Mo ; testé en silencieux : installation par-dessus le Clipper en cours (fermé puis relancé), entrée « Clipper 0.1.0 » avec icône, menu Démarrer, démarrage auto ; désinstallation propre, clips conservés. Icône et version compilées dans l'exe (maquette SVG). **Reste à faire** : l'assistant graphique vu par l'utilisateur. Icône définitive le 2026-10-09 (anneau, flèche « replay », point rouge ; version simplifiée pour 16 à 24 px). |
| 7 | Processus de release (voir ci-dessous) | Conventional Commits appliqués ; CI verte sur `main` ; une Release PR release-please propose la version SemVer et le `CHANGELOG.md` ; sa fusion publie une release GitHub avec le setup construit par la CI **Fait le 2026-10-08** : dépôt privé `HugoBernier/clipper`, CI verte, release `v0.1.1` publiée par release-please avec `ClipperSetup-0.1.1.exe`. |
| 8 | Qualité réglable facilement (voir ci-dessous) | Choisir un préréglage de qualité depuis le menu de l'icône, appliqué sans relancer Clipper ; le clip suivant respecte la résolution, les fps et la taille cible du préréglage **Fait le 2026-10-08** : sous-menu « Qualité » (3 préréglages, l'actif coché), bascule à chaud testée (720p60 ↔ 1440p60, clip suivant en 3440×1440, 34 Mo sous 48), audio ininterrompu. Mémoire stable sur 30 allers-retours (92 → 104 Mo) après correction d'une fuite (`IMFShutdown` de l'encodeur async, désabonnement WGC) ; reste ~3 handles par changement, attribués au pilote. 55 tests. |
| C | Spike : fenêtre WebView2 (voir itération 9) | Une fenêtre Win32 affiche une page HTML locale et lit un clip dans un `<video>` ; un bouton HTML déclenche du code Rust et reçoit une réponse ; fermer la fenêtre libère le moteur (RAM revenue au niveau d'avant) ; un glisser depuis la page dépose le fichier dans Discord. **Code écrit le 2026-10-09** (`examples/webview.rs`, compilé pour Windows, loader WebView2 lié en statique : pas de DLL à livrer). **Reste à faire** : la validation sur le PC. |
| 9 | Réglages dans une fenêtre (voir ci-dessous) | Depuis la fenêtre, sans relancer Clipper : résolution, fps, qualité (avec taille estimée), durée, volume du micro, dossier, raccourci, démarrage avec Windows ; le clip suivant respecte chaque réglage ; un ancien `clipper.toml` est relu sans erreur. Visuel brut : le design system viendra ensuite. **Code écrit le 2026-10-09** : fenêtre `ui.rs` + `ui.html` (clic sur l'icône), débit par niveau de qualité, taille estimée, volume et coupure du micro à chaud, raccourci et dossier changés à chaud (l'ancien raccourci est remis si le nouveau est pris), case « Micro » dans le menu ; tests de la logique pure (config, mixeur, ring). Compilé pour Windows, mais pas encore lancé. **Reste à faire** : tout valider sur le PC (spike C d'abord), calibrer les niveaux sur de vrais clips, et tester 1440p en ultra-large. 120/144 fps retirés le 2026-10-09 (voir ci-dessous). |
| 10 | Bibliothèque de clips (voir ci-dessous) | Dans la fenêtre : liste des clips du dossier, lecteur intégré, copier dans le presse-papiers puis coller dans Discord, glisser-déposer vers Discord, renommer, supprimer (corbeille), montrer dans l'Explorateur. **Code écrit le 2026-10-09** : onglet *Clips* (liste nom/date/durée/taille mise à jour à chaque sauvegarde, lecteur avec plein écran et raccourcis espace/flèches/F, Copier, Renommer, Supprimer, Montrer), glisser natif ; modules `library` (pur, TDD) et `shell` ; page vérifiée dans Chromium avec une imitation de WebView2 (liste, sélection, actions, glisser au seul bouton gauche). **Reste à faire** : valider sur le PC (coller et glisser dans Discord surtout), et le spike C. |
| 11a | Habillage avec le design system (voir ci-dessous) | La fenêtre suit `docs/design-system/` : structure, styles, textes, boîtes de dialogue, lecteur et raccourcis de la liste ; seul `ui.html` change. **Code écrit le 2026-10-09** : styles du design system copiés dans la page, barre de lecture à nous (lecture, position, temps, son, plein écran ; glisser depuis l'image), boîtes « Supprimer ce clip ? » et « Renommer », textes courts, ↑/↓, Suppr, F2 ; mêmes messages vers Rust ; vérifié dans Chromium avec une imitation de WebView2. **Reste à faire** : valider dans la vraie fenêtre. |
| 11b | Ce que le design ajoute côté Rust (voir ci-dessous) | Bandeau d'état et « Ouvrir le dossier » ; vignette, définition et images/s, « Nouveau » sur chaque clip ; messages d'erreur courts. **Code écrit le 2026-10-09** (même PR que 11a, à la demande de l'utilisateur) : hauteur et cadence lues dans `moov` (tests purs, vérifiées sur de vrais clips : 720p et 1440p à 60 i/s), vignettes tirées par la page (vérifiées sur de vrais clips dans Chromium), commande `open_folder` partagée avec le menu de l'icône, « Nouveau » d'après la dernière ouverture de la fenêtre, erreurs courtes dans la fenêtre et détail au log. **Reste à faire** : valider dans la vraie fenêtre, dont la fluidité des vignettes avec beaucoup de clips. |

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
2. Pas de réglage de volume pour l'instant : on juge sur les vrais clips. *(Revu le 2026-10-09 : volume du micro réglable, itération 9.)*
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
- **Icône de l'application** : `assets/clipper.svg` (et `clipper-small.svg` pour 16 à 24 px), design définitif du 2026-10-09, convertie en `.ico` multi-tailles et compilée dans l'exe (crate de build `embed-resource`). L'icône de notification utilise la même ressource. Informations de version (nom, version) dans l'exe.

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

*Préréglages retirés à l'itération 9 (réglages séparés) ; la bascule à chaud est réutilisée.*

**Pourquoi** : la qualité (résolution, fps, taille cible) doit se changer sans éditer `clipper.toml` ni relancer Clipper.

**Ce qui est dans l'itération**
- **Préréglages** (`config::PRESETS`) : un préréglage n'est qu'un trio hauteur / fps / taille cible, écrit tel quel dans `clipper.toml` (pas de nouveau champ ; une config modifiée à la main ne correspond à aucun préréglage et rien n'est coché). « Discord gratuit » (720p60, 19 Mo), « Discord gratuit, plus net » (1080p30, 19 Mo), « Discord Nitro Basic » (1440p60, 48 Mo).
- **Sous-menu « Qualité »** dans le menu de l'icône, préréglage actif coché.
- **Application à chaud** : le thread principal arrête le pipeline vidéo (drapeau vérifié à chaque image), vide le buffer, le relance au nouveau format ; l'audio continue.

## Itérations 9 à 11 : la fenêtre Clipper

**Pourquoi** : un vrai logiciel, réglable sans fichier de config, où l'on revoit ses clips et où on les partage en un geste. Ordre voulu par l'utilisateur : **les fonctions d'abord** (visuel brut), puis l'habillage quand le design system sera disponible.

**Technologie (décidée le 2026-10-09)** : app native, **sans Electron**. Un seul exe Rust. Une fenêtre Win32 à nous dont le contenu est en HTML, affiché par **WebView2** (le moteur d'Edge, déjà présent dans Windows 11, rien à embarquer). Le lecteur vidéo est le `<video>` du moteur, qui lit nos MP4 H.264/AAC sans code de décodage. Le design system se posera ensuite en CSS sans toucher au Rust. Le moteur ne tourne que pendant que la fenêtre est ouverte (environ 100 Mo) ; fermer la fenêtre le détruit et Clipper reste dans la zone de notification. Nouvelle crate **autorisée le 2026-10-09** : `webview2-com` (liaison officielle, sans framework).

**Ouverture** : clic gauche sur l'icône, ou « Ouvrir Clipper » dans le menu.

### Spike C : WebView2

Le plus gros risque passe en premier : intégration à notre boucle de messages, appels HTML → Rust → HTML, lecture d'un MP4 local (dossier mappé sur un nom d'hôte virtuel), libération de la mémoire à la fermeture, et **glisser-déposer d'un fichier depuis la page vers Discord** (le glisser HTML ne transporte pas de fichier : il faudra sans doute lancer un glisser natif côté Rust).

### Itération 9 : réglages

**Ce qui est dans l'itération** (tout s'applique sans relancer Clipper, en réutilisant la bascule à chaud de l'itération 8)
- **Résolution** : 720p, 1080p, 1440p (la largeur suit le ratio de l'écran, comme aujourd'hui).
- **FPS** : 30, 60. *(120 et 144 retirés le 2026-10-09 : voir ci-dessous.)*
- **Qualité** : niveaux nommés **Basse / Moyenne / Haute / Très haute**. Le débit est fixé par le niveau et suit résolution × fps (calcul pur, testé). **Moyenne en 720p60 reproduit le débit actuel** (~4,4 Mb/s). Valeurs exactes calibrées pendant l'itération.
- **Taille estimée** affichée à côté et recalculée à chaque changement : (débit vidéo + 160 kb/s d'audio) × durée. C'est aussi, à peu près, la RAM que prend le buffer.
- **Durée** : valeur libre de **10 à 300 s** (30 par défaut).
- **Volume du micro** : **0 à 200 %** (100 % par défaut). Au-delà de 100 %, amplification, avec saturation à ±32767 dans le mixeur (déjà testée). Il s'applique au clip suivant, sans couper l'audio.
- **Dossier des clips** : sélecteur de dossier Windows. Les anciens clips restent où ils sont.
- **Raccourci** : capture d'une nouvelle combinaison ; refus clair si Windows la refuse (déjà prise).
- **Démarrer avec Windows** : même mécanisme que le menu de l'icône.
- **Menu de l'icône** : le sous-menu « Qualité » disparaît ; ajout de **« Ouvrir Clipper »** et d'une case **« Micro »** (on/off). Le reste ne change pas.
- **`clipper.toml`** : les préréglages Discord et `target_mb` sont retirés au profit des champs ci-dessus. **Changement cassant** (`feat(config)!`) : un ancien fichier est relu sans erreur (`target_mb` ignoré, les autres valeurs gardées).

**Ajouts du 2026-10-09 (retours de testeur)** : choix du micro dans la fenêtre (défaut : micro de communication de Windows) ; le son du PC et le micro suivent un changement de périphérique sans relancer Clipper.

**Tests (TDD, logique pure)** : débit par niveau × résolution × fps, estimation de taille, bornes (durée, volume), relecture d'un ancien `clipper.toml`, gain du micro dans le mixeur (0 %, 100 %, 200 % qui sature).

**Risques à vérifier pendant l'itération**
- **120/144 fps : retirés le 2026-10-09.** Par défaut, WGC ne livre pas plus d'environ 60 images/s, quelle que soit la fréquence de l'écran : au-delà, l'encodeur répétait des images (fichier plus lourd, rien de plus fluide). Lever cette limite demande `MinUpdateInterval` (Windows 11 24H2, build 26100), que la [PR OBS #12759](https://github.com/obsproject/obs-studio/pull/12759) propose sans être fusionnée. Un ancien `clipper.toml` à 120 ou 144 est ramené à 60. Hors scope : y revenir avec `MinUpdateInterval` si le besoin se confirme.
- **Mémoire** : 1440p60 « Très haute » sur 300 s approche 1 Go de RAM. L'estimation affichée le rend visible ; aucun plafond n'est prévu sans demande.

### Itération 10 : bibliothèque de clips

**Ce qui est dans l'itération**
- **Liste** des clips du dossier courant, du plus récent au plus ancien : nom, date, durée, taille. Elle se met à jour quand un clip est sauvegardé.
- **Lecteur intégré** façon Medal, propre : lecture/pause, barre de progression avec recherche, volume, plein écran, raccourcis clavier (espace, flèches, F). Rien de plus (découpe, vitesse, image par image : hors scope, proposés).
- **Copier dans le presse-papiers** (bouton) : le fichier lui-même (`CF_HDROP`), comme un Ctrl+C dans l'Explorateur ; un Ctrl+V dans Discord l'envoie.
- **Glisser-déposer** d'un clip vers Discord ou l'Explorateur.
- **Renommer** (le fichier sur disque).
- **Supprimer** : vers la corbeille Windows, donc récupérable.
- **Montrer dans l'Explorateur** : le dossier s'ouvre avec le fichier sélectionné.

**Validation manuelle** : coller et glisser dans Discord, lire un clip pendant qu'un autre se sauvegarde, renommer ou supprimer le clip en cours de lecture.

### Itération 11 : le design system

**Référence** : `docs/design-system/` (copie de l'[artefact validé](https://claude.ai/artifact/9ce3NWEX8TXJU9qXAVPZRZ) le 2026-10-09). Le code s'adapte au design system, jamais l'inverse ; un écart se règle en modifiant d'abord le design system. À lire avant de toucher à `ui.html` : son `README.md` (principes, textes, couleurs), puis `components/MainWindow`, `SettingsView`, `ClipItem`, `ClipPlayer`, `Dialog`.

**Décisions du 2026-10-09**
- Thème sombre seul, quel que soit le mode de Windows.
- Textes minimaux, sans nom d'application tierce ni de ses limites (« Copié », pas « Ctrl+V dans Discord ») : le texte n'a pas à changer quand un autre logiciel change ses règles.
- Onglet Clips : liste et lecteur côte à côte.
- Qualité : réglages séparés (résolution, images/s, qualité) et taille estimée, pas de préréglages : avec une durée réglable, une taille fixe sur un préréglage serait fausse.
- Suppression : confirmation, puis corbeille. Seule action confirmée.

**11a : habillage** (seul `ui.html` change, aucun code Rust)

| Aujourd'hui dans `ui.html` | Selon le design system |
|---|---|
| Styles bruts | `tokens.css` et `bundle.css` du design system, classes `cl-` |
| Commandes du lecteur fournies par le navigateur | Barre de Clipper : lecture, position, temps, volume, plein écran |
| `confirm()` et `prompt()` | Boîtes « Supprimer ce clip ? » et « Renommer » (composant Dialog) |
| « Copié : X. Ctrl+V dans Discord pour l'envoyer. » | « Copié » |
| « Montrer dans l'Explorateur », « Renommer… », « Supprimer », « Changer… » | Boutons-icônes « Afficher dans le dossier », « Renommer », « Mettre à la corbeille » ; « Modifier » (raccourci), « Changer » (dossier) |
| « Durée du clip », « Images par seconde », « Volume du micro », « Quel micro », « Dossier des clips » | « Durée », « Images/s », « Volume micro », « Périphérique », « Dossier » |
| « Par défaut (micro de communication de Windows) » | « Par défaut » |
| « ≈ X Mo par clip (l×h, N Mb/s) » | Ligne « Taille estimée » : « ≈ 17 Mo », puis définition et débit |
| Titre de clip = nom du fichier | Titre « Aujourd'hui · 21:14 », nom du fichier dessous, puis la taille |
| Pas de raccourci dans la liste | ↑/↓ change de clip, Suppr supprime, F2 renomme |

**11b : ce que le design ajoute côté Rust**
- **Bandeau d'état** : « Enregistrement · Alt+F10 sauvegarde les 30 dernières secondes », d'après le raccourci et la durée déjà envoyés à la page. Pas d'état « en pause » : si la capture s'arrête, Clipper s'arrête et le dit (le design system ne garde qu'un état).
- **« Ouvrir le dossier »** dans l'en-tête : commande `open_folder`, même action que le menu de l'icône (`shell::open_folder`).
- **Définition et images/s de chaque clip** (« 15,2 Mo · 720p · 60 i/s ») : hauteur lue dans `tkhd`, cadence = images de `stsz` ÷ durée de `mdhd`, sur la première piste vidéo (`library::moov_video`, pur).
- **Vignette** de chaque clip, 16:9, durée en bas à droite : tirée par la page, sans Rust. Un lecteur caché charge les clips visibles un par un, prend l'image à 1 s et la garde ; il lâche un clip avant son renommage ou sa suppression.
- **« Nouveau »** sur les clips modifiés depuis la dernière ouverture de la fenêtre (avant la première : depuis le lancement de Clipper) ; Rust envoie l'instant dans l'état (`new_since`).
- **Messages d'erreur** : courts dans la fenêtre (« Raccourci déjà utilisé », « Nom déjà pris », « Suppression impossible »), détail technique au log.

**Validation manuelle** : comparer chaque vue aux `preview.html` du design system, au clavier et à la souris.

Sortie de secours : si le spike A dépasse 3 soirées, on passe l'encodage et le mux à ffmpeg, et on garde WGC et WASAPI en natif.
