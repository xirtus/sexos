# Date: the earliest-started window owns Bell notices

Two open Date windows elect one of them to post Bell notices. The election
picked the smallest task number, but a reborn program row's task carries its
generation in the high bits (1073741825 vs 44), so a newer window could win,
the first window kept believing it owned delivery too, and the calendar gate
never saw the second window report `owns=0`.

Each window's lease file (`notice.ui-TASK`) now holds two words: the refreshed
lease time and the time the run started. The live run that started first owns
delivery; files from older builds (one word) sort last.
