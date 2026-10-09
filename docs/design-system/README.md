# Clipper — design system

Source : https://claude.ai/artifact/9ce3NWEX8TXJU9qXAVPZRZ (version de référence). Jetons : `tokens.json`, compilés dans `tokens.css`. Composants : `components/<Nom>/README.md` et `preview.html` (s'ouvre dans un navigateur).

Clipper is a background replay buffer for Windows: one hotkey keeps the last seconds of a game as an MP4. The interface is a small utility window opened from the tray icon. It should feel like part of Windows 11, always dark, quiet, and gone the moment the user is back in game.

## Principles

- **The hotkey is the product.** It saves the last N seconds, N set by Durée (30 by default). The window only confirms the buffer is running, lists clips to share, and holds settings. Never add a step between pressing the hotkey and having a clip.
- **One glance, one gesture.** The default view answers "is it recording?" and "where is my last clip?" without scrolling. Selecting a clip plays it beside the list; sharing it is one click (Copier) or one drag into any app.
- **Confirm what removes.** Deleting a clip asks first (Dialog: "Supprimer ce clip ?", Annuler / Supprimer), then sends it to the Recycle Bin. Nothing else asks for confirmation.
- **Settings apply live.** No Save button, no restart, no help lines. Taille estimée shows the cost of the current choices.
- **Red means recording.** `rec` is reserved for the live-buffer dot, the brand mark, the selected tab and a Dialog's confirm button. It is never decoration.

## Content fundamentals

- **Less is more.** Every word must help the user act or tell them something new; otherwise cut it. Labels are one or two words ("Durée", "Micro", "Volume micro"), toasts one or two ("Copié"). No help lines under settings.
- **No third parties.** Never name another app or platform (chat apps, upload sites) or their limits. Clipper states facts — size, resolution, frame rate, duration — and the user judges where a clip fits. Copy must never need an update because someone else changed their rules.
- **French**, as in the tray menu. No exclamation marks, no emoji, no final period on labels, toasts and help lines.
- **Buttons and menu items: infinitive verb, sentence case, no final period.** Real copy: "Ouvrir le dossier des clips", "Démarrer avec Windows", "Quitter", "Copier", "Modifier", "Changer".
- **Show, don't instruct.** Prefer a Kbd or a control to a sentence: the empty state is "Aucun clip" plus Alt+F10, not a how-to. When a sentence is unavoidable, it addresses the user with "vous".
- **Numbers the French way**: comma decimal and a space before the unit — "15,2 Mo", "30 s", "100 %", "60 i/s". Resolutions as "720p", "1440p".
- **Dates are relative** in lists: "Aujourd’hui · 21:14", "Hier · 23:05", then "8 oct. · 21:14". File names stay as written on disk: `clip_20261008_211403.mp4`.
- **Errors say what is wrong** in a few words, then the fix if the user has one: "Raccourci déjà utilisé", "Nom déjà pris", "Clip en cours d'utilisation". No apology, no error code; the technical detail goes to the log.
- Product name: "Clipper", capital C, never "CLIPPER" or "clipper" in UI text.

## Visual foundations

**Colour.** Neutrals come from the app icon: `surface-2` (#2a2a31) is its tile, `surface-0` (#16161a) its outer ring; `ink` is the replay arrow's white; `rec` is the record dot.
- Lay the window on `surface-0`, group content in `surface-1` panels, use `surface-2` for hover, inputs, keys and secondary buttons.
- Body text in `ink`, secondary text in `ink-muted`; both pass 4.5:1 on every surface.
- `rec` is a shape colour only (dot, tab underline). For a filled button use `rec-fill` with `on-rec`; for red text use `rec-text`.
- Status text (`ok-text`, `rec-text`) always comes with words ("Copié", "Raccourci déjà utilisé"), never colour alone.
- Video sits on `tile` / `tile-deep` with `tile-ink`.
- One theme only, Sombre, whatever the Windows mode. Don't add a light theme or a theme setting.

**Type.** The Windows system face: Segoe UI Variable (`ui`, `display`), Cascadia Mono (`mono`) for file names, paths and hotkeys. Nothing to ship: both come with Windows 11.
- `title` once per view, `heading` for group titles and clip titles, `body` for labels, `button` for actions, `caption` in `ink-muted` for metadata and help.
- `mono` for anything the user might type or find on disk; `chip` for the duration on thumbnails.

**Space and layout.** A 4px base: `space-1` 4 · `space-2` 8 · `space-3` 12 · `space-4` 16 · `space-6` 24 · `space-8` 32.
- Window: `window-w` 960px default, 720px minimum, `space-6` gutter. Header (status + Ouvrir le dossier) and tabs stay fixed. Clips tab: list (`list-w` 340px) and player side by side, each scrolling on its own.
- Rows: `space-3` padding, `line` hairline between rows, inside a `surface-1` panel with `radius-lg`.
- Controls are `control-h` (32px) tall.

**Shape.** `radius-sm` 4px for keys and chips, `radius-md` 6px for buttons, inputs and thumbnails, `radius-lg` 8px for panels and the window (the Windows 11 corner), `radius-full` for the dot and switches.

**Borders, shadows, motion.** Edges are 1px `line` hairlines; controls that need an edge to be found use `line-strong` (3:1). No drop shadows inside the window (Windows draws the window's own). Motion stays under 150 ms: hover fills, the toast fading out. No looping animation, including the record dot.

**Scrollbars.** Thin, `line-strong` thumb on a transparent track, shown only when content overflows (a small window, a long clip list). Never the system's light scrollbar.

**States.** Hover: `surface-2` fill or a `line-strong` edge. Focus: a solid 2px `focus` ring, 2px offset, on every interactive element. Disabled: 45 % opacity, no hover.

## Iconography

- App icon: `assets/Logos/clipper.svg` from 32px up (tile, outer ring, replay arrow, record dot), `clipper-small.svg` for 16–24px (tray). See `assets/Logos/README.md`.
- UI glyphs: Clipper's own line set, inline SVG on a 16 × 16 grid, 1.5px stroke, round caps and joins, `currentColor`, no fill except the play triangle's outline. Current set: folder, play, pause, volume, full screen, copy, pencil, trash, check, film, sliders, chevron, minimise, close. A new glyph follows the same grid and stroke; no icon font, no emoji.
- An icon never stands alone without `aria-label` and a `title` tooltip.

## Views

- **MainWindow** (default): RecStatus, Ouvrir le dossier, tabs Clips / Réglages; Clips = ClipItem list + ClipPlayer side by side.
- **SettingsView**: Résolution, Images/s, Qualité, Durée, Taille estimée · Micro, Périphérique, Volume micro · Raccourci, Dossier, Démarrer avec Windows.
- **Dialog**: Supprimer ce clip ? and Renommer, in place of the browser's `confirm()` and `prompt()`.
- **Dropdown**: every list is drawn by the design system (Périphérique); never the system's `<select>`.
- **EmptyState** replaces the library until the first clip.
