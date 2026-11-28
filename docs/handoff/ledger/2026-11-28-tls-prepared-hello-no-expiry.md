# 2026-11-28: A locally prepared TLS hello no longer times out 30 s after L

Root cause of the intermittent "TLS layout differs from independently reconstructed
payload" failures in "Diagnostic keyboard payload proof (UNSAFE)"
(`browser_reserve_gate.py` -> `tls_prepare_gate.tls_geometry`).

- `tls_probe::Probe::start_mode` sets a 30 s deadline at L, and `advance` kept checking
  it after the preparation had reached `HelloPending`. A local preparation then turns
  into `TimedOut` 30 s after L even though it waits on nothing. The gate holds that
  HelloPending through three captures (prepared, ZoomToggle expanded, restored), and on
  a slow runner the third capture landed after 30 s: the latest layout receipt carried
  state_code=6 ("timed out"), so every retry in the 45 s capture loop saw a payload that
  differed from the HelloPending model. A rerun on a faster runner passed.
- Fix (`apps/kaleidoscope/src/tls_probe.rs`): a local preparation (`!exchange`) that
  reached HelloPending is `prepared_locally`; `advance` leaves it alone, so the deadline
  bounds only the wait for the policy/seed/time clients. It holds its hello until Q
  (clear), Y or close, as the panel says. An exchange's HelloPending keeps the deadline
  (it still waits for DNS).
- `Probe::polling()` (busy and not prepared locally) replaces `tls.busy()` in Browser's
  two "more work without an event" checks (`apps/kaleidoscope/src/main.rs`), so a
  prepared hello blocks on the inbox instead of waking every 2 ticks. `busy()` is
  unchanged for command admission and resource accounting.
- No ABI change.
