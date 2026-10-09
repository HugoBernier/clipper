# Dropdown

Picks one value from a list too long or too variable for Segmented: the microphone (Périphérique). A list drawn by the design system, never the system's `<select>`.

- Button: 280px, `control-h` tall, `surface-2` with a `line-strong` edge, current value ellipsised, chevron on the right (turned up while open).
- Menu: opens under the button, right-aligned, at least as wide as the button, max 360 × 240px then scrolls; `surface-1`, `line-strong` edge, `radius-md`, `space-1` padding. No shadow.
- Options: `control-h` tall, check mark before the chosen one (600 weight), `surface-2` on hover and on the keyboard-active one.
- First option "Par défaut". Only plugged-in microphones are listed: if the chosen one is unplugged, Clipper records from the default one and "Par défaut" shows as chosen. Choosing applies at once.
- Keyboard: ↓, ↑, Entrée or Espace open; ↑/↓ move; Entrée or Espace choose; Échap closes and returns focus to the button. A click outside or Tab closes without taking focus back.
- Disabled (Micro off): `disabled` on the button, not only greyed.
- ARIA: button `aria-haspopup="listbox"` and `aria-expanded`; menu `role="listbox"`, options `role="option"` with `aria-selected`.
- Consumer provides: options (value, name), chosen value, onChange.
