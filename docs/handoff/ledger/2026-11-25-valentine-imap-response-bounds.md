# Valentine IMAP response bounds

- Gap: the mail protocol core accepted an IMAP `{n}` literal from a server and
  buffered `n` bytes without a limit. Repeated continuation lines could also
  accumulate unbounded response text before parsing.
- Change: reject a literal above 64 MiB before waiting for its body, and reject
  a response above 64 MiB total or 2 MiB of framing text before appending it.
  Larger messages need a streamed Files route when online mail is wired.
- Proof: `imap_bounds` host tests announce an oversized literal with no body
  and send repeated legal continuation lines past the text bound. Existing
  send/receive fixtures still pass. The Valentine guest gate continues to
  capture its local inbox and account-unavailable screen; no boot-routed
  Multipass/TLS service exists yet to exercise IMAP in QEMU.
- Checks: `cargo test --locked -p sex-mail` and `git diff --check` passed
  locally. Full image build and QEMU tests run only on GitHub CI.
