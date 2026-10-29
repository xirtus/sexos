# SexOS and Sex microkernel: reliability roadmap

Date: 2026-10-28 (Europe/Berlin). Baseline: `4a040a86`, clean working tree.

## Decision

The first execution wave repairs display startup ordering, page-fault transport,
domain identity lookup, runqueue pointer transfer, and proof fidelity. The next
investment should be coordinated scheduler wake ownership and capability-bound
memory authority, followed by the actual ring-3 pager. Desktop polish,
additional stub servers, and ARM protection work follow these prerequisites.

“Working perfectly” is a direction, not a defensible release claim. The target
is an explicit set of passing, repeatable behavior and failure-containment
tests on named platforms, with evidence tied to the source and image hashes.

## Research and baseline evidence

Reviewed the canonical `ARCHITECTURE.md`, `CONTRIBUTING.md`, root workspace,
build spec and ABI contract; current build/boot scripts and CI; and the handoffs
listed below. Cross-checked the important findings against live source.

| Area | Baseline evidence | Consequence |
| --- | --- | --- |
| Build | Fresh `make full` succeeds on this Apple Silicon host through the sealed entrypoint. The optional native-host sex-pdx check emits invalid x86 register warnings on ARM. | Keep one build authority; fix the host diagnostic and make dependency/toolchain inputs reproducible. |
| Runtime proof | Historical `MASTER_RUNTIME_GATE_V1.md` reports `RED_MASTER`, with `ORDER_GATE` failing. A fresh 30-second run completes with the same display-readiness failure; all spawn, capability, clock, and fault checks pass. An early read while scanning was initially mistaken for an aborted scanner; no abort was reproduced. | Harden the harness, then distinguish real boot-order violations from incomplete or incorrectly interpreted observations. Never turn missing proof into PASS. |
| Scheduler | `unpark_thread()` only stores Ready. Detached blocked tasks are not enqueued. `WorkStealingQueue::push()` silently drops full-queue insertions. Confirmed in `kernel/src/scheduler.rs` and the Phase-1 ownership handoff. | Design bounded wake ownership, duplicate suppression, and explicit saturation handling before enabling recoverable faults. Preserve TaskContext/IRET layout and the RSP0 ratchet fix. |
| Pager | `servers/sext` is not in the workspace or build. It contains duplicate entry/panic handlers and references nonexistent APIs. `forward_page_fault()` queues opcode zero and a pointer to a stack-local enum. Contrary to the old stub audit, initial rights can put a self-Domain capability in slot 2: absent sext does not always mean an absent slot. | Register-only fault messages need a nonzero opcode and values with a stable lifetime. Reject self-routing as well as missing/wrong-kind pager capabilities. A dispatch repair alone does not implement paging. |
| Fault completion | Faulting tasks are blocked; recovery needs a real completion/wakeup path, per-domain authority, and a mapping interface. | Do not spawn a fake-success pager or grant a nonexistent mapping authority. Specify and test the whole transaction before activation. |
| Protection | The SyncCall audit documents kernel-context execution with unrestricted PKRU; x86 userland can execute WRPKRU. | Current x86 execution is trusted-but-faulty domains. Do not claim hostile-code containment. A secure fast path requires separate design and proof. |
| Storage | DiskFS crash/growth/read work exists, but closeout still asks for multi-object application evidence and interleaved clients. | Prove byte-exact application saves, reopen, reboot persistence, crash recovery, and no client cross-talk. |
| Network | Fresh boot includes `network.sprint.final.runtime.smoke.v1 mode=mock ... fetched=1 status=200 ... ok=0` and a passive sexnet/browser-network-disabled marker. | Mock HTTP output is not a successful real TCP/HTTP exchange. Prove host/guest packet exchange, checksums, retransmission/timeouts and real HTTP with mocks disabled before claiming network usability. |
| Test discovery | `cargo test --locked -p silkbar-model -p sex-object-model` succeeds but executes zero tests. `cargo metadata` confirms the three legacy root `tests/phase*.rs` files are not Cargo targets in this virtual workspace. | Do not count a successful Cargo invocation as exercised behavior. Explicit host gates added by this execution are wired into CI; the legacy scenarios still need a real harness. |
| Input/display | Existing QMP/USB paths and RSP0 regression gates exist; USB hardware, descriptors, and interrupts remain incomplete. | Re-run synthetic input end to end, then record a named hardware matrix. A boot log alone is not desktop usability proof. |
| ARM64 | Latest decode-correction report supersedes older S1POE-positive claims. It records S1POE absent on the tested QEMU and no protected userland. | Preserve development mode and fail-closed required-feature behavior; require fresh model evidence before protection work. |

The configured `/Users/xirtus/sites/microkernel` path does not exist on this
machine. This report covers the Sex microkernel in this repository. The sibling
`Sex_ArmV9` directory contains supporting documents/backups; it is not silently
treated as a second source checkout.

Primary local references:

- `docs/handoff/SCHEDULER_NO_RUNNABLE_OWNERSHIP_V1.md`
- `docs/handoff/SCHEDULER_TICK_PD8_PF_FLAKE_V1.md`
- `docs/handoff/STUB_SERVER_KILL_LIST_V1.md`
- `docs/handoff/PDX_SYNCCALL_GODMODE_SEMANTICS_V1.md`
- `docs/handoff/SYSCALL_USER_POINTER_HARDENING_V1.md`
- `docs/handoff/MASTER_RUNTIME_GATE_V1.md`
- `docs/handoff/DISKFS_LANE3_CLOSEOUT_AND_TEXT_APP_PROOF_V1.md`
- `docs/handoff/SEXUSB_SAFE_HARDWARE_PATH_V1.md`
- `docs/handoff/ARM64_PHASE_6B_DECODE_CORRECTION_AND_PLATFORM_SELECTION_V1.md`

## Execution order and acceptance criteria

### P0 — Establish reliable proof (execute now)

Repair scanner portability/pipefail problems, QEMU failure propagation, and
result provenance. Resolve reproduced ORDER_GATE failures at the actual sender
or receiver, or correct the checker only when its interpretation is demonstrably
wrong. Add adversarial fixtures for missing, premature, duplicate, or malformed
markers. Acceptance: a fresh image produces a complete verdict; negative logs
fail; source revision, dirty state, ISO checksum, command, and duration are
recorded. Historical reports are evidence, not prerequisites that imply success.

### P1 — Repair foundational kernel defects (execute bounded changes now)

1. Scheduler: give wake requests a bounded ownership mechanism; use atomic
   state transitions to suppress duplicates; make saturation explicit and
   recoverable. Test repeated wake, wake while still attached, detached wake,
   full queue, exited task, and queue wrap. Preserve NoRunnable ownership,
   per-core queue ownership, TaskContext offsets, and frame restore behavior.
2. Fault transport: nonzero operation code, `fault_addr/error_code/pd_id` passed
   by value, no borrowed stack pointer, documented register contract, and
   rejection of invalid requests. Preserve capability lookup and absent-pager
   denial. Activation is a later step until recovery is implemented.

Acceptance: targeted regression tests, existing scheduler gates, sealed build,
and fresh QEMU regression all pass. Update ABI/spec hashes when authoritative
ABI inputs change. Do not bypass the hash gate or add an alternate image build.

### P2 — Complete the pager transaction

The deeper execution audit found these prerequisites in live code:

| Interface | Gap to close before pager activation |
| --- | --- |
| `MAP_MEMORY` (syscall 30) | Accepts raw physical addresses without a memory capability, ownership, region, or quota check; maps user RW with pkey 0. |
| `ALLOCATE_MEMORY` (31) | Allocates global frames without caller-specific authority/budget or zero-before-exposure. |
| `ALLOC_SHARED_BUFFER` (40) | Truncates producer/consumer IDs to u8 and compares them, without a memory capability or registered-consumer check. |
| `GRANT_MEM_LEND` (50) | A communication Domain capability is used to authorize allocation and dynamic MemLend minting; dedicated memory-grant authority is missing. |
| `MAP_MEM_LEND` (51) | Does check MemLend kind/write rights, but chooses a new VA rather than the faulting VA; repeated mapping creates aliases. |
| `DomainRegistry` | Baseline lookup/insertion indexes modulo 1024, so invalid IDs alias real entries. This execution adds a separate identity repair. |
| `send_reply` | Raw target ID and one-deep reply overwrite are unsuitable for authenticated, uniquely bound fault completion. |
| Mapping/VA allocation | Range/size validation, checked arithmetic, zeroing, rollback, and release need review. Current implementations allocate/map inside the kernel despite the canonical sext-owned model. |

These are implementation gaps against the project's capability architecture,
not evidence that the documented authority guarantees already hold. Reusing the
unrestricted memory syscalls as the pager API would preserve the bypasses.

Define the boot-sanctioned memory capabilities and ring-3 frame pool, mapping
interface, fault request identity, completion authorization, and wake protocol.
Audit the actual privileged memory operations against the architecture before
choosing implementation. Restrict first-touch allocation to explicitly granted
regions; zero before exposure; reject protection faults, duplicate requests,
invalid addresses, foreign-domain completions, and exhausted budgets. Only then
rewrite sext against real APIs, add it to the workspace/spec/boot DAG, and grant
slot 2 to selected clients. Avoid “allocate on any fault” policy.

Acceptance: first-touch resumes the exact saved instruction; denied faults stay
contained; repeated faults do not leak; cross-domain writes fail; an absent or
dead pager fails safely; the last runnable task cannot fall through into unsafe
user return. Test multiple clients and saturation, then repeat under input load.

### P3 — Prove an everyday application session

Automate launch, focus, typing, editing, save-as of two different objects,
reopen, terminal readback, and reboot readback. Compare bytes rather than only
marker presence. Add interleaved Quil/Spindle clients and crash injection.
Exercise pointer, USB keyboard/mouse/tablet, window movement and close/reopen;
capture serial logs and screenshots. Run a sustained session with no kernel
faults, corrupt-task tripwires, lost tasks, or unbounded retry loops.

Acceptance: repeatable clean runs with image/source provenance and artifact
paths. Keep graphics correctness, input latency, storage durability, and
scheduler liveness as separate measured results.

For network consumers, add a local controlled host endpoint so tests can prove
real bytes in both directions without relying on an arbitrary Internet server.
Require an actual TCP handshake/request/response and clean timeout behavior;
mock responses or packet-build-only markers cannot satisfy this gate.

### P4 — Protection, determinism, ports, and release discipline

- Replace or constrain privileged SyncCall execution with a reviewed transition
  design before stronger isolation claims; test authority denial and PKRU return
  paths. Preserve GLOBAL_VAS and capability mediation.
- Turn snapshot/transform/swap/commit and epoch replay into executable tests.
  Same recorded input and capability graph must yield identical state/frame
  hashes; include forced display interruption and latch recovery.
- Pin tested Rust, dependency lockfile, Limine version/revision and hashes in CI.
  Add boot and host-model regression jobs after build, retaining evidence.
  Normalize packaging timestamps before claiming byte-for-byte ISO
  reproducibility; a fixed build trace alone does not prove identical images.
- Reconfirm ARM feature IDs on the actual available platform. Do not infer
  protected execution from an advertised CPU name or an older handoff.
- Publish a hardware support matrix and reproducible benchmarks. Keep untested
  hardware, hostile-code security, and full replay explicitly unproven.

## Agent assignments for this execution

| Lane | Model | Scope |
| --- | --- | --- |
| Scheduler correctness | Astra | Analyze wake/runqueue ownership; implement and test a bounded safe repair if feasible without changing context ABI. |
| Page-fault protocol | Default model | Repair by-value transport and its contract/tests; assess sext prerequisites; do not activate an incomplete pager. |
| Runtime proof | Default model | Repair gate completion and evidence fidelity; add negative checker tests; identify boot-order causes without weakening checks. |
| Integration | Parent | Resolve overlaps, build through the entrypoint, run fresh proof, review changes, and append exact outcomes and remaining work. |

All agents share this checkout and have disjoint file ownership. No publishing,
release upload, or broad architecture replacement is included. Kernel work is
authorized by the user's request to execute this microkernel repair plan;
historical task-specific STOP-FIRST notes are not treated as a new approval
requirement for routine fixes within that request.

## Concrete follow-up after this execution

1. **Display lifecycle:** the startup handshake is now implemented and passes
   the runtime ordering gate. Extend it to restart/epoch identity and later
   display failure before claiming continued availability. Keep the current
   authenticated notification, bounded deferred work and true readiness point.
2. **Task ownership:** introduce pending wake tokens independently of lifecycle
   state. Release an outgoing context only after its final save and after
   leaving its stack. Coordinate timer, page-fault, yield, and bootstrap paths;
   cover last-runnable/NoRunnable wake locally. Do not enqueue a still-attached
   task merely because its state says Ready. Reject Exited and duplicate wakes.
3. **Memory authority:** migrate existing users of raw mapping/allocation to
   explicit frame/region rights, then implement exact-VA mapping and authenticated
   fault completion. Activate sext only after those tests pass.
4. **Application proof:** complete the two-object Quil/Spindle session, byte-exact
   reboot recovery, and interleaved-client tests with screenshots and raw data.

The queue replacement deliberately fixes pointer transfer rather than pretending
to finish step 2. It adds about 512 KiB of static storage across 128 queues.
Its compatibility `push` stops the affected core on insertion failure instead
of silently dropping a task; `try_push` exposes recoverable ownership to future
callers. This is fail-stop insertion, not graceful overload recovery. Reserved
head progress relies on current non-preemptible kernel scheduling behavior;
future kernel preemption/IRQ queue dispatch needs explicit discipline.

## Execution results

The baseline `RED_MASTER` is now **GREEN_MASTER** on a fresh 40-second QEMU boot
of the sealed build. This closes the observed startup-order defect; it does not
claim that paging, hostile-code isolation, SMP task ownership, full replay, or
general hardware support is finished.

Implemented:

- Capability-mediated display readiness after real renderer initialization,
  with authenticated versioned notifications, FIFO retention during startup,
  independent notification retries, and retained essential startup operations.
  The first-send marker now records successful enqueue rather than an attempt.
- Register-only page-fault notification with nonzero opcode, copied values,
  caller/domain binding, valid Domain target checks, and rejection of the
  default slot-2 self capability. Fault dispatch uses the actual protection
  domain ID rather than assuming task ID equals domain ID. Sext remains off.
- Exact registry indexing and identity checks; invalid or duplicate boot
  registration cannot alias or replace a live domain.
- Bounded MPMC queue slot ownership and explicit insertion failure. The unsafe
  shortcut of enqueuing a still-owned task was deliberately not introduced.
- Honest runtime evidence: actual kernel fault signatures, process/window
  failures, missing/duplicate/malformed proof, source/image provenance, isolated
  reports and stale-artifact cleanup. Removed premature duplicate sexfiles ready.
- Bash-3-compatible headless USB proof. Persistence markers now identify the
  document and fingerprint its actual committed bytes; required screenshots
  fail closed on missing, malformed, truncated or undersized images.
- Pinned Rust `nightly-2026-07-22` and Limine revision
  `ca3a5b20d2501e3819983c5cf838c70c30ee532a` (v7.13.3). Cargo build/check uses
  `--locked`; ABI snapshots cover both new protocol modules. The ABI guard
  cross-checks x86 instead of trying to assemble x86 registers on the ARM host.
- CI now runs the explicit host gates, sealed build/boot, USB lanes and reboot
  persistence, retaining evidence. The updated GitHub job itself has not been
  executed from this local session; runner/QEMU differences remain to validate.

Host validation: **63 tests pass**: queue 8, fault transport 8, registry 8,
readiness 9, and Python proof/harness regressions 30. Existing NoRunnable,
RSP0 and syscall-pointer static gates pass. Shell syntax and diff checks pass.
The separate Cargo model command compiled successfully but ran zero tests;
it is not included in the 63.

Final image SHA-256:

```
7f6cf420c7f9f05b1376f5889fb6d83e31f68e5a763071d7c8e9876c2b886dce
```

Integration command (performs the sealed build internally):

```sh
GATE_DIR=/tmp/sexos-final-20261028 bash scripts/master_runtime_gate.sh --probe 40 --keep-log
```

All master rows pass, including ORDER_GATE; SCHED_GATE retains its pre-existing
advisory status. The ready marker is at log line 2758, notifications at 2759–2760,
client acceptance at 2764/2784, and successful startup sends at 2806/2856.
The checker still labels activity-dependent unobserved edges UNOBSERVED.
`/tmp/sexos-final-20261028/provenance.json` records
`source_image_binding=sealed_build_this_run`, dirty source/diff fingerprints,
the exact ISO hash, command and observation result. The report and build log
are beside it. Later documentation/CI edits do not change the tested binaries.

The exact image is preserved as `/tmp/sexos-final-20261028/tested.iso` for the
follow-up gates. All four final USB lanes pass, including tablet contact/drag,
no device, keyboard and relative mouse. Logs:
`/tmp/sexos-usb-final-20261028/lane_*.log`.

Final fingerprint/reboot proof: **PASS**. Document 2 saves 242 bytes and both
post-reboot loads match length and fingerprint `0xdbc6d3de7f43121f`.
Spindle enumeration, Linen disk-open intent, Quil receipt/load, clean fault
scans on both boots, and the RSP0 regression gate pass. The required screenshot
contains 3,139 expected text-color pixels in the Quil region. These are content
fingerprint and rendering checks, not exhaustive byte-for-byte or adversarial
durability guarantees. Raw evidence is in `/tmp/sexos-dpg-final-20261028/`:
`b1.log`, `b2.log`, `persistrows.txt`, `pixrows.txt`, and `final.ppm`.

Both final follow-up gates use the preserved image above:

```sh
GATE_DIR=/tmp/sexos-usb-final-20261028 ISO=/tmp/sexos-final-20261028/tested.iso SKIP_BUILD=1 bash scripts/usb_path_gate.sh
GATE_DIR=/tmp/sexos-dpg-final-20261028 ISO=/tmp/sexos-final-20261028/tested.iso SKIP_BUILD=1 bash scripts/disk_persistence_gate.sh
```

The runtime platform is QEMU 11.0.2 on Apple Silicon macOS; these are emulated
x86_64 results. No bare-metal or ARM protected-userland result is implied.

Detailed implementation notes:

- `docs/handoff/DISPLAY_READY_HANDSHAKE_2026_10_28.md`
- `docs/handoff/DOMAIN_REGISTRY_EXACT_IDENTITY_V1.md`
- `docs/handoff/PAGEFAULT_REGISTER_TRANSPORT_V1.md`
- `docs/handoff/SCHEDULER_QUEUE_AND_WAKE_BOUNDARY_2026_10_28.md`

No commits, pushes, releases or deployments were performed. Changes remain in
the shared checkout for review. Remaining P1 wake work, P2 memory authority and
pager recovery, multi-object/client storage proof, real networking, stronger
protection, epoch replay and physical hardware validation remain substantive.
