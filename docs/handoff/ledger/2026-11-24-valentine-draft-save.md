# Valentine local draft compose and safe save

- Branch: `codex/valentine-draft-save` (stacked on `codex/valentine-mail-list`).
- Gap: the first native viewer could read messages but not compose a local
  draft or preserve text across a restart.
- Change: Ctrl+N starts an unsent draft; To, Subject and body are editable;
  Ctrl+S saves, and idle edits save after three seconds. Escape and close try
  to save pending text. Files writes a unique `.pending` sibling, syncs it,
  renames it over the draft and syncs again, keeping the prior draft intact
  if a write fails. Ctrl+E reopens a local unsent draft. No message is sent.
- Tests: host round trip and header-injection checks for `LocalDraft`; QEMU
  composes, saves, restarts, searches and reopens the draft, preserving
  `valentine-draft.ppm` and `valentine-draft-reopened.ppm` screenshots in the
  productivity artifact. Full build and QEMU run only on GitHub CI.
- Scope: Valentine, its mail model, QEMU gate, gap list and this ledger.
