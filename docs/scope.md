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
- préréglages de qualité (retirés à l'itération 9).

Les six premières lignes ci-dessus ont été proposées le 2026-10-09 : à demander quand le besoin se présente.

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
| C | Spike : fenêtre WebView2 (voir itération 9) | Une fenêtre Win32 affiche une page HTML locale et lit un clip dans un `<video>` ; un bouton HTML déclenche du code Rust et reçoit une réponse ; fermer la fenêtre libère le moteur (RAM revenue au niveau d'avant) ; un glisser depuis la page dépose le fichier dans Discord. |
| 9 | Réglages dans une fenêtre (voir ci-dessous) | Depuis la fenêtre, sans relancer Clipper : résolution, fps, qualité (avec taille estimée), durée, volume du micro, dossier, raccourci, démarrage avec Windows ; le clip suivant respecte chaque réglage ; un ancien `clipper.toml` est relu sans erreur. Visuel brut : le design system viendra ensuite. |
| 10 | Bibliothèque de clips (voir ci-dessous) | Dans la fenêtre : liste des clips du dossier, lecteur intégré, copier dans le presse-papiers puis coller dans Discord, glisser-déposer vers Discord, renommer, supprimer (corbeille), montrer dans l'Explorateur. |
| 11 | Habillage avec le design system | La fenêtre suit le design system (fourni par l'utilisateur) ; aucune fonction ajoutée. |

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

*Préréglages retirés à l'itération 9 (réglages séparés) ; la bascule à chaud est réutilisée.*

**Pourquoi** : la qualité (résolution, fps, taille cible) doit se changer sans éditer `clipper.toml` ni relancer Clipper.

**Ce qui est dans l'itération**
- **Préréglages** (`config::PRESETS`) : un préréglage n'est qu'un trio hauteur / fps / taille cible, écrit tel quel dans `clipper.toml` (pas de nouveau champ ; une config modifiée à la main ne correspond à aucun préréglage et rien n'est coché). « Discord gratuit » (720p60, 19 Mo), « Discord gratuit, plus net » (1080p30, 19 Mo), « Discord Nitro Basic » (1440p60, 48 Mo).
- **Sous-menu « Qualité »** dans le menu de l'icône, préréglage actif coché.
- **Application à chaud** : le thread principal arrête le pipeline vidéo (drapeau vérifié à chaque image), vide le buffer, le relance au nouveau format ; l'audio continue.

## Itérations 9 à 11 : la fenêtre Clipper

**Pourquoi** : un vrai logiciel, réglable sans fichier de config, où l'on revoit ses clips et où on les partage en un geste. Ordre voulu par l'utilisateur : **les fonctions d'abord** (visuel brut), puis l'habillage quand le design system sera disponible.

**Technologie (décidée le 2026-10-09)** : app native, **sans Electron**. Un seul exe Rust. Une fenêtre Win32 à nous dont le contenu est en HTML, affiché par **WebView2** (le moteur d'Edge, déjà présent dans Windows 11, rien à embarquer). Le lecteur vidéo est le `<video>` du moteur, qui lit nos MP4 H.264/AAC sans code de décodage. Le design system se posera ensuite en CSS sans toucher au Rust. Le moteur ne tourne que pendant que la fenêtre est ouverte (environ 100 Mo) ; fermer la fenêtre le détruit et Clipper reste dans la zone de notification. Nouvelle crate **à autoriser dans `CLAUDE.md`** : `webview2-com` (liaison officielle, sans framework).

**Ouverture** : clic gauche sur l'icône, ou « Ouvrir Clipper » dans le menu.

### Spike C : WebView2

Le plus gros risque passe en premier : intégration à notre boucle de messages, appels HTML → Rust → HTML, lecture d'un MP4 local (dossier mappé sur un nom d'hôte virtuel), libération de la mémoire à la fermeture, et **glisser-déposer d'un fichier depuis la page vers Discord** (le glisser HTML ne transporte pas de fichier : il faudra sans doute lancer un glisser natif côté Rust).

### Itération 9 : réglages

**Ce qui est dans l'itération** (tout s'applique sans relancer Clipper, en réutilisant la bascule à chaud de l'itération 8)
- **Résolution** : 720p, 1080p, 1440p (la largeur suit le ratio de l'écran, comme aujourd'hui).
- **FPS** : 30, 60, 120, 144.
- **Qualité** : niveaux nommés **Basse / Moyenne / Haute / Très haute**. Le débit est fixé par le niveau et suit résolution × fps (calcul pur, testé). **Moyenne en 720p60 reproduit le débit actuel** (~4,4 Mb/s). Valeurs exactes calibrées pendant l'itération.
- **Taille estimée** affichée à côté et recalculée à chaque changement : (débit vidéo + 160 kb/s d'audio) × durée. C'est aussi, à peu près, la RAM que prend le buffer.
- **Durée** : valeur libre de **10 à 300 s** (30 par défaut).
- **Volume du micro** : **0 à 200 %** (100 % par défaut). Au-delà de 100 %, amplification, avec saturation à ±32767 dans le mixeur (déjà testée). Il s'applique au clip suivant, sans couper l'audio.
- **Dossier des clips** : sélecteur de dossier Windows. Les anciens clips restent où ils sont.
- **Raccourci** : capture d'une nouvelle combinaison ; refus clair si Windows la refuse (déjà prise).
- **Démarrer avec Windows** : même mécanisme que le menu de l'icône.
- **Menu de l'icône** : le sous-menu « Qualité » disparaît ; ajout de **« Ouvrir Clipper »** et d'une case **« Micro »** (on/off). Le reste ne change pas.
- **`clipper.toml`** : les préréglages Discord et `target_mb` sont retirés au profit des champs ci-dessus. **Changement cassant** (`feat(config)!`) : un ancien fichier est relu sans erreur (`target_mb` ignoré, les autres valeurs gardées).

**Tests (TDD, logique pure)** : débit par niveau × résolution × fps, estimation de taille, bornes (durée, volume), relecture d'un ancien `clipper.toml`, gain du micro dans le mixeur (0 %, 100 %, 200 % qui sature).

**Risques à vérifier pendant l'itération**
- **120/144 fps** : WGC ne livre pas plus d'images que la fréquence de l'écran. Sur un écran à 60 Hz, 144 fps double les images sans rien gagner. À afficher ou à brider selon l'écran.
- **1440p à 120/144 fps sur ultra-large** (3440×1440) : ça dépasse le niveau H.264 5.2 (environ 2,07 M macroblocs/s, contre 2,3 M à 120 fps et 2,8 M à 144). AMF peut refuser, et des lecteurs (Discord compris) peuvent ne pas lire. Si le test échoue, ces combinaisons sont grisées.
- **Mémoire** : 1440p144 « Très haute » sur 300 s peut dépasser 1 Go de RAM. L'estimation affichée le rend visible ; aucun plafond n'est prévu sans demande.

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

### Itération 11 : habillage

Le design system (disponible plus tard) habille la fenêtre : CSS et structure HTML seulement, aucune fonction ajoutée.

**Question ouverte** : des vignettes dans la liste (comme Medal) ? Elles dépendent du design et ne sont pas demandées pour l'instant.

Sortie de secours : si le spike A dépasse 3 soirées, on passe l'encodage et le mux à ffmpeg, et on garde WGC et WASAPI en natif.
