# Scheduler custody and capability-backed memory authority

Date: 2026-10-28 (Europe/Berlin). Continuation of architectural-truth work.
Starting HEAD `4a040a86`, with the previous verified wave uncommitted. Its
changes are preserved. The canonical SASOS architecture remains unchanged:
one GLOBAL_VAS for addressing, sex-pdx for authority, optional PKU enforcement,
BOOT_DAG capability genesis, no per-process page tables or Unix process model.

## Execution order and acceptance

First complete scheduler wake custody across the actual runtime paths. Then
migrate runtime memory users to explicit boot-sanctioned region capabilities.
Three Astra lanes own the runtime scheduler, atomic protocol/adversarial tests,
and memory design/migration respectively; integration owns ABI, gates, compiled
inspection, sealed builds and guest evidence. Cross-review is required before
accepting either stage. Host models are not guest/SMP proofs.

Scheduler acceptance:

- One atomic owner/lifecycle/pending-event word; no independently raced mirrors.
- Constructed tasks require explicit admission. Zero-RIP trampolines are never
  activated by a generic wake. Queued/admission/dispatch/attached/switching-out
  custody transfers admit at most one execution owner.
- Wake-before-park is retained; duplicate wakes coalesce; exit is terminal;
  queue-full/contended publication retains retryable task custody.
- Outgoing context is fully saved before switch, and publication occurs only
  after RSP has left its stack. The callback runs before narrowing PKRU.
- NoRunnable retains the current owner; a pending wake can resume it locally
  without a second queue token. Fault waits cannot be released by ordinary
  IRQ/reply events; no failed pager dispatch may retry an unresolved fault.
- Timer, yield, page-fault and bootstrap callers use the same handoff. TaskContext,
  GPR/IRET offsets and the RSP0 frame-top invariant remain pinned.
- Real self-inbox waiting and accepted Domain enqueue wakeups exercise detached
  waits in the normal image. Syscall 53 has no cross-domain wake target.
- Host adversarial traces/races, compiled switch inspection, sealed build,
  fresh QEMU boot/input/persistence checks and targeted wake observations pass.

Physical AP scheduling is not silently enabled: the existing AP entry parks,
and per-core TSS/GDT bring-up remains a separate prerequisite. A multi-vCPU QEMU
boot alone cannot prove several cores actually schedule protection domains.

Memory acceptance:

- Runtime does not mint new capabilities or infer authority from a raw PA,
  target ID, pkey, caller whitelist or user-supplied region descriptor.
- Bootstrap grants immutable, bounded, zeroed DMA/MMIO/shared region views.
  Runtime resolution checks the actual caller capability's rights and bounds;
  it neither allocates frames nor mutates GLOBAL_VAS.
- Driver-local byte suballocation stays inside an already-assigned region.
  Storage uses reusable DMA backing; timeout quarantines it until a real,
  separately proven device reset makes reuse safe.
- Existing block-lend and frame buffers become boot-bound paired views;
  capability tables seal only after the final static boot grants.
- All active callers migrate before obsolete raw APIs are denied. No stub pager
  is activated and no kernel allocation policy is disguised as sext authority.
- Negative rights/range/identity tests, caller migration, sealed build and fresh
  runtime gates establish the implemented slice. Real demand paging, hostile
  native-code containment and complete epoch replay require separate evidence.

## Findings before integration

The prior lifecycle CAS did not transfer scheduling custody or retain pending
events. Timer scheduling published outgoing task pointers before handler saves;
yield published them before stack departure. Page-fault idle waited for Running
while generic wake wrote Ready. Syscall entry also left IF/DF/TF/AC unmasked,
allowing interrupts in the GS/RSP transition and violating Rust ABI entry needs.

Memory review found unbounded per-operation bounce-buffer allocation in
sexdrive. A bounded bump pool alone would regress sustained persistence.
Runtime PCI BAR mapping also needs migration; removing raw syscalls 30/31 while
leaving fresh runtime mappings in syscall 43 is not a complete mapping boundary.

## Results

### Accepted scheduler slice

The actual scheduler now uses one atomic custody/lifecycle/pending-event word.
Task publication follows the saved context and physical stack departure; the
switch callback verifies stack bounds before admitting the old task. Timer,
yield, page-fault and ordinary inbox waits use this protocol. An unresolved fault
is distinct from a message wait. Queue pressure retains retryable custody.

Syscall 53 waits only on the current domain's inbox. Accepted asynchronous PDX
enqueues publish their payload before waking the recipient. Constructed tasks
require explicit activation, and wake events cannot activate a zero-entry
trampoline. The timer stub also preserves entry PKRU on ordinary IRQ returns;
syscall entry masks IF, DF, TF and AC before the GS/RSP transition.

The first accepted normal scheduler image is retained in
`/tmp/sexos-owner-accepted-20261028/`: sealed build and 40-second master boot PASS,
compiled stack-release and timer-PKRU instruction gates PASS, observed park,
detached wake, post-stack release and subsequent user entry PASS. Its kernel
SHA-256 is `93af95690a66aa456736230a554233a522cc303182919d2e9a42139720544385`.
The earlier declared `scheduler_wake_proof` image retained in
`/tmp/sexos-owner-proof-20261028/` additionally parked the final runnable task,
held 13 queue tokens, woke from a real timer, resumed the same owner and restored
all 13 tokens. A final integrated rerun is recorded below.

The ownership host suite passes 22 tests, including 517,744 finite event traces,
concurrent lifecycle/publication exercises and 20,000 payload transfers. These
are bounded host observations, not an exhaustive memory-model or physical SMP
proof. APs still park; per-core TSS/GDT scheduling and remote wake IPI latency
remain separate work.

### Memory integration and rejected candidate

Runtime region syscalls 54/55 resolve immutable grants from the actual caller's
capability table. Full-width selectors/rights, checked nonempty extents, permission
subsets and zeroed error outputs precede disclosure. Ordinary sharing discloses
no physical address. Syscall 43 queries a probed, premapped PCI BAR0 grant. The
old raw interfaces 30/31/40/50/51 deny requests, and the obsolete shared-buffer
allocator module is unexported.

BOOT_DAG now reserves and zeroes fixed DMA backing (USB/NIC 256 KiB each, drive
512 KiB), pairs a 4 KiB block region and a 3 MiB frame region, creates each
holder's own pkey alias, and seals capabilities before task admission. Display's
frame alias is read-only; all these region aliases are NX. This does not establish
a new live frame-rendering path: display must resolve its own alias before future
use, and the current UI render path remains separate.

PCI probing preserves W1C status and restores BAR/command configuration. Boot
frame reservations exclude whole backing intervals from later page-table frame
allocation. A second review found the old metadata cursor shortcut could overwrite
metadata on fragmented memory maps; actual placement is now reserved, and free
lists are published only after metadata initialization. Collision fails boot
before exposing or zeroing another owner's backing. The general buddy allocator
remains a prototype; these checks do not constitute its complete proof.

Driver migration replaces raw allocations/maps with bounded local reservations.
Normal storage requests reuse two 4 KiB leases; a validated completion releases
DMA custody, while an unresolved submission quarantines it and disables I/O.
Static block access authenticates the PDX caller against the boot-bound peer.

The first integrated candidate in `/tmp/sexos-memory-proof-20261028/` is explicitly
**rejected**: scheduler last-runnable proof passed, but the shell's first 640-byte
allocation failed. The shared `sex-rt` heap still issued unsuffixed raw 31/30
syscalls, which the initial driver scans missed. The deny policy correctly held.
The runtime now uses a bounded reusable arena inside its existing
ELF backing; the regression guard scans `sex-rt` as well. The boot negative
query test also now uses sexfiles' actual Domain route for mandatory wrong-kind
checks, rather than conditionally skipping a nonexistent drive-to-shell route.

The 64 KiB ELF arena uses the existing linked-list allocator with real
deallocation. A shared lock protects one-time initialization and allocator state;
repeated initialization cannot reset live bytes. Eight production allocator tests
cover the shell's first 640-byte request, 100,000 allocate/free cycles, alignment
through 4096, exhaustion/coalescing, duplicate initialization and concurrent use.
This subdivides memory already loaded for the domain; it creates no frame or
mapping authority. Exhaustion returns null instead of requesting ambient memory.

Normal sexfiles startup also performs three non-destructive *guest syscall*
checks: absent slot, wrong capability kind and unauthorized DMA address rights.
It asserts all three denials before readiness. Optional disk proof suites remain
optional; the normal memory gate does not pretend those destructive suites ran.

### Final evidence

The corrected declared scheduler-proof build in
`/tmp/sexos-memory-proof-fixed-20261028/` passes the sealed build and full
45-second master window, compiled stack-release/PKRU gates, guest ownership gate
with `--require-last-runnable`, and guest memory authority gate. Task 8 was parked
with 13 held tokens, awakened by the timer, resumed under its original owner and
restored all 13 tokens. This run includes both the timer repair and memory/heap
migration, superseding the narrower earlier scheduler proof. Proof kernel SHA-256:
`8cf2fc664865a55ca7c4b5257f913ac93f61919f81d8b86409f42fd2a9c2d466`.
Proof ISO SHA-256:
`e5f4dc886563c6fe9b1b9f5a81bc2f2eab693b2e379bca8e70921d05e8999fa7`.

The normal build has `rustflags = ""` for the kernel. Its retained image and
build/provenance/logs are in `/tmp/sexos-memory-final-20261028/`. The sealed
build and full 45-second master window pass, as do both compiled checks and
the normal guest ownership/memory gates. Normal kernel SHA-256:
`b33b1a7f28c5fe2e36e858dd855db080c3f3e60918d4863ee884292dba0305ff`.
Normal ISO SHA-256:
`b29cb16b129e61eb12bfb3fd2efd9b96709b855a7620cac1f4fd29ccbd7ff51e`.

Sixteen registered host/static gates pass with **172 Rust test executions**
(some intentionally repeat shared helper suites), plus NoRunnable/RSP0/user-pointer
source checks. The retained report in `/tmp/sexos-accepted-host-20261028/` binds
205 source/harness/dependency files by SHA-256. All **88 Python regressions** pass;
their output is `/tmp/sexos-accepted-python-20261028.log`. In particular, evidence
checker mutations reject missing wake stages, publication/PKRU ordering defects,
wrong frame-role rights and forged shared-buffer peer observations.

All four USB lanes pass on the normal image: tablet, no device, keyboard and
relative mouse (`/tmp/sexos-memory-usb-20261028/`). Two-boot persistence also
passes (`/tmp/sexos-memory-persistence-20261028/`): both disk publications,
Quil save/reload with matching bytes and fingerprint, disk-open intent, document
load/render observations, RSP0 checks and fault scans. The observed saved document
was 242 bytes with fingerprint `0xdbc6d3de7f43121f`.

The dynamic object gate passes all ten rows: create, duplicate-name rejection,
list membership, rename, protected system slot, survival across reboot, deletion,
updated list, slot reuse and fault scan. Logs and image binding are copied to
`/tmp/sexos-memory-dynamic-20261028/`. The root ISO hash was checked before and
after this gate and matches the normal sealed image above. This legacy gate uses
QEMU's default CPU; its guest reports PKU unsupported and still passes the same
startup region-query denial checks. That is useful additional observation, not
a complete on/off authority-equivalence or native-code containment proof.

Validation used QEMU 11.0.2 on the local macOS host. The proof image uses a
declared kernel cfg, while the accepted normal image does not. No runtime code
changed between its build and these input/storage gates. Source/evidence-checker
metadata and reports were subsequently updated without rebuilding the image.
The heap host gate's optional argument handling was corrected for macOS Bash 3;
the accepted host log contains all eight allocator test results.

The root `Cargo.lock` was previously ignored even though the sealed build uses
`--locked`. A root-only ignore exception now makes the exact dependency lock part
of the reviewable changes; nested package lockfiles retain their existing ignore
policy. CI now runs registered host/static gates, a real sealed build/guest boot,
compiled stack and timer checks, guest ownership/memory checks, USB and persistence
gates with retained artifacts. This describes configured CI; the executions above
were local and are not a claim of a completed remote CI run.

### Remaining architectural work

The kernel boot allocator/mapper is still explicit bootstrap debt. No sext runtime
frame-acquire/commit service, recoverable demand-fault mapping or kernel-policy-free
pager has been implemented. Memory view authorization does not prove native
WRPKRU containment, DMA/IOMMU confinement or privileged SyncCall safety. The APs
still park; actual multi-core execution needs per-core TSS/GDT, explicit kick
policy and physical concurrent evidence. Complete epochs/input recording and
system replay remain open. None of these obligations is marked proven in the
24-obligation `.sexspec` inventory (8 open, 16 partial).

The source changes remain uncommitted; existing work from the previous wave is
preserved. No commit, push or canonical architecture change was performed.
