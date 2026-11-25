# Loom restores folder tabs

- Branch: `codex/loom-tab-session` (stacked on `codex/loom-folder-tabs`).
- Gap: folder tabs vanished when Loom closed or restarted.
- Change: store bounded, versioned tab paths, cursor, scroll position and active
  tab in `.loom-tabs` under the granted root. Validate paths against the grant and
  existing directories on load. Save through a staged file and a recoverable
  previous copy, because removable FAT volumes refuse rename over an existing
  destination. Hide the session files from the browser view.
- Tests: `loom-core` host tests reject truncated, malformed and traversing
  sessions; the Loom QEMU gate opens two tabs, restarts the app, verifies the
  restored active folder and captures `folder-tabs-restored.ppm`.
- Scope: `loom-core` session codec, Loom app, its QEMU gate, gap list and ledger.
- CI fix: compare the root as a byte slice when validating restored tab paths;
  the target Rust toolchain requires a slice pattern for `strip_prefix`.
