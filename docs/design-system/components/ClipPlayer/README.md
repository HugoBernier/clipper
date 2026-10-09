# ClipPlayer

The right side of the Clips tab: the selected clip's video, its name and its actions.

- Video: 16:9, `tile` behind it, `radius-md`. Clipper's own control bar on `scrim` at the bottom, not the browser's: play/pause, seek bar (`rec-fill`), time "0:12 / 0:31" in `mono`, volume, full screen, all in `tile-ink`. The video itself can be dragged into any app.
- Keys when the list or the player has focus: Espace, ←/→ (5 s), F.
- Below: title and file name left; right, Copier (secondary), then icon buttons Renommer, Afficher dans le dossier, Mettre à la corbeille.
- Copier puts the file on the clipboard and shows "Copié" for 2.4 s next to the actions. No other text, no app name.
- Renommer and Supprimer open a Dialog; both unload the video first.
- No clip selected: empty `tile`, actions disabled.
