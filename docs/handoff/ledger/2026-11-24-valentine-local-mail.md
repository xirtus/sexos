# Valentine native local mail view

- Branch: `codex/valentine-mail-list`.
- Gap: `sex-mail` already spoke IMAP/SMTP and parsed MIME, but no Valentine app
  window made mail readable on SexOS.
- Change: add a native Valentine program that lists, searches and opens `.eml`
  messages in its own Files grant. It renders only decoded plain text, counts
  attachments and leaves HTML remote content closed. The UI follows shared
  SexOS appearance tokens. The mail summary and search model live in `sex-mail`.
- Test: `sex-mail` host tests cover decoded headers, search and remote HTML
  detection. The CI QEMU gate imports a multipart message, opens, searches and
  reloads it, then captures `valentine-inbox.ppm` and
  `valentine-search.ppm` in `sexos-productivity-proof`.
- CI correction: compare the pointer's `i16` x coordinate as `i32` against the
  scaled list width; the first CI build caught this type mismatch.
- CI correction: QEMU's unshifted `semicolon` key typed `;` for both colons in
  the fixture URL, so the import never reached Spindle's fetch route. The gate
  now sends Shift+semicolon for `:` and Shift+2 for `@`; host tests assert the
  actual QMP key sequences, and the gate passes Python syntax checking.
- Limit: Multipass account flow, live IMAP/SMTP binding, drafts and sending are
  separate pending slices, documented in `codex-week/valentine.md`.
- Scope: Valentine app, mail summary model, build manifest, CI gate and docs.
  No kernel, Wi-Fi or tether changes.
