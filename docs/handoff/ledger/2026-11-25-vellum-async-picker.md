# Vellum document consent: nonblocking picker support

The native Files picker now exposes `pick_begin` and `pick_poll` in `sex-app`. A program can ask the person for one file, keep processing its own PDX messages while Loom shows the question, and poll until Files returns a path grant or a refusal. `pick_poll` treats `PENDING` as an unfinished question and rejects an empty or malformed successful reply. The existing blocking `pick` keeps its behavior and uses the same begin call.

The focused unit tests cover pending, granted, cancelled, and malformed replies. Full guest build and QEMU evidence are pending the exact-head GitHub CI run. This is support for a later Shibari document-review flow; this change does not alter Vellum's current signing approval or claim that it is document-bound.
