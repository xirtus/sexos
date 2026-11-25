# Networking above the driver and microkernel server upgrades (2026-11-25)

Gemini's six branches, combined (PR #819) and landed after a Claude review.

## Networking
- `crates/sex-net`: RFC 7323 window scaling, SACK codec and receive scoreboard, loss
  detection and fast recovery (`tcp_options.rs`); integer CUBIC and BBR
  (`congestion.rs`); SPSC descriptor rings, zero-copy pool and RSS per-core lanes
  (`datapath.rs`); a burst `Adapter`. Host gate: `scripts/tcp_modern_host_gate.sh`.
- DNS cache with DoT/DoH (`dns_secure.rs`) wired into `servers/sexnet/src/web.rs`;
  TLS 1.3 session resumption store and a shared `ClientConfig` cache
  (`crates/sex-tls/src/open.rs`); ALPN `h2` through sexnet into Kaleidoscope with
  fallback; HTTP/3, QUIC framing and QPACK foundations (`crates/sex-http/src/h3/`).
  Host gate: `scripts/web_protocols_host_gate.sh`.
- Modern virtio-net (`1af4:1041`) driver in sexnet (`servers/sexnet/src/virtio_net.rs`),
  used when no PCI Ethernet card opens; QEMU benchmark `scripts/virtio_net_bench_gate.py`.
  The Wi-Fi driver and tether code are untouched.

## Microkernel servers
- Files: 4 MiB CLOCK page cache and 8-stream read-ahead (`page_cache.rs`), IPC v2
  endpoint serving (`ipc_v2.rs`), reply-and-receive (syscall 103) on its reply paths.
  Host gate: `scripts/sexfiles_page_cache_host_gate.sh`.
- Fault recovery: every exit, fault, retirement and restart path purges per-core
  handoff slots naming the dead task (`scheduler::purge_handoff_for_task/_pd`) and
  tears down its IPC v2 state (`ipc_v2::syscall::teardown_task/_pd`), waking blocked
  clients with `E_RESTARTED` or `E_CLOSED`. Clients retry idempotent calls across a
  restart (`sex_pdx::ipc_recv::retry_on_peer_restart_with`, used by `sex-app`).
- One-hop window open: `OP_WINDOW_OPEN_COMBINED` replaces the separate create, clear and
  present calls of `Window::open`.
- Drive, Bell and Files reply with syscall 103 where they already blocked in
  listen; Display, Input and USB declare deadline classes with syscall 104.

## Review changes (Claude)
- `take_handoff` no longer drops a live task of a service being restarted (that lost its
  run token); it queues it like any wake. Exited, Dead and Constructed handoffs are
  still refused.
- Shell keeps its single listen path (the reply-and-receive shortcut for the autodemo
  tick put new messages ahead of the startup backlog), and Net keeps its receive loop
  (a held message skipped the pending-work branch the receipt custody check requires).
- Host test doubles, gates and self-wait pins updated for the new calls; the scheduler
  model lists the three new custody functions with a test for retiring a stale token.
  Gemini's host runs were on an arm64 Mac, where the new `x86_64`-only wrappers compile
  out, so several doubles only failed on x86_64 CI.
- The TLS resumption test missed resuming its second connection in about 1 of 3 runs
  with every TLS test in parallel (never alone; the client offered a ticket and the
  server did a full handshake). It now allows three follow-up connections and requires
  one to resume; the root cause is a follow-up. Tests that add roots no longer rebuild
  the shared config mid-test.
- Gemini's program-store directory cache (`DIRECTORY_SECTORS`) was dropped: it kept
  serving a directory after it was deleted or found corrupt, which the recording image
  loader test catches. Files reads the directory from disk on each load, as before.
- The recording runtime host doubles gained `ipc_recv::pdx_call_recv` and the
  trampoline's `reply_fast_queued!` for the new block call and reply paths.

## ABI
- New: `sex_pdx::endpoint::E_RESTARTED = 0xFFFF_FFFF_FFFF_FFF7`, `OP_BIND_ENDPOINT =
  0x5E02_0001`, rights bits and badge/request/reply codecs.
- New: `sex_pdx::app_lifecycle::OP_WINDOW_OPEN_COMBINED = 0x10B` (Shell and Display).
- No new syscalls; existing 102-104 gain callers.
