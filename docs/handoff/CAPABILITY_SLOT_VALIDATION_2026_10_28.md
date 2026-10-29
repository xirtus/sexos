# Capability selector validation and authority audit

Date: 2026-10-28. Scope: x86 SASOS register selector validation, preserving
GLOBAL_VAS and existing callers. No commits. Existing working-tree changes
were preserved. This is a bounded input-validation repair, not completion of
the capability authority architecture.

## Reproduced defect and repair

Before the repair, syscall handlers narrowed register-width selectors with
`as u32` before validation. A PDX selector `0x1_0000_0000` reached the slot-0
bridge; `0x1_0000_0005` selected capability 5. Reply target IDs had the same
aliasing. Aliasing an existing capability does not establish possession of a
new capability; it violates exact identity and permits malformed selectors
to perform operations instead of being rejected.

There was also a concrete allocation failure mode. Given a valid Domain
capability, a registered target, length 4096, and available frame/VA/mapping
resources, `SYS_GRANT_MEM_LEND(domain_slot, 4096, 0)` or destination 1025
passed the absent-slot check. The kernel then allocated/mapped a page;
`CapabilityTable::insert_at` silently ignored the invalid destination; the
syscall logged success and returned the VA without installing a MemLend.
Repeated invalid destinations could consume resources without the purported
grant. This path was established by source inspection; no guest exploit or
allocator-exhaustion experiment was run.

`kernel/src/capability_slot.rs` now validates the complete u64 input:

- Capability slots: 1 through 1024 inclusive.
- PDX call/listen: the same slots, plus their existing explicit slot-0 bridge.
- Reply domain IDs: 0 through `MAX_DOMAINS - 1`; registry lookup still checks
  existence. Reserved domain zero behavior is preserved.
- MemLend grants validate both the caller's Domain slot and the destination.

The dispatch preflight runs before any capability lookup, queue operation,
allocation, mapping, or reply insertion. It covers syscalls 0, 28, 29, 43,
50 and 51. Invalid calls return `ERR_CAP_INVALID` with value zero; invalid
listens retain empty output with all payload registers zero; invalid replies
return 1; invalid PCI/MemLend requests return `u64::MAX`. Existing valid ABI
values and capability-kind checks remain in place. `grant_capability` also
checks full width before direct insertion. The ineffective constructor grant
to slot 0 was removed: the one-indexed table already discarded it, and the
slot-0 self-listen behavior belongs to the syscall bridge.

## Regression evidence

Red first: the three production decoder functions initially contained the
existing narrowing operation `Some(raw as u32)`. Running
`bash scripts/authority_slot_validation_gate.sh` exited 101: **2 passed,
5 failed**, including actual `4294967296 -> Some(0)` and invalid destination
`0 -> Some(0)`. This is a reproducible host demonstration of the old decoding
operation, not a boot of an intentionally vulnerable kernel.

After full-width validation and integration, the same gate passes **10 tests**.
Nine execute production decoding/preflight logic. One source integration guard
checks that dispatch rejects and returns before its operation match; it does
not execute the kernel allocator or ring operations on the host. Tests cover
all valid slots/registry IDs, slot zero, upper bounds, high-bit aliases,
invalid grant destinations, valid-call compatibility and dispatch ordering.

Additional commands and observed results:

```sh
bash scripts/authority_slot_validation_gate.sh       # 10 passed
bash scripts/domain_registry_identity_gate.sh        # 8 passed
bash scripts/syscall_user_pointer_hardening_gate.sh   # PASS
bash scripts/rsp0_regression_gate.sh                  # static PASS
bash scripts/scheduler_no_runnable_ownership_gate.sh  # static PASS
bash -n scripts/authority_slot_validation_gate.sh     # PASS
git diff --check                                     # PASS
```

Image build and fresh runtime proof belong to parent integration. Neither
decoder tests nor static guards prove hostile-code containment, secure
memory grants, or a functioning pager.

## Remaining authority gaps, prerequisites, and live callers

Line references identify the working tree at this handoff; function names
remain the stable anchors as integration adds lines.

| Path | Current implementation and prerequisites | Required follow-up |
| --- | --- | --- |
| `kernel/src/syscalls/mod.rs:385` / `MAP_MEMORY` | Any caller can submit a raw PA; successful mapping makes it user RW with pkey 0, without a Memory/DMA capability or ownership check. Pkey 0 is opened by `PkruValue::for_domain`, `capability.rs:186`. | Explicit boot-sanctioned frame/region rights, mapping limits, zeroing and rollback; migrate existing raw DMA callers in `servers/sexusb/src/main.rs:40`, `servers/sexnet/src/main.rs:104`, `apps/sexdrive/src/main.rs:151`. |
| `kernel/src/syscalls/mod.rs:407` / `ALLOCATE_MEMORY` | Global frame allocation from size alone, without caller budget/rights or zero-before-exposure. | Same migration; do not simply disable active USB, NIC and storage callers. |
| `kernel/src/syscalls/mod.rs:519` / `ALLOC_SHARED_BUFFER` | Producer and consumer narrow to u8; only inequality is checked. `ipc/buffer.rs:25` maps using fixed shared key 14, regardless of per-edge authority or registered consumer. | Explicit granted producer/consumer rights and exact identities. Existing caller: `apps/sexdrive/src/main.rs:2830`. |
| `kernel/src/syscalls/mod.rs:535` / `GRANT_MEM_LEND` | A Domain communication capability authorizes a new frame and dynamic MemLend mint. This repair only rejects malformed selectors. | Dedicated memory-grant authority and caller migration. Active sexfiles VFS/diskfs grants feed `apps/sexdrive/src/main.rs:2922` and `:2973` mappings. |
| `kernel/src/capability.rs:116` / `seal`; `kernel/src/init.rs:752` / `RegistryFrozen` | No capability-table seal call exists. The boot phase freezes registry insertion (`ipc/domain_registry.rs:35`), not capability tables; dynamic MemLend remains active. `insert` also wraps at 1024 and `insert_at` replaces occupied entries. | Do not claim immutable genesis. Sealing immediately would panic active grants; first define the boot-sanctioned memory-grant lifecycle and migration. |
| `kernel/src/syscalls/mod.rs:191` / slot-0 call; slot-0 listen | Direct bridge bypasses capability-table lookup. Operations include display information, raw print, init argument and frame-present debug hook; listen directly consumes the caller's own ring. | Explicitly specify bridge authority and migration. Full-width slot validation preserves existing behavior; it does not make this bridge capability-backed. |
| `crates/sex-pdx/src/lib.rs:726` / `pdx_spawn_pd` | Wrapper calls slot 0 opcode 0x10, absent from dispatcher, so it returns invalid. Kernel helper `ipc.rs:168` leads to a stub loading silk-shell and does not itself validate Spawn authority. No active IPC-capability mint site was found. | Document unavailable runtime spawn; do not call this an implemented BOOT_DAG-gated spawn API. |
| `kernel/src/ipc.rs:105,132` / resolve/traverse SyncCall | Given an IPC capability, the callee is a raw function called from the syscall context. Entry at `interrupts.rs:163` opens PKRU=0; no callee PKRU transition exists. | Trusted-but-faulty model only. Native WRPKRU is user-executable on x86. No hostile-code containment claim; a constrained transition needs its own design and proof. |
| `kernel/src/ipc/router.rs:36` / `send_reply` | Any syscall caller can target an existing numeric domain, and an occupied one-deep reply buffer is overwritten. Exact-width checking does not authenticate replies. | Request-bound return authority and explicit saturation behavior before pager completion can use it. |
| `kernel/src/memory/va_allocator.rs:8`; `memory/manager.rs:66,82` | Size arithmetic and range formation are unchecked; frame/VA allocation is in kernel, and mapping failures lack transaction rollback. | Preserve the SASOS design while defining a complete capability-backed memory transaction and eventual ring-3 sext ownership. |

These are implementation gaps against the canonical architecture, not reasons
to introduce UNIX process semantics, per-process page tables, a kernel pager,
or arbitrary fault-driven allocation. Global addressing remains distinct from
authority; the present implementation does not yet enforce every documented
authority invariant.
