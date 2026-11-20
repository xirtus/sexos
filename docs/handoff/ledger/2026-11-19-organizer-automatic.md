# Files organizes new arrivals automatically (2026-11-19, organizer plan phase 2)

- **What the person gets:** with `organize auto on`, a file that lands in `/Inbox`,
  `/Downloads`, `/Desktop` or a program's own `Downloads` folder (Kaleidoscope's) is
  moved by rule once it is saved: documents to `/Documents`, pictures to `/Pictures`,
  music to `/Music`, videos to `/Videos`; anything else stays. It works whether or not
  a terminal is open. `organize` / `organize --dry-run` do it now or preview it, with a
  reason for every file that stays.
- **The person's controls (Spindle):** `organize rule *.zip /Archives` (also
  `invoice-*` and `scan-*.pdf` patterns; their rules outrank the built-in ones; tied
  rules never move a file), `organize rules`, `organize pin F` / `unpin F`,
  `organize send F FOLDER` (an explicit, undoable placement), `organize undo` (puts the
  last file back **and pins it**, so automatic mode leaves it alone), `organize log`
  (each arrival and what was decided), `organize status`.
- **Events:** SexFiles stages every change to a file in an organized folder and turns it
  into a sequenced event only once the filesystem has committed it, so nothing moves on
  a write a power cut could take back. A file is placed after 1.5 s without writes;
  hidden names (`.NAME.part`) never are. Kaleidoscope now downloads into
  `Downloads/.NAME.part` and renames when whole. The ring (64 events) and staging table
  (32) fall back to a full scan when they overflow, and at every Files start.
- **Where it runs:** `servers/sexfiles/src/organize.rs`, inside the file authority,
  between requests; no timer and no Spindle polling (the old 64-tick Spindle poll is
  gone). Policy is `crates/sex-organize` (pure); pins, rules, the switch and the
  32-entry undo log are `crates/sexfs/src/organizer.rs`, each change in one commit.
- **ABI:** fs kinds renumbered 43-51 (MOVE_IF_ID, ORGANIZE_STATE, _SET, _MOVE, _UNDO,
  _SCAN, _PIN, _RULE, _LOG); #419's 30-34 collided with `sex_pdx::disks`. Programs are
  refused every organizer kind; MOVE_IF_ID stays confined to their own folder.
- **Gates:** `scripts/spindle_organize_gate.py` (preview, now, rules, auto, events,
  undo-pins, pins, send, restart) and `scripts/files_save_picker_gate.py` (Jot's Save As
  through Loom's picker grants exactly the named new file; save again; cancel), both in
  CI's "Disks, power and fuzzing" job. Host: `cargo test -p sex-organize`,
  `cargo test -p sexfs --test organizer --test checked_move`.
- **Not yet:** a separate Organizer protection domain (an IPC v2 notification from Files
  would wake it), a rule editor and review queue in Loom, a desktop that shows
  `/Desktop`, project-affinity rules.
