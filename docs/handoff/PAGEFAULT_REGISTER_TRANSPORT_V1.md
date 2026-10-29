# Page-fault register transport V1

Date: 2026-10-28 (Europe/Berlin).

## Result and scope

The fault forwarder now sends an ordinary `IpcCall` carrying values rather than
opcode zero and a pointer to a temporary `MessageType::PageFault`. Zero remains
the listener's empty sentinel. The dedicated protocol is defined in
`crates/sex-pdx/src/pagefault.rs` and `[ipc.page_fault]` in `sexos_contract.toml`.

| Listen register | Value |
| --- | --- |
| rax / type_id | `OP_PAGE_FAULT = 0x100` (256), reserved on the sext edge |
| rsi / caller_pd | Authenticated sending protection-domain ID |
| rdx / arg0 | Fault address, by value |
| r10 / arg1 | Zero-extended `u32` x86 page-fault error code |
| r8 / arg2 | Faulting protection-domain ID, by value |

This notification grants no mapping authority and carries no borrowed pointer
or lent capability. A receiver must decode and bind `arg2` to `caller_pd`, then
apply its own authorized policy. Both canonical 48-bit address halves, address
zero, and protection-error flags are representable. Transport validation never
means an address is permitted, user-owned, or eligible for allocation. LA57 is
outside the current four-level address contract.

`forward_page_fault` rejects malformed addresses, kernel domain zero, IDs wider
than `u32`, IDs outside the kernel registry, and a faulting ID different from
the current protection domain. Slot 2 must exist in that domain's own capability
table and be a Domain enqueue edge with an in-range nonzero target distinct
from the faulting domain. A SyncCall
edge is rejected: it executes in kernel context and its current ABI drops the
opcode. `safe_pdx_call` still performs the actual capability lookup, edge
resolution, and enqueue; missing targets and full queues propagate errors.
Dispatch `Ok(())` means acceptance only, never page resolution or task resumption.

The earlier stub report's absent-slot assumption was incomplete:
`CapEngine::grant_initial_rights` inserts a Node capability first and a
`Domain(pd.id)` capability second. Auto-allocation therefore puts the self edge
in slot 2 even with no sext. This forwarder explicitly rejects that self edge;
otherwise the repaired nonzero opcode would queue a fake pager notification to
the blocked faulting domain itself. A future real boot grant must replace that
legacy self edge with a distinct authorized pager target.

## Task identity finding

Main tasks currently use a task ID equal to their domain ID, masking a mismatch
in the original interrupt caller. Signal trampoline tasks use
`pd_id | 0x8000_0000`; `task.context.pd_id` and current-PD identity are the actual
protection-domain identities. Passing a task ID to the new forwarder is rejected.
The parent integration changes the interrupt caller and fallback signal lookup
to use current-PD identity. No TaskContext layout or interrupt return convention
changes are required by this protocol.

The subsequent `DOMAIN_REGISTRY_EXACT_IDENTITY_V1.md` repair removes modulo
lookup/insertion and adds exact registry identity validation. Fault forwarding
keeps its own early ID bounds check.
It also does not alter the existing signal fallback's delivery or termination
semantics.

## Verification

Run `./scripts/pagefault_register_transport_gate.sh`. It checks the TOML contract
against the actual opcode and listen-register mapping, then runs eight native
Rust tests on the actual codec and actual forwarding function. On the Apple
Silicon host: **8 passed, 0 failed**, plus contract PASS.

Coverage includes canonical-address boundaries and the noncanonical hole;
round trips including protection flags and `u32::MAX`; zero/wrong opcode;
domain/error-code truncation; authenticated-caller mismatch; by-value contents
after forwarding returns; task-ID/registry-bound rejection; absent/wrong-kind
slot-2 capability; rejection of the default slot-2 self edge; and propagation of
missing-target/full-ring failures.

The kernel forwarding tests use explicit capability-table and dispatch boundary
doubles. They prove the forwarder delegates appropriately and preserves errors;
they do not constitute real ring concurrency, hardware-fault, mapping, completion,
or wakeup evidence. The gate uses native `rustc --test`, not an alternate kernel
or ISO build. The parent performs the sealed `make full` build, updates ABI/spec
hashes, and runs QEMU integration after all lanes are joined.

## Remaining blockers before activation

`servers/sext` remains unbuilt and unspawned, with no new slot-2 grants. Do not
activate its stale fake-success implementation. A usable pager still needs:

- Boot-sanctioned frame ownership and region authority, zero-before-exposure,
  a privileged mapping interface audited against the SASOS architecture, and
  bounded exhaustion behavior.
- Fault request identity, saved-task binding, authorized completion, replay and
  duplicate handling, and safe scheduler wakeup of the precise blocked task.
- Explicit denial of protection faults and out-of-policy addresses, failure
  containment for absent/dead pagers, and safe last-runnable-task behavior.
- Live first-touch resume, multiple clients, queue saturation, and adversarial
  foreign-completion tests before any demand-paging success claim.

The older stub report's task-specific STOP-FIRST instructions are superseded
for this bounded kernel/ABI repair by the current explicit execution request;
this change does not fold in pager activation or new authority grants.

## Parent integration closeout (2026-10-28)

Sealed build and 40-second QEMU master gate: **GREEN_MASTER**. All four USB
lanes and the strengthened two-boot persistence/rendering gate pass on the same
image. See [the execution report](SEXOS_RELIABILITY_ROADMAP_2026_10_28.md)
for source/image provenance, commands, evidence paths and remaining limitations.
