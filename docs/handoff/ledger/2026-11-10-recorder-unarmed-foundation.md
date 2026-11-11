# Recorder custody foundation: authority remains inactive

Local integration foundation, not an available screen recorder. No recorder BOOT
source pin, capture bank, activation policy or recording-profile image is assigned.
`SEXOS_RECORDER` defaults off. The real Record app/controller and Display indicator
remain integration work; no native recording or restart-isolation claim is made.

Files now has a guarded single-main-thread runtime, separately fenced unbound
image reads, retained typed role/data/cleanup replies, explicit `/recordings`
binding construction, and a dedicated pointer-free KEEP journal. The journal
retains resource obligations across Files restart without retaining authority;
Files epoch changes never imply an old Shell kernel-copy operation is quiescent.
Nested disk waits only defer stamped requests, and task retirement drops exact
queued old-task work before its Files cleanup ACK. Native KEEP/shared-backing
persistence and an actual target build remain required before authority use.

Spindle has an allocator-free, retained Begin/name-chunks/Commit sender and a
priority Stop path. Whole-desktop/profile/duration disclosure precedes intent
submission. Begin carries the observed Shell epoch; positive status admission is
fenced by the same held-peer epoch/liveness. Enqueue, Starting, Active, revocation
and durable Saved statuses remain distinct. Shell integration is still absent.

Host gates select ordinary subjects from complete pre-seal BOOT successor serial
observations plus the source catalog. `source_pinned` is only a BOOT serial fact,
not a capability API or permission to remove the source restriction. Heap/disk
capacity excludes constrained rows. Both JIT gates require a key unique among
all reusable rows, including pinned rows. Current keys are 5 for ordinals0/1,
6 for ordinal2 and shared8 thereafter: reserving42 leaves ONE ordinary private
row43 and six ordinary shared rows. No row is pinned in this foundation.

Validation: the unchanged Files production modules passed44 standalone cases;
raw failures, final logs/source hashes are retained under
`/Users/xmacbookm5/Projects/sexos-evidence/desktop-recorder/host-20261110T214347Z`.
Subsequent targeted checks passed7 wire cases,4 actual Spindle sender cases and
6 Python BOOT-fact cases. The combined script has not yet been rerun after these
additions; no target/guest evidence is implied by these host controls. ABI inputs
were regenerated using `scripts/update_abi_hashes.sh` (83 inputs). Kernel change
here is an added field on the existing BOOT serial line, zero nonblank-line growth.
