# SexOS architectural truth: report and execution

Date: 2026-10-28 (Europe/Berlin). Baseline HEAD: `4a040a86`, with existing
uncommitted reliability changes. Those changes are part of the starting state,
not new accomplishments of this mission. Constitutional authority:
`ARCHITECTURE.md`. SexOS remains a single-address-space OS, not Unix.

## Assessment of the suggestion

Adopt the central proposal: give architectural claims stable identifiers,
explicit transition contracts, executable negative tests, source bindings, and
an honest evidence ledger. Use adversarial auditing before runtime edits.
Prioritize security, correctness, determinism, performance, then cleanup.
The model and marketing assertions in the supplied ChatGPT text are not evidence
about this checkout; this report evaluates the engineering proposal from local
source and tests. Three explicitly selected Astra agents investigate scheduler
ownership, authority, and epoch/storage behavior in disjoint lanes.

Several details need correction before adopting the proposed model:

- A runnable task needs exactly one scheduling custodian; a queued task is not
  executing. Ownership must distinguish constructed, queued, attached,
  switching-out and detached tasks from Ready/Running/Blocked/Exited lifecycle.
  Queue publication cannot release a still-active kernel stack.
- The canonical capability graph is immutable after BOOT_DAG lock. CAP_GRANT,
  CAP_REVOKE and dynamic domain creation cannot be casually introduced as
  ordinary runtime transitions. Revocation needs a specified sanctioned
  lifecycle mechanism first; a stale-capability test alone does not supply one.
- `S[n+1] = T(S[n], I[n], C)` is a contract, not proof that the current kernel
  implements it. Every relevant IRQ, time source, device completion and external
  byte would need an explicit recorded input. Hashing a toy model, frame, or
  incomplete subset of state cannot establish complete system replay.
- Millions of generated traces establish only the coverage of that model and
  generator. They do not prove production scheduler interleavings, atomics,
  stack lifetimes, hardware isolation, or crash consistency by themselves.
- PKU-off equivalence concerns authority decisions. Current trusted-but-faulty
  domains and privileged SyncCall execution do not establish hostile-code
  containment. Removing PKU is not a safe replacement backend by itself.
- The sext pager transaction is incomplete. Raw kernel memory interfaces cannot
  simply be repackaged as a pager without explicit sanctioned frame/region
  rights, authenticated completion, exact-VA mapping and safe wake ownership.

## Scope and initial evidence, before runtime repairs

Read the canonical architecture, contribution rules, build/ABI contracts,
current reliability report and the relevant live subsystem sources and
handoffs. This is a targeted source audit, not a claim to have read every
historical file or proved every path in the repository.

| Area | Source-backed initial finding | Required next step |
| --- | --- | --- |
| Scheduler | `unpark_thread` unconditionally stores Ready. A wake of Running can prevent its subsequent requeue; a wake of Exited resurrects it. Wake-before-park and detached wake are independently unresolved. | Model ownership first; reproduce lifecycle corruption; constrain wake transition without publishing attached tasks. |
| PDX arguments | Raw u64 capability-slot syscall arguments narrow to u32 before lookup, so high bits can alias valid slots and the slot-0 bridge. Invalid memory-lend destinations can reach allocation without validating the destination capability-slot range. | Reproduce at the production argument-validation boundary, reject malformed identities before side effects, preserve valid caller ABI. |
| Memory authority | Raw map/allocate/shared-buffer and memory-lend interfaces still lack the complete boot-sanctioned frame/region authority described by the architecture. | Specify and migrate real users before activating sext. Do not claim slot validation completes memory authorization. |
| Epochs/UiIR | The active Rust paths do not contain the documented full epoch/UiIR engine; desktop replay restores only a workspace identifier. Display state uses mutable rendering paths. | Record as open; test a real deterministic production component without claiming full OS replay. |
| Build/proof | There is one sealed image build path and several existing host/runtime gates. Some checks are structural, and scheduler runtime coverage is advisory. | Keep the sealed path; label each check's scope and retain image/source provenance. |

## Remediation dependency graph

1. **Evidence foundation:** six versioned JSON `.sexspec` documents, a strict
   validator, negative validator tests and CI inventory enforcement. Architecture
   changes require review of the pinned constitution hash. Source anchors bind
   the inventory to real code; anchor existence alone is structural evidence.
2. **Security:** full-width capability and destination-slot
   validation before allocation. Then design sanctioned memory authority and
   migrate the actual map/allocation/shared-buffer callers.
3. **Correctness:** constrain invalid wake lifecycle transitions now. The full
   park/wake protocol depends on pending tokens, save-and-stack-release
   ownership, safe detached admission, bounded saturation retry and local
   NoRunnable resumption. Do not shortcut it with direct wake enqueue.
4. **Determinism:** identify actual deterministic production transitions and
   test their boundaries; then design explicit epoch/input capture and compare
   complete observable state under controlled replay. DiskFS proof must name
   precisely which writes/crash points and objects it covers.
5. **Integration:** existing host gates, sealed build, fresh QEMU master proof,
   USB paths and reboot persistence using the same preserved image. No claim of
   bare-metal, SMP, hostile-code security, or complete replay from a boot test.
6. **Later:** pager activation after authority and wake prerequisites; broader
   device/network/application evidence; performance measurements and ports only
   after the correctness dependencies are satisfied.

`spec/*.sexspec` uses JSON rather than an unimplemented custom language.
Each invariant has a requirement, live source anchors, check identifiers,
an evidence status and explicit missing proof. The inventory gate validates
these bindings; separate tests execute the actual behavior. Passing inventory
validation does not turn open architectural obligations into passing proofs.

## Audit-driven implementation

The initial report above was written before runtime repairs. Each lane added
negative tests before its runtime edit, then cross-reviewed another lane.

- **Scheduler lifecycle:** the exact pre-edit wake body, executed with a host
  task stub, failed 6 of 8 assertions. The production helper now uses one
  Blocked-to-Ready CAS. Running, Exited and unknown states remain unchanged;
  no enqueue or ownership transfer is added. Eight real-helper regressions and
  twelve explicitly separate historical/reference ownership models pass.
- **Capability selectors:** the old narrowing operation failed 5 of 7 initial
  assertions. Full-width preflight now validates call/listen/reply/PCI/lend
  selectors and destination slots before dispatch side effects. Nine tests execute production validation, with a tenth source integration
  guard. This is identity
  validation, not new operation authority; slot-0 bridge semantics remain an
  explicit unresolved architectural gap.
- **DiskFS recovery:** production-function tests first reproduced two objects
  aliasing pool block LBA 400 after descriptor commit but before manifest-size
  commit. Recovery now reserves valid extents even for a live size-zero object.
  Cross-review reproduced a second case: stale descriptors in reused empty slots
  could evict a later nonempty owner. CREATE now resets/verifies the free slot's
  descriptor before publishing its new identity; failed DELETE persistence
  restores the live in-memory entry before permitting reuse. Thirteen tests of
  extracted production functions pass, with descriptor I/O, manifest persistence
  and cache invalidation supplied as fixtures. Ambiguous device-write failures
  and previously corrupted images remain outside this proof.
- **Specification/evidence:** six JSON inventories contain 24 architectural
  obligations and transition contracts with source/check/dependency bindings.
  The validator rejects missing or stale bindings, changed constitution, removed
  core obligations, cycles, duplicate identities, unsupported proof claims and
  unsafe log identifiers. Its runner excludes runtime gates, propagates failures,
  hashes source inputs, and replaces old reports before work so interruption
  cannot leave a prior success presented as the current run. Twenty-six negative
  and positive validator/runner tests pass. CI runs the registered host/static
  checks and retains their evidence; the sealed build validates the inventory.

## Fresh runtime finding and follow-up

The first rebuilt image compiled successfully. Its 40-second QEMU observation
passed spawn, clock, capability, scheduler and fault checks, but correctly
returned RED_MASTER: sexinput's first marked HID send was at serial line 2612,
while silk-shell became ready at line 2918. An unmarked synthetic send happened
even earlier. A successful asynchronous enqueue had been mistaken for receiver
readiness; `sys_set_state` is currently a no-op. This pre-existing dependency
became visible under the changed schedule. Evidence is retained at
`/tmp/sexos-truth-runtime-20261028/`; it is a failed intermediate image, not the
final validation artifact.

The follow-up adds an authenticated shell-ready notification over a boot-granted
reverse Domain capability. Sexinput waits before all shell producers, including
synthetic paths, retains incoming USB messages in a bounded FIFO, and drains
those messages before receiving newer input. The shell publishes only after its
actual ready point, retries a full recipient ring from its normal loop, and
cannot grant authority through the notification. The original ordering
requirement remains in force. Exact final checks and results follow below.

## Execution results

The final sealed build and fresh 40-second QEMU master observation pass:
**GREEN_MASTER**. All hard boot rows pass, and every sampled required domain
has task.running observations; the scheduler row retains its advisory policy.
There are no observed fault/panic signatures or scheduler corruption tripwires.
The shell-ready aliases occur at lines 2883–2884, successful notification at
2885, and authenticated input acceptance at 2903. No ordering requirement was
relaxed; the checker additionally requires the boot grant, notification and
acceptance, and rejects input sends before acceptance.

Final image SHA-256:

```
5fe36088ee5a0735fd28c14c5c7ea0533d895f6592ab4789e6b7cf9fb2d7453f
```

The exact image is preserved as
`/tmp/sexos-truth-final-20261028/tested.iso`.
`/tmp/sexos-truth-final-20261028/provenance.json` records the full source
revision, dirty tracked-diff hash, untracked-file hashes, image hash and
`source_image_binding=sealed_build_this_run`. Logs/report/build output are
beside it. Subsequent report-only edits do not change the tested binaries.
The runtime platform is emulated x86_64, QEMU 11.0.2 on Apple Silicon macOS;
this does not imply bare-metal, ARM protected-userland or SMP validation.

Host results:

| Check | Observed result | Scope |
| --- | --- | --- |
| Inventory validation | PASS: 24 obligations, 8 open / 16 partial | Schema/source/dependency consistency; system_proven remains false. |
| Rust host tests | 88 passed | Queue 8; wake helper 8; separate ownership models 12; selectors 10; registry 8; fault protocol 8; display readiness 9; DiskFS 13; shell readiness 12. Includes source wiring checks, not 88 guest executions. |
| Python regressions | 59 passed | 26 inventory/runner and 33 proof/harness regressions, including three new shell-startup evidence checks. |
| Existing static guards | PASS | NoRunnable branch/owner patterns, RSP0 offset, syscall pointer guards. |
| Runtime RSP0 guard | PASS | Correct marker; no observed faults/tripwires in final boot log. |
| Shell syntax / diff whitespace | PASS | Selected changed scripts and full tracked diff. |

Host evidence is in `/tmp/sexos-truth-host-final-20261028/`: per-check logs,
`report.json`, exact source SHA-256 bindings including the extracted DiskFS
message constants, exit statuses and evidence scopes. Registered runtime gates
are excluded from the host runner. CI now invokes this runner and uploads its
evidence alongside boot/input/persistence evidence; the remote CI job has not
been run from this session.

Commands executed:

```sh
python3 scripts/check_architectural_truth.py --run-host --evidence-dir /tmp/sexos-truth-host-final-20261028
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests -p 'test_*.py' -v
GATE_DIR=/tmp/sexos-truth-final-20261028 bash scripts/master_runtime_gate.sh --probe 40 --keep-log
bash scripts/rsp0_regression_gate.sh /tmp/sexos-truth-final-20261028/serial.log
```

The legacy `scripts/audit_invariant_gates.sh --working` was also attempted.
It exits before inspection on this host with Bash 3's
`DIFF_ARGS[@]: unbound variable`. It is not counted as passing. Its hardcoded
kernel/sex-pdx/multi-domain edit restrictions reflect an earlier narrow patch
scope; this mission explicitly authorizes those changes. It is not replaced
with a false architecture proof or bypassed to manufacture a PASS. Full
DiskFS crash/metadata windows, bare metal and exhaustive SMP interleavings
have not been executed by this mission.

Follow-up on the preserved image:

- **USB: PASS**, all four lanes (tablet/contact, no device, keyboard, relative
  mouse). Accepted shell readiness precedes the first marked HID send in every
  active lane. Logs: `/tmp/sexos-truth-usb-20261028/lane_*.log`.
- **Reboot persistence/rendering: PASS.** Document 2 saves 242 bytes and both
  post-reboot loads match length and fingerprint `0xdbc6d3de7f43121f`. The scoped
  Quil text-region check finds 3,139 expected-color pixels. Both boot fault
  scans and the runtime RSP0 guard pass. Evidence:
  `/tmp/sexos-truth-dpg-20261028/{b1.log,b2.log,persistrows.txt,pixrows.txt,final.ppm}`.
  `final.png` is a format conversion for visual inspection. Inspection confirms
  the text is present but also shows overlapping/incomplete desktop surfaces;
  this is content/rendering evidence, not a general desktop-layout or usability
  claim. Fingerprints are not an exhaustive byte-by-byte adversarial proof.
- **Dynamic object lifecycle: PASS**, all ten rows: creation, duplicate rejection,
  enumeration, rename, system-slot protection, reboot survival, deletion,
  updated enumeration, slot reuse and fault scan. It exercises the changed
  CREATE/DELETE paths separately from fixed-document persistence. Raw logs and
  the shared test disk are in `/tmp/sexos_dyo/` (`b1.log`, `b2.log`, `nvme.img`).

```sh
GATE_DIR=/tmp/sexos-truth-usb-20261028 ISO=/tmp/sexos-truth-final-20261028/tested.iso SKIP_BUILD=1 bash scripts/usb_path_gate.sh
GATE_DIR=/tmp/sexos-truth-dpg-20261028 ISO=/tmp/sexos-truth-final-20261028/tested.iso SKIP_BUILD=1 bash scripts/disk_persistence_gate.sh
SKIP_BUILD=1 bash scripts/dynamic_object_gate.sh
```

The dynamic gate uses its existing hardcoded repository ISO and QEMU default
CPU, rather than the master gate's `max,+pku`. The root ISO was hash-checked to
match the preserved final image before the run. This is a separate emulated
runtime configuration, not a PKU-on/off authority-equivalence proof.
The Python run is retained in
`/tmp/sexos-truth-host-final-20261028/python-tests.log` (59 passed).

## Remaining obligations and next engineering work

1. Complete the scheduler pending-wake/park handshake and post-stack-release
   custody transfer before detached wakes, page-fault completion or SMP safety
   can be claimed. The historical failure models remain explicit evidence of
   these open defects; lifecycle CAS does not repair them.
2. Define/migrate BOOT_DAG-sanctioned frame/region rights for raw mapping,
   allocation, shared buffers and lending; remove dynamic unsanctioned genesis.
   Then design request-bound pager completion and exact-VA mapping before sext
   activation. Preserve GLOBAL_VAS and userland memory authority.
3. Design explicit epoch/input capture, real UiIR causal streams and complete
   observable replay. Current desktop workspace restoration and display traces
   do not satisfy that contract.
4. Extend actual crash-A recovery to competing object allocation, regrow and
   byte-exact reboot checks; add delete-crash/reused-slot coverage and explicit
   ambiguous-I/O outcomes. Host fixtures do not substitute for those runs.
5. Preserve stronger protection, real-network, hardware and performance claims
   as unproven until the relevant evidence exists. The README now explicitly
   separates the intended architecture from implementation gaps and marks sext
   as absent from the sealed image.

No commits, pushes, releases or deployments were performed. New files and
changes remain in the shared checkout; pre-existing reliability work was
preserved.

Related new handoffs:

- `SCHEDULER_WAKE_LIFECYCLE_2026_10_28.md`
- `CAPABILITY_SLOT_VALIDATION_2026_10_28.md`
- `DISKFS_ZERO_LENGTH_RECOVERY_2026_10_28.md`
- `SHELL_READY_HANDSHAKE_2026_10_28.md`
