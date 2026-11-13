# Files: thumbnails grid, cached

Workstream E, slice E3 of `docs/handoff/DAILY_DRIVER_NEXT_100_SLICES.md`.

- Ctrl+G in Files shows the folder as a grid of tiles (pixels, names drawn with Geist Sans);
  arrows move across and by whole rows, Enter and every other key work as in the list.
  (`apps/files/src/thumbs.rs`)
- Pictures (PNG, JPEG, GIF, WebP, via `sex-image`) are shrunk to 112 px in the background, one
  per piece of work between keys, and kept in Files' own folder (`thumbs/`, named by path hash,
  size and modification time). PDF, video, audio, text, archive and folders get kind tiles; real
  PDF first pages and video frames are a follow-up.
- Gate: `scripts/files_thumbs_gate.py`: a stick of 50 PNG pictures (640x480): 50 thumbnails made
  in 5.8 s locally (budget 60 s), the first row's colours on screen; a second run draws all 50
  from the cache in 1.4 s with nothing decoded.
