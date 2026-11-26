# Spindle private input diagnostics

- **Why:** Typing `run +net kaleidoscope --private URL` exposed the address one
  character at a time and as a full line in serial diagnostics. Spindle also
  retained that launch in disk-backed history and session restore.
- **Change:** Redact typed characters and command text in native Spindle and
  Shell Yarn receipts. Keep fixed command names needed by existing gates. Skip
  history and session restore for exact private Kaleidoscope launch syntax;
  Shell Yarn also skips its in-memory command echo and history. After Enter,
  wipe both prompts' vi undo buffers and Spindle's navigation snapshot so a
  private launch cannot be recalled. Yarn also excludes an unfinished browser
  command from session snapshots, since its URL may precede `--private`.
  Spindle filters private launches from older validated history snapshots on
  load and queues a scrubbed replacement for disk persistence.
- **Proof:** Unit tests cover exact private launch recognition and false
  positives. The existing Kaleidoscope private QEMU gate now types a private
  launch through Spindle, waits for the browser, captures a screenshot, and
  rejects its URL sentinel or a history push in the serial slice. It then
  checks vi undo leaves the prompt empty. Full build
  and QEMU proof are assigned to GitHub CI.
- **Limit:** Shell Yarn has no app launch command. Its saved sessions are
  in-memory only. Other commands may intentionally display their own output;
  this change addresses private browser launches and terminal input receipts.
