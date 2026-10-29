# Display startup readiness handshake

Date: 2026-10-28. Addresses the observed early shell/SilkBar-to-display sends
that made the strengthened runtime ORDER_GATE fail.

## Behavior

Display completes its existing renderer initialization and initial render,
then publishes `OP_DISPLAY_READY` to shell and SilkBar. The existing
`[sexdisplay.ready]` marker remains at that completion point. Notifications
use Domain capabilities granted in kernel init's static Phase-25 capability
genesis: display's existing slot 6 targets shell; slot 7 targets SilkBar.
No runtime capability minting, global readiness flag, or shared pointer is used.

`crates/sex-pdx/src/display_ready.rs` defines opcode `0x101`, version 1 in arg0,
and zero reserved arg1/arg2. Receivers require kernel-authenticated caller PD 1,
and reject wrong caller/version/reserved fields. Kernel init asserts display's
fixed genesis PD identity before granting the return edges. This notification
grants no authority; the existing capabilities remain the sole invocation path.
The ABI is recorded in `sexos_contract.toml`.

Display polls at most once per pending recipient per event-loop pass. A failed
enqueue retains that recipient's pending bit for retry; successful enqueue
clears only that bit. Display continues draining its ordinary message ring
between retries. The same notification is not resent after acceptance.

Shell and SilkBar wait cooperatively before their ordinary startup work. Each
uses a static, local 256-message inbox to retain unrelated messages received
while waiting. Static allocation avoids placing another ring on the 64 KiB
task stack. Invalid readiness controls are rejected. Missing readiness never
becomes success through a timeout. Full deferred storage causes explicit
startup failure before another dequeue, preserving the existing deferred
values and leaving subsequent messages in the kernel ring rather than silently
overwriting or dropping them. This is bounded failure containment under a flood,
not guaranteed startup availability under unlimited incoming traffic.

After readiness, deferred messages are delivered FIFO through the normal
dispatch path before new messages. Shell's input-first drain, synthetic proof
work, and initial synchronous Linen painting are bypassed until that backlog
empties. Nested Linen painting uses cached state while deferred messages remain;
it does not mark the remote snapshot fetched until a real fetch is allowed.
Static inbox access uses short-lived borrows, avoiding aliasing with nested
handlers. SilkBar's single normal receive path drains its inbox first.

Shell's required display WM registration and cursor creation use a bounded
64-attempt retry on `ERR_SERVICE_NOT_READY`; other errors fail immediately, and
exhaustion fails startup explicitly. SilkBar advances its deferred initialization
cursor only after a successful enqueue, retaining failed and subsequent work
for a later loop pass. Its first-send marker now records the first successful
enqueue, rather than an attempted call. Later existing best-effort drawing
protocols retain their prior behavior.

## Scope and limitations

This is a boot-only readiness notification, not a restart/epoch protocol or a
promise of continued display availability. The fixed PD identity is part of
the current static boot graph and is asserted, not inferred from payload data.
The two new Domain edges authorize message enqueue to their destinations; the
existing capability representation does not restrict them to a single opcode.
Readiness receivers perform the opcode/version/caller validation themselves.

No scheduler, interrupt, TaskContext, IRET, RSP0, paging, or task-wakeup ownership
changes are part of this handshake. Pager recovery and the scheduler wake gap
remain separate unfinished work. Startup flood containment can halt a client;
it does not discard a full inbox and continue with incomplete state.

## Verification

`./scripts/display_ready_protocol_gate.sh`: PASS, nine tests execute the actual
protocol with syscall-boundary doubles. They cover authenticated shape,
unrelated FIFO retention, forged readiness, duplicate readiness, overflow
ownership, full-wait non-dequeue, independent recipient retry, bounded polling,
preserved send arguments, capability errors, and retry exhaustion. Source
checks cover TOML agreement, the actual initialization/publish ordering, boot
grants, truthful first-send marker, startup-send retention, and all three shell
receive paths' backlog guards.

`git diff --check`: PASS.

The parent integration lane must run sealed `make full`, then a fresh runtime
gate proving ORDER_GATE passes and authentic readiness acceptance precedes
ordinary display traffic. Repeat synthetic USB and persistence proofs because
startup message retention affects shell input arrival timing. Host tests do
not substitute for those runtime results.

## Parent integration closeout (2026-10-28)

Sealed build and 40-second QEMU master gate: **GREEN_MASTER**. All four USB
lanes and the strengthened two-boot persistence/rendering gate pass on the same
image. See [the execution report](SEXOS_RELIABILITY_ROADMAP_2026_10_28.md)
for source/image provenance, commands, evidence paths and remaining limitations.
