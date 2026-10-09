# ClipItem

One clip in the library list: thumbnail, date, file name, size. Selecting it loads it in ClipPlayer.

- Thumbnail 96 × 54px (16:9), the frame at 1 s cropped to 16:9, `tile` until it loads (and for a file that cannot be read without downloading it), duration chip bottom-right.
- Title: relative day and time ("Aujourd’hui · 21:14", "Hier · 23:05", then "7 oct. · 21:58"). File name in `mono`, ellipsised. Meta in `caption`: size, resolution, frame rate ("15,2 Mo · 720p · 60 i/s"). Facts only, no verdict on size.
- Clips saved since the window was last closed (or since Clipper started) get "Nouveau" in `ok-text`.
- Selected: `surface-2` fill, `aria-selected="true"`. The list is a `listbox`: ↑/↓ move the selection, Suppr deletes, F2 renames.
- A left-button drag starts a native file drag (into any app or the Explorer); a click selects.
- Consumer provides: thumbnail URL, timestamp, file name, size in bytes, duration, height, fps.
