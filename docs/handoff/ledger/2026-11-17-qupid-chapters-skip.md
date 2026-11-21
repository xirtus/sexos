# Qupid Q2: chapters and the "Skip intro" card

The FFmpeg bridge reads chapters (`sx_chapter`, `Decoder::chapters`, `Player::chapters`).
Qupid turns them into skip segments with `sex_episodes::segments::from_chapters` and shows
a card in the lower right of the picture ("Skip intro (Enter)", also recap, credits and
preview) while one plays; Enter jumps to its end. C shows the chapter list, PgUp/PgDn
move between chapters, 1-9 jump to 10%-90%, M mutes. Gate:
`scripts/qupid_chapters_gate.py` (CI media job): a host-ffmpeg clip with Prologue /
Opening / Story chapters; the card is on screen during Opening and Enter lands at 7000 ms.
