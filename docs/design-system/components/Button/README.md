# Button

Buttons with a 600-weight label, `control-h` tall, `radius-md` corners.

- `cl-btn--primary` (`rec-fill`, text `on-rec`): only the confirm button of a Dialog (Supprimer, Renommer).
- `cl-btn--secondary` (`surface-2`, `line` edge): visible actions: Ouvrir le dossier, Copier, Modifier, Changer.
- `cl-btn--ghost`: secondary actions; icon-only ones (Renommer, Afficher dans le dossier, Mettre à la corbeille) need `aria-label` and `title`.
- Labels: infinitive verb, at most 3 words, no final period, no ellipsis ("Modifier", not "Changer…").
- Disabled when no clip is selected.
