# RecStatus

States whether the buffer is filling: the `rec` dot and a short line.

- On: solid `rec` dot, "Enregistrement", sub-line "Alt+F10 sauvegarde les 30 dernières secondes" with the current hotkey and Durée.
- Off: hollow dot in `line-strong`, "En pause", sub-line with the reason ("Aucun écran détecté").
- The dot never carries the state alone.
