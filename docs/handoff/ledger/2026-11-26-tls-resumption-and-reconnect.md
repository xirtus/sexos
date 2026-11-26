# 2026-11-26 TLS 1.3 resumption flake and `sex-app` busy-vs-restart reconnect

## TLS 1.3 session resumption flake (`crates/sex-tls/src/open.rs`)

- **Root cause (test flake)**: `second_connection_to_same_host_resumes_tls_session`
  ran without holding `ROOTS_FROZEN.write()`, which had been placed in
  `alpn_h2_carries_fifty_streams_over_one_session` by mistake in `37e8fa7e0`. When a
  parallel test pushed a new test root between connection 0 and connection 1,
  `ClientCache::config_for` saw `TEST_ROOTS.len()` change and rebuilt `ClientConfig`
  with a fresh `WebPkiServerVerifier`. `rustls` then popped the cached ticket on
  connection 1 and rejected it in `ClientSessionValue::compatible_config` because the
  verifier pointer no longer matched, forcing a full handshake.
- **Root cause (production `Session::poll`)**: in `ConnectionState::WriteTraffic`,
  `else if self.peer_closed` ended the session immediately even when `discard > 0`
  (a complete post-handshake record such as `NewSessionTicket` had just been decoded
  while additional records remained in `wire_in`). Guarded with `self.peer_closed &&
  discard == 0` so all complete records drain before closing.
- **Fix**: moved `ROOTS_FROZEN.write()` into
  `second_connection_to_same_host_resumes_tls_session`, restored the strict
  `[false, true, true, true]` (`reused=0,1,1,1`) assertion, and verified 50/50
  consecutive parallel runs.

## `sex-app` busy backpressure vs. peer restart (`sex-pdx`, `sex-app`)

- **Root cause**: `files::once`, `net::call_held`, and `ipc_recv::call_with_reconnect`
  mapped `Err(ERR_SERVICE_NOT_READY)` to `AttemptOutcome::PeerRestarted`, burning one
  of the 8 `DEFAULT_RECONNECT_RETRIES` on every transient full-inbox refusal from a
  busy `Live` service (causing Polyamour save and Splicy export failures).
- **Fix**: added `AttemptOutcome::NotReady` in `crates/sex-pdx/src/ipc_recv.rs`.
  `retry_on_peer_restart_with` now queries `peer_state(slot)` on `NotReady`:
  - `PeerLife::Live` at an unchanged epoch (or unsupervised/contended snapshot): yields
    without consuming `max_retries` (bounded by `timeout_ticks`).
  - `PeerLife::GaveUp`: fails immediately with `ReconnectError::GaveUp`.
  - Non-`Live` state or epoch advance (`p.epoch > last_epoch`): counts as a real
    restart against `max_retries` and waits for `Live`.
- Re-wired `files::once` and `net::call_held` in `crates/sex-app/src/lib.rs` to use
  `retry_on_peer_restart` with `AttemptOutcome::NotReady` on `ERR_SERVICE_NOT_READY`,
  and added host tests in `tests/fault_recovery.rs` covering 25+ transient busy
  refusals, mixed busy + genuine restart recovery, `GaveUp` fast-fail, timeout, and
  epoch-advance bounding.
