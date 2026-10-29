# Authenticated shell readiness before input production

Date: 2026-10-28. Scope: boot-only SASOS dependency over sex-pdx.

## Observed failure

The first integrated image failed ORDER_GATE in
`/tmp/sexos-truth-runtime-20261028/serial.log`: sexinput's first marked HID send
was line 2612, but silk-shell readiness was line 2918. Display readiness was
line 2770, and shell was still awaiting display while input started sending.
An unmarked synthetic HID call occurred even earlier in sexinput startup.

The gate was correct. A successful Domain enqueue only proves acceptance into
the receiver ring; `send_shell_hid_event` incorrectly used that as readiness.
`sys_set_state` is not a readiness barrier: its kernel setter is a no-op.

## Protocol and boot authority

`crates/sex-pdx/src/shell_ready.rs` defines:

- `OP_SHELL_READY = 0x102` (258), `SHELL_READY_VERSION = 1`.
- Authenticated publisher `SHELL_PD_ID = 3`; recipient `INPUT_PD_ID = 4`.
- `arg0 = version`, reserved `arg1 = arg2 = 0`.
- Private shell table slot `SLOT_SHELL_INPUT_READY = 9` routes to sexinput.

Kernel init grants this Domain return edge during the existing boot genesis
phase, before `phase25.complete`, with exact publisher/recipient identity and
vacant-slot assertions. Runtime code only invokes the granted edge. No ambient
memory flag, new authority decision, kernel readiness policy, or page table is
introduced.

Slot collision review: `CapEngine::grant_initial_rights` grants root VFS at 1,
self-Domain at 2, and self-signal Interrupt at 3. Consequently shell slot 3
cannot be reused despite the general `SLOT_INPUT` name. Shell slot 9 has no
existing grant or invocation; sexusb's separate table uses its own slot 9 to
reach sexinput. The startup assertion prevents replacing any future shell
authority silently. Existing slot 3 self-signal authority is preserved.

## Runtime behavior

Shell creates its publisher after both existing readiness markers. It polls
once at the start of each normal loop; failed enqueue retains the notification
and normal input draining continues. Success is emitted once as
`[silk-shell.ready.notify] to=sexinput`. While publication is pending, the empty
receive path yields back to the publisher instead of entering an indefinite
blocking listen. No readiness marker moved and no ordering check was relaxed.

Sexinput waits before its main loop, including every synthetic, USB and PS/2
producer. Only the kernel-supplied caller 3 with the exact opcode/version/zero
reserved fields can unlock it. It emits
`[sexinput.shell.ready.accept] caller=3` after validation. A static 256-message
FIFO preserves all ordinary incoming payloads while waiting and drains before
new slot-zero messages. Forged/malformed readiness controls are rejected and
duplicate valid controls are idempotent. At capacity, wait fails before another
dequeue, logs `[shell.ready.wait.failed] reason=inbox_full`, and fail-stops;
overflow cannot silently discard input or become readiness success.

## Verification and limits

Red-first: three tests read the unmodified production startup sources and all
failed before implementation: no authenticated wait, no post-ready publisher,
and no boot return edge. Evidence: `/tmp/sexos-shell-ready-red.log`. These are
source integration regressions, not a runtime interleaving proof.

`./scripts/shell_ready_protocol_gate.sh`: **12/12 PASS**, comprising nine tests
of the actual protocol with deterministic syscall doubles and three production
source wiring checks. Coverage includes authenticated shape, forged controls,
exact USB keyboard/pointer payload preservation, FIFO wrap and overflow,
duplicate controls, failed enqueue retries, and bounded publisher work while
normal receives progress.

`./scripts/display_ready_protocol_gate.sh`: existing **9/9 PASS**, plus contract
and startup wiring checks. The display protocol was not changed.

The parent owns contract/ABI hash synchronization, sealed image builds, and
QEMU order/runtime verification. This protocol is boot-only: it does not claim
restart epochs, runtime availability, or the unresolved scheduler wake/stack
handoff protocol. Serial notify and accept markers may interleave after enqueue;
both depend on shell ready, and first input work depends on acceptance.
