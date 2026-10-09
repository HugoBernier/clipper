# MainWindow

The window Clipper opens on: recording status and Ouvrir le dossier in the header, then two tabs, Clips (default) and Réglages.

- Opened from the tray icon (left click, or Ouvrir Clipper in its menu). Closing it destroys the page; Clipper keeps recording in the tray.
- Structure: `cl-window` > `cl-titlebar` > `cl-header` (RecStatus left, Ouvrir le dossier right) > `cl-nav` (Tabs) > one `cl-body` per tab.
- Width `window-w` (960px), minimum 720px. Header and tabs never scroll.
- Clips tab: `cl-library`, the ClipItem list (`list-w`, 340px) on the left and ClipPlayer on the right, both scrolling independently.
- An action that fails shows its error in the header, left of Ouvrir le dossier, in `rec-text`, for 6 s: visible on both tabs. "Copié" stays next to the player's actions.
- Consumer provides: recording state, hotkey, clip duration, clip list (newest first), selected clip.
