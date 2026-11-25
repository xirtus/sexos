# Kaleidoscope private diagnostics

- Gap: private browsing kept history, cookies and session data out of Files, but
  URL, host, title, typed address, page console and script error values could
  enter the UART and kernel log through browser diagnostics. Those logs can be
  captured by QEMU or read by privileged local diagnostic tools.
- Change: route all content-bearing Kaleidoscope serial records through one
  process-lifetime privacy policy. Opening any private tab latches that policy
  before its page runs. The latch remains set across tab switches and late
  callbacks. Keep only fixed-schema numeric lifecycle receipts; the tab-shown
  receipt replaces its address with `<private>` after the latch.
- Proof: policy unit test covers the irreversible transition. A source audit
  test keeps new raw serial call sites on a small allowlist. The private QEMU
  gate checks a unique URL/query/title/console sentinel and a delayed WebSocket
  handshake after switching tabs, then captures `web-private-diagnostics.ppm`.
- Limit: public diagnostic detail is also reduced after any private tab has
  opened in that browser process. This prevents a late private callback from
  being mistaken for public work; per-request provenance can restore only
  demonstrably public detail later.
- Scope: the lower fetch, rendering, script, and app networking crates have no
  URL/content serial records; SexNet's runtime records carry flow IDs, sizes,
  states and payload hashes rather than URL text. Silk's window event records
  carry frame IDs, not titles. Shell search, screen reader and tray diagnostics
  can log text explicitly sent to those separate services; this change does not
  make those services private, and it does not hide network metadata from the
  destination or network operator.
- Local checks: focused `sex-web` policy test, browser x86_64 target check,
  source audit unit test, Python guest-gate syntax, and `git diff --check`
  passed. Full build and QEMU proof run on GitHub CI.
