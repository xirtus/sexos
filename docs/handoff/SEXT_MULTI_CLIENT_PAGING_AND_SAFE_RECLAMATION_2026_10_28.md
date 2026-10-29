# Sext multi-client paging, task-exit cleanup and desktop growth

This wave extends the previous bounded pager to two clients and safe frame reuse,
then adds cleanup after task completion or pager rejection. It also fixes a real
Linen document-opening defect exposed by reboot testing. Work remains within one
`GLOBAL_VAS`, immutable BOOT_DAG capabilities and sex-pdx authorization. Physical
APs remain parked. The architectural constitution is unchanged.

## Implemented behavior

Kaleidoscope (domain 15/key 15) and Bell (domain 10/key 10) each receive a distinct
eight-page demand range. Sext (domain 14/key 14) owns one shared eight-frame pool and a
four-page quota per client. Static mapping capabilities occupy sext slots 36/37;
its pool is 34 and each client's demand capability is 35. Boot reserves backing,
VA and intermediate page tables, removes all 16 target leaves, and seals the
capability tables before admission. Runtime paging allocates no kernel frames,
VA, page tables or capabilities.

Sext routes each fault by authenticated client identity and retains independent
in-flight work, quota and release state. A retry for one client cannot replace
its frame or block the other client's policy step. Selected frames are zeroed
through the pager's authorized alias before exposure. The kernel verifies the
whole 4096 bytes and performs only the authorized leaf transition.

Explicit release uses opcode 0x104 and a nonzero monotonic request ID. Sext binds
the request to the original committed fault token, invokes reclaim 60, and frees
only after the exact old leaf is cleared and locally invalidated. The last
per-client request/result prevents a delayed duplicate from looking up and
reclaiming a replacement at the same VA. Replies authenticate the actual sender
and carry the request ID. Old commit/reclaim retries cannot affect a new owner.

## Cleanup after task exit

Self-only internal operation 62 finishes the current scheduler task and reuses
the existing save/switch/custody path. It accepts no foreign target selector.
The old sex-rt exit shim now forwards to this operation rather than colliding
with reclaim 60. Pager rejection and terminal-fault exits use the same ownership
state observed by cleanup.

Two fixed boot exit watches pin the original main-task pointers. On exit intent,
the watch wakes sext so a last-runnable exiting task can find a replacement.
It sends an actionable kernel-origin hint only after `Dead`, no CPU owner and
an inactive trampoline. A full pager ring retains retry state; an accepted
notice is not duplicated. The hint confers no memory authority.

Cleanup 61 requires the pager's actual immutable mapping and pool capabilities.
It verifies exact dead-task identity and custody, preflights all eight leaves, and
retires the dead task's fault transaction before any PTE changes. It includes
published but unfinished bindings. Every owned leaf is cleared and invalidated
before any ledger entry becomes reusable. The range then closes permanently;
future commits and fault queries cannot reactivate it. Exact cleanup retries
return the cached original frame bitmap/task ID without touching new owners.
Sext releases only its current custody for that target, once; it never frees a
replayed bitmap after another client has acquired those indices.

```mermaid
flowchart LR
  A[Task finishes or is rejected] --> B[Exit intent wakes pager]
  B --> C[Stack custody released]
  C --> D[Dead task hint]
  D --> E[sex-pdx authorizes cleanup]
  E --> F[Preflight all leaves and fault state]
  F --> G[Clear and invalidate all old leaves]
  G --> H[Close range and release ledger]
  H --> I[Sext returns frames once]
  I --> J[Zero and reuse for surviving client]
```

Statuses are 0 success, 1 invalid, 2 retry, 3 denied, 4 exhausted and 5 target still
live. Cleanup returns status/bitmap/task ID in RAX/RSI/RDX and zeroes remaining
outputs. Exit hints use opcode 0x105; the bounded enrolled-client cleanup wait
uses 0x106 and grants no new mapping authority. The contract and ABI hash include
the changed transport and paging interfaces.

## Desktop behavior corrected

The initial two-boot storage run restored the correct 242 document bytes and
fingerprint, but failed Linen→Quil opening. Shell's synchronous Linen fetch
accepted any service's reply as object metadata. A foreign value became a second
object with ID 3, so keyboard selection opened the wrong item. The guest also
recorded a foreign caller 8 reply during this fetch.

Reply delivery now captures the actual sending domain rather than placeholder 1,
and a full one-entry reply slot rejects without overwriting its earlier value.
Sext retries its ACK. SilkBar authenticates Bell's actual sender 10. This improves
provenance and retention; the legacy target-directed reply path is still not a
return capability or a fully correlated invocation protocol.

Linen's shell projection accepts snapshot replies only from Linen 7. It stages a
complete bounded replacement, rejects duplicate/invalid IDs and incomplete names,
keeps the previous view while fetching, and preserves selection when that object
still exists. A refresh cannot expose a partially rebuilt object table. The
[desktop plan](DESKTOP_GROWTH_PLAN_2026_10_28.md) sequences the next visible gains:
reliable object opening, responsive storage/input, app lifecycle, shared visual
language and durable sessions.

## Evidence and retained artifacts

The first multi-client image failed at the first release because the legacy
reply sender was placeholder 1; it is retained as rejected evidence under
`/tmp/sexos-multi-candidate-20261028/`. The subsequent normal and quota images
passed the strict paging checker. A separate storage regression exposed the
Linen defect above; that failed run remains under
`/tmp/sexos-multi-persistence-20261028/`. Missing stages remain failures.

| Evidence | Result |
|---|---|
| `/tmp/sexos-multi-verified-20261028/` normal sealed build, full 50s | GREEN_MASTER;9 recoveries, 4 releases, 3 cross-client reuses, 1 same-address reuse |
| `/tmp/sexos-multi-quota-verified-20261028/` declared quota build, full 50s | GREEN_MASTER; exact fifth-page rejection, no rejected-task resurrection, continued clock IPC |
| `/tmp/sexos-multi-usb-20261028/` | All 4 USB lanes PASS |
| `/tmp/sexos-multi-dynamic-20261028/` | All 10 object lifecycle rows PASS over 2 boots; strict multi-client paging also PASS without PKU |
| `/tmp/sexos-task-exit-verified-20261028/` ordinary self-finish, full 50s | GREEN_MASTER ; 4 exact dead-client leaves removed, range closed, 3 survivor reuses with retained neighbor; repeated cleanup preserves new owners |
| `/tmp/sexos-quota-cleanup-verified-20261028/` quota-rejection exit, full 50s | GREEN_MASTER and strict cleanup checker PASS; exact fault rejection followed by full dead-client cleanup and surviving-client reuse |

The self-finish proof has 12 actual fault recoveries: 9 before termination and 3 in
the surviving client. The quota proof additionally rejects one in-range pending
fault. Both close 4 resident bindings and preserve Bell's held page. Host execution
of the actual paging module additionally covers unfinished publication, full-PTE
preflight before side effects, task mismatch, custody retry, closed-range replay,
read-only mappings, hardware A/D bits, and 1000 reuse cycles. Sensitivity mutations
that remove preflight or closure correctly fail those tests.

Final verified evidence:

| Evidence | Result |
|---|---|
| `/tmp/sexos-desktop-final-20261028/` normal sealed build and full 50s | GREEN_MASTER; strict multi-client, memory-authority and scheduler-custody checks PASS; compiled stack-release/timer PKRU and optimized client helper checks PASS |
| `/tmp/sexos-desktop-final-host-20261028/` | All 24 registered host/static gates PASS; 267 Rust test executions; 251 source/harness/dependency bindings match the final tree |
| `/tmp/sexos-desktop-final-python-20261028.log` | 145 Python regressions PASS, including malformed trace rejection and profile restoration tests |
| `/tmp/sexos-desktop-final-usb-20261028/` | Tablet, absent-device, keyboard and relative-mouse lanes PASS |
| `/tmp/sexos-desktop-final-persistence-20261028/` | Both boots PASS; 242 saved bytes restored twice with hash `0xdbc6d3de7f43121f`; correct Linen→Quil document identity, RSP0 checks and 3139 matching text pixels |
| `/tmp/sexos-desktop-final-dynamic-20261028/` | All 10 object lifecycle rows PASS over 2 boots; strict multi-client paging PASS on both default-CPU boots without PKU |

The final normal ISO is `2bf3b15f3f7c9238687fab6e97c0d4d7d9b30e24104d2352e62f90883b1fa7d5`.
Its kernel is `aa4ee413d5f7c32525e0801fa0bbc40d961162780d4ad33105403a3b9d3cf507`.
The ordinary-finish ISO is `130b8c62814c392cfe3e07d84fccb9465738fc2dfd57d66a0f5436f93c60fe2f`;
the quota-cleanup ISO is `e2f696f49c38467b6f3ca37fcc184f2b9c8ec7469c2100954e130cee6c37228b`.
All three use the same kernel. The root ISO is the verified normal image, and the
kernel, Bell and Kaleidoscope declared rustflags are restored to empty. Quil's
existing declared persistence-proof cfg remains unchanged.

Linen's 9 production-function host tests cover foreign replies, publication,
failed-fetch preservation, duplicates, complete names and selection changed by
input during the fetch. Temporary source mutations reproduce both foreign-reply
acceptance and stale selection, and fail those tests. Cleanup checker tests reject
missing custody, wrong generations, incomplete flushes, changed task identities,
bitmap replay errors and ended-task resurrection.

CI now includes both declared cleanup profiles through
`scripts/paging_cleanup_profiles_gate.sh`, then rebuilds/verifies the normal image.
The orchestration's three host tests exercise normal restoration, failed-build
restoration (including the original ISO) and preservation of custom flags.
The real profiles above were built and checked locally through the sealed master
gate; remote CI has not been executed. Native macOS Bash could not run the existing
associative-array root-allowlist script; its same tracked-file allowlist was
checked directly and passed.

Three Astra lanes handled kernel mechanism, scheduler/fault custody and sext
policy/adversarial review; parent integration handled guest evidence, the desktop
fix and the growth plan. Changes are uncommitted.

## Limits

Cleanup currently covers demand-paging leases for two static main-task ranges.
It does not free ELF mappings, stacks, driver DMA, capabilities, display surfaces
or arbitrary service objects. The old domain/task incarnation is not restarted.
The trusted pager retains its authorized alias. There is no autonomous eviction,
expandable frame pool, physical SMP shootdown or full epoch/replay proof. Fault
recovery remains limited to eligible nonpresent user data faults; protection and
instruction-fetch faults do not recover. CI wiring and local guest evidence do
not establish remote-CI execution or hostile-code containment.
