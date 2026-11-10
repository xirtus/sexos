# Generic BOOT-owned exact image-source constraint

Adds a dormant generic mechanism: an AppActivation grant may carry one exact
approved immutable image source. All existing BOOT assignments remain None, so
ordinary rows retain their existing ability to run disk programs. This does not
reserve a recorder row, assign a new image/capture capability, add a syscall,
change the static BOOT manifest bound, or establish native recording authority.

BOOT's retention helper copies at most IMAGE_BYTES (32 MiB) into kernel-private
backing reserved through existing `backing()`. That helper reserves the physical
frames with the frame manager and never returns them to the Program frame pool.
The source has no user/DMA Region grant, alias helper call, runtime writer or
free path. Only an unsafe crate-private constructor can make the descriptor;
its caller must retain exclusive immutable backing for the boot lifetime.

The existing `program_image()` helper obtains a kernel-owned candidate copy from
Files' shared input. The grant's source checks its exact length and every byte.
The very same owned copy is then parsed and loaded, removing a verification/use
gap through the mutable Files input. First image activation and rebirth-with-image
check before task id, generation, manifest, arena or leaf mutations. Kind3 is
refused with RETRY while a previous rebirth is pending; kind2 remains the progress
operation. Existing Shell PROGRAM_REBIRTH polling uses that kind2 operation.

The exact net kernel growth is 32 nonblank Rust lines across app_resource_state,
app_resources and boot_memory. This is capability narrowing and BOOT backing,
with no recorder filename/profile/consent policy in the loader. Capture PR147's
existing BOOT pair and the 80 ABI input pins are retained on master e49b5f54.

The actual complete app_resources module is compiled in the host gate. A child
fixture inspects private state without adding production test hooks. Registry,
page-frame allocation/mapping, scheduler and shared loader input remain explicit
host doubles. Actual first activation and rebirth reject a same-length one-byte
mutation and a wrong-length candidate, preserving task id, generation, manifest,
arena, leaves and pending reply contents. The original approved ELF subsequently
loads with exact private frame bytes in both paths. A third case proves a kind3
request cannot advance the old pending rebirth using a new ignored candidate.
Two compiled mutants (source bypass and generation before proof) are rejected.
Pure exact-source and private-copy controls also reject prefix/suffix/byte changes
and prove changing Files input does not change the owned verified copy.

All AppActivation literals compile under the app-resource and full catalog gates,
including the catalog's compiled fault controls. Guardrails and optimized x86
kernel target build pass. All current source checks are model qualification;
actual source-constrained BOOT assignment and native denial/remaining-row gates
belong to the separate recorder integration slice. Normal current-head native
program gates in CI must still pass before merge.

Scope: the atomicity claim concerns source mismatch. An approved source that then
fails ELF parsing or resource allocation retains existing loader failure behavior;
this change does not claim transactional recovery for every loader rejection.
Alternate rearm routes were inspected: service restart captures only six fixed
service domains (not Program rows), Program workers stay in their caller's image,
and background records use a different fixed key/domain allocation path. BOOT
placeholder restoration does not load disk bytes and cannot restore consent.
