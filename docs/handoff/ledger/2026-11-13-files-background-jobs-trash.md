# Files: background copy with progress and cancel, a trash you can see into, delete for good

Workstream E, slices E1 and E2 of `docs/handoff/DAILY_DRIVER_NEXT_100_SLICES.md`.

- Copy, and moves between disks, run in the background in the Files app (`apps/files/src/jobs.rs`),
  one 192 KiB piece at a time between keys: a progress bar with bytes, rate and percent; the window
  stays usable; Esc stops the job. A file is written as hidden `.NAME.part` and renamed only when
  whole, so a stopped copy, a closed window or a power cut never leaves a file that looks complete.
  Stopping removes everything the job made; a stopped move keeps the source untouched (the source is
  removed only after every copy is whole and synced).
- The trash shows where each item came from; Ctrl+R restores, Ctrl+E (twice) empties it for good.
- Shift+Del (twice) deletes for good, any other key in between keeps the file.
- Ctrl+I shows a file's size and SHA-256 (read through in the background), a folder's file count and size.
- Gate: `scripts/files_jobs_gate.py` (CI devices job): a 32 MiB copy stopped at ~20% leaves the
  destination empty; copied again its SHA-256 read back through Files equals the source; trash →
  restore keeps the hash; Shift+Del and Ctrl+E delete for good. Local run: copy 32 MiB in 6 s under TCG.
