# Dialog

Confirms a deletion or asks for a new name. Replaces the browser's `confirm()` and `prompt()`.

- Delete: title "Supprimer ce clip ?", file name in `mono`, Annuler then Supprimer (primary). Annuler has focus.
- Rename: title "Renommer", a text field with the name selected and ".mp4" after it in `ink-muted`, Annuler then Renommer (primary). Entrée confirms.
- Échap cancels; focus returns to the element that opened it. Centered on `scrim`, panel `surface-1`, `line` edge, `radius-lg`, `space-4` padding, max 360px.
