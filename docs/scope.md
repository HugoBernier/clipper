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
| 3 | Prod : sous-système Windows, log, bips, démarrage auto | Survit à un reboot et à une soirée de Hunt et R6. **Code fait le 2026-10-08** : pas de console, log fichier en heure locale, bips, clips dans Vidéos\Clipper, changement de résolution géré (bandes noires), double appui ignoré (testé), 37 tests. Installé avec démarrage auto le 2026-10-08 ; clips avec son validés par l'utilisateur. **Reste à faire** : reboot, soirée de jeu et cas limites en jeu. |
| 4 | Micro mixé au son du PC (voir ci-dessous) | Voix audible et synchro (< 1 image) avec le jeu ; micro absent ou débranché → clip quand même, avec le son du PC ; pas de dérive sur 1 h. **Code fait le 2026-10-08** : micro de communication capté (Scarlett Solo, paquets de 10 ms, ~8 ms de latence, aucune correction), mixeur pur (6 tests), clip mixé vidéo/audio alignés, 45 tests. **Reste à faire** : voix + tir en jeu, débrancher/rebrancher le micro, 1 h. |

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

Sortie de secours : si le spike A dépasse 3 soirées, on passe l'encodage et le mux à ffmpeg, et on garde WGC et WASAPI en natif.
