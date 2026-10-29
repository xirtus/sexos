# Domain registry exact identity V1

Date: 2026-10-28 (Europe/Berlin).

## Defect and repair

`DomainRegistry::get` and `insert` previously computed `id % MAX_DOMAINS`.
An ID such as 1031 could therefore resolve domain 7, and registration at 1031
could replace domain 7's slot. Duplicate in-range registrations also replaced
the previous pointer. This affected shared identity lookup used by IPC,
replies, signals and MemLend; capability presence alone did not prevent aliases.

The actual registry implementation now lives in
`kernel/src/ipc/domain_registry.rs`, re-exported by `kernel/src/ipc.rs` with the
same public constructor, `get`, `insert`, `len`, and `domains` array.

- `get` returns `None` outside the array, for an empty slot, or when the stored
  protection domain's `id` differs from the requested identity.
- `insert` preserves the existing `RegistryBuild` phase assertion, rejects
  out-of-range IDs, null pointers and mismatched domain identities, then uses
  atomic compare-exchange to publish into an empty slot exactly once.
- Invalid or duplicate insertion panics with a specific registry invariant
  diagnostic. It never clears or replaces an existing registration, including
  when a duplicate supplies the same pointer.
- Domain zero remains representable for legitimate kernel registration. The
  fault notification protocol independently rejects zero as a user fault owner.

Insertion retains the existing trusted-kernel caller contract: a supplied raw
pointer must refer to a fully initialized, permanently live protection domain.
The public array remains available for compatibility; direct mutation can still
violate publication rules, although lookup now refuses a mismatched identity.
Boot phase transitions retain the existing serialized initialization contract;
this slice does not redesign BootController synchronization.

## Verification

`./scripts/domain_registry_identity_gate.sh`: **8 tests passed** on the Apple
Silicon host using native `rustc --test`. The harness includes the actual kernel
module, with only ProtectionDomain identity and boot phase fixtures replacing
kernel-specific dependencies. Registry logic is not duplicated in a model.

Coverage includes valid IDs 0/1/1023, empty slots, ID+1024 and `u32::MAX` lookup
and insertion, null/mismatched pointers, same/different-pointer duplicates,
every non-build boot phase, stored-identity mismatch, and concurrent duplicate
publication with exactly one winner. Re-running
`./scripts/pagefault_register_transport_gate.sh` also passes its contract check
and all eight tests.

This gate does not build a kernel or image. The parent runs the sealed build
and QEMU integration after all lanes join. No allocator, mapping syscall,
capability-grant policy, pager activation or interrupt behavior changes belong
to this registry slice. The pager authority/completion blockers in the fault
transport handoff remain open.

## Parent integration closeout (2026-10-28)

Sealed build and 40-second QEMU master gate: **GREEN_MASTER**. All four USB
lanes and the strengthened two-boot persistence/rendering gate pass on the same
image. See [the execution report](SEXOS_RELIABILITY_ROADMAP_2026_10_28.md)
for source/image provenance, commands, evidence paths and remaining limitations.
