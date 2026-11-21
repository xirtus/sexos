# Qupid --thumb for Files and Loom

`qupid --thumb @IN @OUT SIZE` (apps/play/src/thumb.rs) runs with no window, like Gaze's and
Vellum's: a picture about 10% into the video (the first picture for a short clip, one that
cannot seek, or a song's cover art), fitted inside SIZE without upscaling, written whole as
u16 LE width, u16 LE height and RGB rows, status 0 only when written. A song without
cover art fails with no file. Gate `scripts/qupid_thumb_gate.py` (CI media job). Files can
add `("Video", b"qupid")` style helpers in apps/files/src/thumbs.rs.
