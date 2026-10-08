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

- micro ;
- commande vocale ;
- upload ;
- UI, tray ou notifications ;
- capture d'une fenêtre ou de plusieurs écrans ;
- HDR ;
- HEVC/AV1 ;
- fallback logiciel ;
- rotation des logs ;
- rechargement à chaud de la config ;
- hotkey « quitter » ;
- installeur ;
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
| 3 | Prod : sous-système Windows, log, bips, démarrage auto | Survit à un reboot et à une soirée de Hunt et R6. **Code fait le 2026-10-08** : pas de console, log fichier en heure locale, bips, clips dans Vidéos\Clipper, changement de résolution géré (bandes noires), double appui ignoré (testé), 37 tests. **Reste à faire** : démarrage auto, reboot, soirée de jeu et cas limites en jeu. |

Sortie de secours : si le spike A dépasse 3 soirées, on passe l'encodage et le mux à ffmpeg, et on garde WGC et WASAPI en natif.
