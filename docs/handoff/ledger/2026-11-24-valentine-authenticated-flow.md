# Valentine authenticated mail flow prerequisite

- Branch: `codex/valentine-authenticated-flow` (stacked on
  `codex/valentine-draft-save`).
- Gap: IMAP and SMTP clients expected to read a greeting and authenticate
  themselves. A future Multipass flow handoff will already have consumed both
  steps; Valentine must be able to continue the same stream without seeing a
  password or bearer token.
- Change: add `resume_authenticated` constructors for IMAP and SMTP clients.
  Show a clear local-only account status in Valentine's window and on Ctrl+M.
  This does not connect to an account.
- Test: a host fixture authenticates both flows as Multipass would, hands the
  live stream to Valentine, and verifies IMAP folder access and SMTP delivery
  without passing secrets through the resumed client. The QEMU gate records the
  unavailable account status and captures `valentine-account-status.ppm`.
- Remaining dependency: `crates/sex-pdx/src/accounts.rs` defines
  `OPEN_MAIL_FLOW`, but there is no boot-routed `servers/multipass` or safe flow
  handoff. Those must land before live mail can use these constructors.
- Scope: mail core, Valentine UI, guest gate, gap list and this ledger.
