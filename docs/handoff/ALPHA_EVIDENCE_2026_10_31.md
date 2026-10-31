# Replayable sustained-alpha evidence

31 October 2026. The alpha runner now preserves every verified capture under
a unique sequence/name, with the exact serial-prefix hash, screenshot hash,
checker inputs and owned-region proof. Repeated soak phases no longer overwrite
earlier screenshots. The same registry covers minimized Quil erasure, automatic
Linen lists and the third BOOT-reserved Quil incarnation.

The production image is unchanged from `73c06c82` and the status-bar qualification.
The new frozen runner cohort records 574 identical production inputs and 569
runner inputs. Its explicit q35 RTC/NIC/resolution configuration and complete
source binding are recorded before guest launch. No guest ABI, capability,
address-space, scheduler or rendering behavior changes in this milestone.

## Changes

The normal Quil workflow still exercises real pointer and keyboard input,
find/replace/undo, two named documents, immutable saves with a newer edit,
resize/minimize/restore, mixed Linen/Spindle traffic and exact BOOT successor
recovery/retirement. The runner's read-only `--check` path never launches QEMU.
It replays every capture from its immutable prefix, including the status-bar
cell, then checks terminal disk bytes and lifecycle receipts.

Final document recovery now runs after the owned QEMU exits. Seed and terminal
disks are compressed and hashed with the raw evidence closure. A failed or
interrupted attempt remains diagnostic; it cannot acquire a complete registry
or become accepted through replay. The original raw files are not rewritten.

Version2 separates an ordinary Close attempted during a pending save from the
actual state at confirmed terminal Close. A declined pending save needs exact
revision/snapshot receipts and the unsaved-work guard. Terminal pending0 needs
positive completion of that exact immutable save and is labelled
`safe_completed_save_close`; it leaves **Quil pending-I/O terminal teardown**
unproved. Terminal pending1 retains its separate teardown meaning. The older
version1 criterion still requires actual pending terminal receipts.

Each active cycle has unique IDs, ordered raw-serial windows and measured
active/idle durations. Replay joins its edit and exact save bytes/hash/object,
Spindle Stat/echo/history, Linen refresh and two distinct input/save captures.
The minimum sustained duration/cycle requirements remain. Control release is
observed before following navigation; the independent save observer is cancelled
and joined on failure. A short rehearsal cannot become sustained acceptance.

## Completed verification

- All 42 registered host/static checks pass without source drift, with 2,628
  passing Rust test executions. All 902 Python tests pass.
- The compact 1024×768 rehearsal passes the complete workflow, including
  find/replace, automatic lists, pending-save Close protection and reserved
  successors 16/17/18. It preserves **20 immutable verified captures**.
- A fresh read-only process replays all 20 captures, exact raw cycle/Close
  receipts, source-bound pixels and terminal durable payloads.
- The rehearsal has one active cycle and **acceptance=false**. Its confirmed
  Quil Close observed pending0 after a successful save; pending-I/O teardown is
  explicitly left open.

[Machine evidence](assets/sexos-alpha-evidence-rehearsal-2026-10-31-evidence.json)
links the [rehearsal archive](assets/sexos-alpha-evidence-rehearsal-2026-10-31-raw.tar.gz).
It includes frozen sources/checkers/image, raw frames/prefixes, terminal disks,
host/Python logs and independent replay. The separately running 1800-second
session is excluded from this completed evidence.

The unchanged image hashes are:

- ISO: `79a48af561534484182f02cb641f38054962996c77f96f619e46a7bf4b9d2b3f`.
- Kernel: `7f2bb264bce9bf74945fd0a7df07efd338ad8c437e68859a5bdeb738297a5740`.

## Work in progress

The compact 1800-second session is running from the frozen runner cohort with
find/replace, workspace refresh, the third Quil reserve and 16 typing bursts.
It must finish active cycles, guarded Close, recovery and cleanup, then pass
fresh replay before sustained-alpha acceptance can be recorded. Its live
progress file and process handle are tracked separately; no elapsed intent or
short rehearsal substitutes for its terminal result. The wide repeat follows.

The OS inventory remains **2 Open / 28 Partial / System Proven false**. This is
an evidence/lifecycle improvement, not global execution-epoch replay or a general
daily-driver qualification. GLOBAL_VAS remains addressing only; BOOT sanctions
capability genesis, sex-pdx supplies runtime authority and PKU enforces it.
SexOS retains its own SASOS design, with no Unix/Linux/POSIX compatibility layer.
