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
  endpoint serving (`ipc_v2.rs`).
  Host gate: `scripts/sexfiles_page_cache_host_gate.sh`.
- Fault recovery: every exit, fault, retirement and restart path purges per-core
  handoff slots naming the dead task (`scheduler::purge_handoff_for_task/_pd`) and
  tears down its IPC v2 state (`ipc_v2::syscall::teardown_task/_pd`), waking blocked
  clients with `E_RESTARTED` or `E_CLOSED`. Clients retry idempotent calls across a
  restart (`sex_pdx::ipc_recv::retry_on_peer_restart_with`, used by `sex-app`).
- One-hop window open: Shell and Display serve `OP_WINDOW_OPEN_COMBINED` (create, clear
  and present in one request). `Window::open` does not use it yet (see review changes).
- Bell replies with syscall 103 where it already blocked in listen; Display, Input and
  USB declare deadline classes with syscall 104.

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
- Drive's reply-and-receive loop, Files' block calls through syscall 102 and the wider
  reply-and-receive use in Files' trampoline were taken back out. With them in, Drive
  restarted during boot (a second `[sexdrive.init.start]` in the app-resource proof) and
  a Polyamour save never finished, on Gemini's run and on the re-land. Files keeps
  master's reply-and-receive for `fs::OP` and serves the IPC v2 bind request.
- `Window::open` is back to its separate calls. With the combined open, Display never
  admitted the program as its surface's owner (no `[display.app.admitted]`), its first
  text was never committed (the Jot gate timed out), and a program replaced by a new
  run of itself kept running, so five Qupid runs used up program memory (the Kodi gate
  failed with `no_memory`). Both failed on Gemini's run too. The combined open needs the
  app's own admission to Display before it can replace the calls.

## ABI
- New: `sex_pdx::endpoint::E_RESTARTED = 0xFFFF_FFFF_FFFF_FFF7`, `OP_BIND_ENDPOINT =
  0x5E02_0001`, rights bits and badge/request/reply codecs.
- New: `sex_pdx::app_lifecycle::OP_WINDOW_OPEN_COMBINED = 0x10B` (Shell and Display).
- No new syscalls; existing 102-104 gain callers.
