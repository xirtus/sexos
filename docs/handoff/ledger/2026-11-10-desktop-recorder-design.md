# Proposed bounded desktop recorder authority

Design only. No capture grant, activation right, allocator or executable change has
been made by this note. Screenshot PR147 and encoder PR148 remain independent
prerequisites. The initial explicit profile is whole-desktop 640x360 at up to
5 frames/s, video only, maximum 60 seconds. PR148 defaults to a decoder-only
build whose encoder returns Unavailable. Start must prove encoder availability
before capture can arm or the indicator imply an active recording. Full recorder
qualification requires a deliberate recording-profile image/module, not merely
the optional synthetic proof; the encoder owner retains profile/build wiring. Native screenshots retain their own
16 MiB bank. Higher recording resolution is a later measured profile.

## Executable identity and memory

Reserve existing Program ordinal 1 / PD 42 for the recorder, leaving seven
ordinary loadable program rows (ordinal 0 is the BOOT predecessor). Shell excludes this row from generic launch and background record
selection. BOOT supplies a tiny idle recorder placeholder, activated before ordinary
program launch so the ordinal 2+ chain remains ready. Reserving key 5 reduces
ordinary independently keyed reusable programs from two to one; shared-key scheduling
remains unchanged. Actual organs mapping is key 5 for ordinal 0/1; the trusted
ordinal 0 hello anchor exits immediately before 42 admission. Other ordinary
program keys 6/shared 8 and background-record key 10 must receive real read-fault
denial for both recorder bank and heap; software Region denial alone is
insufficient. No untrusted row may share key 5 with the recorder. Last row 49 is unsuitable without a new activation-fork
mechanism: it cannot start before row 48 has activated.

Keep the small BOOT placeholder bound: reusable BOOT images have a
64-page manifest including all PT_LOAD segments and a 64 KiB stack, so loading the
FFmpeg executable as that static image would exceed the existing bound.

Use the existing dynamic Program arena, image loader, tenancy key, lifecycle and
retirement machinery. Add a generic BOOT-owned image-source restriction to the
row's activation authority. BOOT assigns the recorder's immutable trusted module
as the one approved image. Normal rows retain unrestricted image replacement. The approved source is copied
once into bounded kernel-private BOOT backing (at most IMAGE_BYTES, currently
32 MiB), retained for the boot lifetime and given no user Region or DMA alias.
Audit that its allocation cannot re-enter the program frame pool; the raw Limine
module pointer alone is not sufficient lifetime proof.
A filename, generic LOADED state, or user-supplied hash is never identity evidence.

Preferred minimal implementation retains/copies the approved module in private
BOOT backing and compares length and bytes against the already kernel-owned
`program_image()` copy inside `load_program()`, before parsing or modifying state.
This avoids adding cryptography to the kernel. The approved source is immutable throughout comparison and activation; the
candidate copy stays privately owned and is the very copy parsed and loaded.
Both first kind3 activation and
`rebirth(Some(image))` pass that choke point. Ordinary activation and
`rebirth(None)` may restore only the tiny non-capturing BOOT placeholder; they
cannot regain recording consent. Main-task restart must use the same validated
image or placeholder and always obtain a fresh session. Audit service-restart and
all rearm callers to exclude an alternate image substitution route.

Existing capabilities give Shell broad image-load authority, but cannot restrict
that authority to one approved source. Checking bytes in Shell/Files alone leaves
a check-to-use interval in the shared Files image buffer before the kernel takes
its private copy. A separate user loader could enforce this only with exclusive
image-buffer authority, which the existing global Files load path does not offer.
The source restriction closes that gap at the existing private-copy choke point.
A rejected candidate must leave the original image usable. Verification precedes
all app/arena/image/generation mutation, including the current rebirth path that
advances generation before calling load_program; move verification/preflight
earlier rather than merely inserting a check inside the mutating branch.

The kernel implements source-constrained loading mechanically; recorder naming,
profile, consent and destination remain in user servers. Exact kernel line growth
must be disclosed before implementation. Do not raise the static manifest limit.

The recorder binary has its own bounded allocator, explicitly capped at 64 MiB of
existing `program_memory` pages, with the cap checked before each growth call.
This is explicitly a heap cap, not a total image-memory cap. It includes the
Rust output queue (maximum 32 MiB), FFmpeg allocations, private 640x360 XRGB
frame (921600 bytes), UI allocations and any heap-backed alternate stack.
Separately account for actual PT_LOAD private/writable pages, the kernel-owned
64 KiB BOOT idle stack, dynamic Program stack (default 1 MiB unless ELF
GNU_STACK overrides) and immutable image pages; report both heap and total resident
bounds. Prove peak/denial controls before increasing limits. Spindle stays
allocator-free.

## Consent, routes and Files ownership

`record FILE.mkv` and `record-stop` originate only from the first live Spindle.
Shell receives actual stamped PD/task, starts the pinned recorder and binds an
explicit recorder Files role to its actual admitted task and `/recordings/`.
That binding is independent of mutable generic store LASTREAD/LOADED state.
Files authenticates Shell and checks current lifecycle before applying the role.
No recorder network or microphone binding is requested.

Spindle gives visible whole-desktop/profile/duration disclosure. Shell starts a
session only after matching the initiating Spindle task and recorder task; Display
checks both live lifecycles and its own kernel-owned restart epoch. Shell owns a
persistent trusted recording indicator and Stop route; reuse the existing Chip0/Net 24px target per slim-geometry coordination.
Logical 320 has no spare slot: paint a red ring/stop mark (optional 13px Sans R),
using the captured Scene.bar rectangle and Scene.metrics; preserve existing hit
geometry. Give it a Stop recording tooltip/label and keyboard command route;
shape conveys state without relying on color. Bar click stops recording while
active, even if codec or Files is busy. Net remains via palette/terminal/panel. Override offline chip.visible=false while
recording. Missing/zero/out-of-bounds slot or a hidden overlay must refuse arming.
Display privacy/paint edits follow the #146 refresh to avoid geometry collision.
Net updates must preserve recording paint/tag; never derive it only from transient
network visibility. At postcommit, current active epoch/session must still match
the prepared indicator tag; an old prepared frame cannot rearm a revoked session.
Start and Stop force the required indicator damage/publication transitions. Recording must not continue
without that indicator. Display arms copy only after committing an actual
publication containing the matching session indicator; an update message sent
by Shell is insufficient. Indicator receipt belongs to the real published scene
generation. Verify visibility at every supported scale/workarea, including
logical 320 fit, or refuse before arming. Stop invalidates copy permission before
waiting for encoder drain or any Files reply. Closing/minimizing/restarting the recorder cannot silently
turn it into hidden capture. Escape/Stop/Close/kill, either party's retirement,
Display/Shell restart or duration expiry invalidates consent before further copy.

Final output is CREATE-exclusive within `/recordings/`, no replacement rename.
Failure/cancel may retain a named partial and must say so. The recorder owns its
Files shared request buffer until the authenticated matching actual reply, even
on timeout/cancel. No C callback receives or issues Files IPC. Queue token ACK is
performed only after that actual successful write length. Only drained encoder
End plus file sync plus capture-release acknowledgement permits a Saved result.

## Frame custody and state

Create one separate 1 MiB BOOT pair: Display writer, reserved recorder read-only.
Ordinary Program rows receive no bank or capture route. The bank starts scrubbed
and is never populated by the recorder's request alone. Header records Display
epoch, consent/session sequence, both task ids, published generation, native
source dimensions, explicit output 640x360 and monotonic publication timestamp.
Use matching-width atomics as in PR147, not an immutable borrowed shared slice.

The fixed 640x360 profile preserves source aspect ratio with bounded letterbox
rectangles rather than stretching 16:10 desktops; metadata records the content
rectangle. No scaling claim is applied to native screenshots.

Display samples only a successfully committed private front publication, with
cursor and that publication's night-light transform. Downsample into the fixed
profile after commit, never map scanout or share Display's long-lived front slice.
Publish at most once per 200 ms and only when the previous frame is ACKed/scrubbed.
A slow encoder drops opportunities rather than queuing more frames. Wall-clock
monotonic timestamps preserve gaps; no invented repeated frames for dropped time.

Recorder copies atomically into its one private XRGB frame, checking the held
header before and after. Then it ACKs the frame, allowing scrub, before encoding.
It drains every ordered 32 KiB output turn before accepting another frame. States:
Idle -> Starting -> Consented -> WaitingFrame -> Copying -> Encoding -> Draining
-> WaitingFrame; Stop -> Releasing -> Finalizing -> DrainingTrailer -> Syncing
-> Saved. Failure -> Releasing -> AwaitFilesReply -> Failed. Consent denial never
enters WaitingFrame. A stale frame/ACK/session must not advance any state.

Provider reply backpressure retains authorized receipts until delivered and
blocks reuse; lifecycle retirement drops old-owner receipts. Restart, timeout,
Stop and EOF/file-error all invalidate the session and scrub the entire bank
before reuse/cleanup ACK. Exact task fencing is mandatory even when a service
restarts with the same task id; Display epoch changes invalidate all old metadata.

## Files and proof work

Expected changes: `crates/sex-pdx/src/desktop_recording.rs` wire/role constants;
`kernel/src/app_resource_state.rs`, `app_resources.rs`, `boot_memory.rs`, `init.rs`
for generic source-restricted authority and bounded pair wiring; app catalog
reserved-row metadata; `servers/silk-shell/src/desktop_recording.rs` with narrow
main dispatch/indicator hooks; Files explicit role binding; Display separate
recording module and light postcommit hook; Spindle separate command module;
new `apps/record` binary/allocator; build/ISO wiring and ABI pin regeneration.
Do not alter the encoder owner's sex-media/build_ffmpeg files.

Before a shipping PR, require production-code host controls for unauthorized PD,
wrong executable length/bytes, all alternate image-load paths, ordinary-row
behavior, stale tasks/epochs, same-task service restart, timeout/cancellation,
scrub, provider queue pressure, private frame coherence, output custody and heap
exhaustion. Actual guest proof must show visible Start/Stop plus a changing real
desktop, recover the actual MKV from SexFS, independently decode it, verify image
content and timestamps/frame gaps, exercise cancel/Close/restart denial and show
normal Program rows still launch. Synthetic encoder pixels alone are not a
screen-recording proof. Current design is awaiting source/authority review before
implementation.


## Implementation custody refinements

The recorder root is explicitly `/recordings`, outside generic `/data/<basename>` bindings. Ordinary binaries named `record` or `recordings` must never inherit that root. No LOADED or LOADED_ARGS state is consumed to identify the role.

Merged generic peer-state mechanism #151 supplies actual service epochs and live status through held Domain capabilities. Files needs one explicit BOOT-held Shell Domain route at slot 6, currently absent; programs already hold Shell at 6 and Display at 5. Epoch snapshots are admission checks, not lifetime locks. A write admitted while the initiating session is valid may finish after Stop; its retained receipt must still be fenced by the admitted task, Shell epoch, and Files epoch. Unknown/contended peer state holds custody without admitting new work or delivering a positive. Conclusive retirement drops the old positive and revokes the role. Provider retained bind responses have the same rule on every delivery retry.

Role sequences reserve their high four bits for the full Shell epoch (1–15), with a checked 20-bit counter; recording sessions reserve an eight-bit epoch and 24-bit counter. Exhaustion refuses rather than truncates or reuses an old nonce. Current restart limits fit these ranges. All checks use actual kernel-stamped sender identity and fresh held-capability snapshots; embedded numbers alone grant nothing.

Generic heap-limit work is isolated from recorder authority in its own topic. It limits heap pages only, anchors all growth to the initial base and accounted range, charges partial successful loans, and permanently fences malformed results. The recorder must configure 64 MiB before any Rust/C allocation; executable/static memory, the dynamic main stack and both shared banks remain separately disclosed budgets.

The first recorder process owns one file/session and then exits after its actual outstanding Files reply and final status are resolved. A later Start reloads the reserved row through rebirth. It must not reset/reuse Files nonces inside a surviving run; finite checked sequences and actual task/epoch custody prevent an old response from completing a later recording. Stop immediately revokes Display copy, while an already-admitted private frame/write may finish under that old scope. No new Start is accepted while that run still owns a pending Files buffer.

## Independent image loading and startup sequence

Recorder image reads use a separate unbound mode. They must not consume or clear generic LOADED, LOADED_ARGS, PREPARED, or PREPARED_ARGS state. A convenience basename chooses data; exact kernel matching against retained immutable BOOT bytes remains the identity check. Both filesystem and disk-directory fallback paths preserve ordinary loader bookkeeping.

Files owns the shared image-buffer transaction as well as Shell serializing its own controller. While that lease is held, every ordinary image-write entry refuses before I/O. A typed read receipt carries the epoch-prefixed transaction and bounded image length. Shell releases only after the synchronous kernel copy/validation call returns; Files retains the exact release receipt through backpressure and refuses new image writes until delivery. Owner retirement can reclaim the lease only when the old execution is quiescent (new service epoch or attested Dead state). Ending/Exited alone cannot imply that an old kernel copy has stopped. An exact authenticated cleanup release can surrender the buffer while the same owner epoch is closing; its receipt grants no capture authority.

After pinned loading and actual activation, Record waits for an authenticated Shell HELLO naming its exact task/session and Shell/Files epochs. It configures the heap before allocation and creates the encoder/private pixel storage before reporting available. Unavailable builds return a negative status without requesting capture or showing an active indicator. Window::open does not require Files arguments, so no generic argument latch is needed. Shell then obtains the typed Files role binding; Record receives the basename through the private bounded protocol, exclusively creates its destination and drains any encoder header before requesting arm. Shell announces active capture only after Display's committed matching indicator proof. Close/Stop revokes Display immediately while existing private encoder output/Files custody drains.


The generic `[boot.app.successor]` receipt gains `source_pinned=0|1`, taken directly from that BOOT activation grant's optional retained source constraint. It reports presence only: no filename, source contents or authority to remove/change the constraint. Gate policy must require complete sealed-BOOT records and reject other flag values, contradictory/duplicate targets and unknown catalog subjects. Ordinary launches retain their real target/task/surface assertions while excluding source-constrained rows; they do not disable source matching or relocate the recorder to a shared key. Kernel line count is unchanged by the appended observation; ABI inputs are deliberately regenerated.

BOOT serial observations are host-gate facts, not a new capability receipt API.
The successor line exposes only `source_pinned=0|1`, before the memory-authority
seal. Ordinary capacity and launch subjects come from those complete records and
the source catalog; JIT subjects additionally require an unshared reusable key.
Current `organs::program_key` assigns key 5 to ordinals 0/1, key 6 to ordinal 2,
and key 8 to later rows. Reserving row 42 therefore consumes one of eight reusable
rows and one of TWO reusable private-key rows, leaving one ordinary private row
(43), six ordinary shared rows, and seven ordinary reusable rows in total.
