# SettingsView

The Réglages tab: every option of `clipper.toml`, applied live, no Save button, no help lines.

- Three `cl-settings` panels, no headings, in this order:
  - Résolution (720p · 1080p · 1440p), Images/s (30 · 60), Qualité (Basse · Moyenne · Haute · Très haute), Durée (10–300 s), Taille estimée;
  - Micro, Périphérique (Select, first option "Par défaut"), Volume micro (0–200 %);
  - Raccourci, Dossier, Démarrer avec Windows.
- Taille estimée is recomputed on every change: "≈ 17 Mo", then resolution and bitrate in `caption`.
- Périphérique and Volume micro are disabled while Micro is off.
- A refused value shows its reason under the setting name in `rec-text` ("Raccourci déjà utilisé"); the previous value stays.
- A microphone that is unplugged keeps its name in Périphérique followed by " (débranché)".
