# Files: search by name and by words inside files, indexed; real modification times

Workstream E, slice E4 of `docs/handoff/DAILY_DRIVER_NEXT_100_SLICES.md`.

- Ctrl+F in Files searches the folder being looked at and everything under it, in the background
  (Esc stops and keeps what was found): names containing every term, and text files (up to 1 MiB,
  no zero bytes) holding a word starting with every term. Results list in place of the folder;
  Enter opens the folder a result is in, on it; Esc goes back. (`apps/files/src/search.rs`)
- The words are kept in an index in Files' own folder (`search.idx`) by path, size and modification
  time; a later search reads only changed files and drops files that are gone. Files with an unknown
  time are read again rather than trusted.
- sexfs files now get real modification times: `sexfiles` stamps creates, writes and truncates with
  Unix seconds from the civil clock (`sex_pdx::civil_time_ms`), where it used to write 0.
- Gate: `scripts/files_jobs_gate.py` step 6: first search reads 2 files, the second reads none and
  answers in 8 ms locally (gate bound: < 1 s).
