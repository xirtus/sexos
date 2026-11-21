# Kaleidoscope session restore (N13f), 2026-11-21

Like Chrome's "Continue where you left off", Kaleidoscope keeps the window's tabs as they change. It stores each tab's address, title, group and container, the groups (colour, name, folded), and which tab was shown. They go in `Kaleidoscope/session.tsv`, written only when the session changes.

Started without an address, Kaleidoscope opens the kept tabs again:
- Only the shown tab loads at once.
- The other tabs show their kept titles, and load the first time they are shown.

What is left out or skipped:
- Private tabs and the side panel are not kept.
- A private window (`--private`), the games' own programs (solitaire, chess and the others), and starting with an address all skip the restore. The session is still kept while you browse.

Code:
- `sex_web::session` handles the file (`Session::text`/`parse`, which leaves out bad lines), with unit tests.
- `apps/web/src/session.rs` has `save_session` and `restore_session`.
- `Tab.restore` holds the address a waiting tab loads when first shown.

Serial records: `[web.session.saved] tabs= bytes=`, `[web.session.restored] tabs= groups= shown=`.

Gate: `scripts/program_web_session_gate.py` (CI):
- Before the restart: four tabs, one of them private, and a group.
- After the restart: three tabs come back with B shown. A and C wait until shown, and B is still in its group.

Not yet: several windows, and each tab's back/forward history.
