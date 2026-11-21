# Recording bank and Display contract

Unarmed implementation plan, agreed with the controller/app owner before edits.
Foundation: `719585d6`, with current master merged separately. No BOOT grant is
enabled by this work. Files/controller/app and BOOT remain separately owned.

The dedicated SDK module is `desktop_recording_capture`; slot 95, region 0x6101,
one MiB. Display is the only writer; the source-pinned recorder PD 42 receives a
read-only alias. Existing screenshot slot 94/region 0x6100 remains separate.
The fixed output is 640×360 XRGB, letterboxed to preserve native aspect ratio,
at most five frames per second and at most 60 seconds on the qualified timer
profile. No microphone or network grant belongs to recording.

## Typed transport

Control opcode 0x2e4 uses these three words:

1. kind8 | RoleSequence24 | SessionSequence32, from high to low bits.
2. DisplayEpoch32 | recorderTask32.
3. ShellEpoch32 | initiatingSpindleTask32.

Every narrowing conversion is checked; epochs are nonzero and the existing
epoch-prefixed sequence constructors remain authoritative for canonical encoding.
Arm=1 and Stop=2 come only from actual Shell PD/main task 3. Acquire=4 comes
only from the current admitted main task of recorder PD 42. Embedded numbers
never replace stamped identity, current lifecycle or held-peer epoch checks.

Control replies use top byte 0xC0+status, then task32 and RoleSequence24:
Armed=0, Stopped=1, Denied=2, Busy=3, Unavailable=4, Expired=5, Cancelled=6.
Frame replies use 0xD0+status: Ready=0, Acked=1, Released=2, Denied=3, Busy=4,
Cancelled=5, Expired=6. Actual Display sender and matching task/nonce are required.
Malformed-nonce denial never authenticates as a matching positive reply.

Frame opcode 0x2e5 uses kind8 | RoleSequence24 | SessionSequence32, then full
publication generation64, then full Display epoch64. Ack=1 releases exactly a
delivered immutable frame. Release=2 requires generation zero and revokes/scrubs
the entire session before retaining Released; it does not authorize another frame.
Shell Stop is the immediate revocation path and precedes app/codec/Files waits.
Stop is idempotent for the same valid request; wrong scope/nonce cannot arm.

Negative-only opcode 0x2e7 informs Shell of session closure after Release, expiry
or retirement. It reuses the control scope packing and carries no arm authority.
Display already holds Shell route 6. Retained delivery must remain epoch fenced.

## Data and publication

Atomic frame metadata names full Display/Shell epochs, session, actual recorder
and initiating Spindle tasks, Acquire nonce, committed generation, publication_ms,
native dimensions, fixed output/content rectangle and night-light transform.
Publication_ms is sampled by Display at successful CPU publication, never derived
from frame number. The current x86 LAPIC timer is calibrated to one ms ticks;
native proof must preserve calibration and compare decoded PTS gaps to host time.

Writer and reader use matching-width atomic cells. A monotonic publication token
and exact header checks bracket bounded copies into the app's single private frame.
Cancellation may scrub concurrently; no shared Rust slice survives a turn. Frame
ACK precedes C encoding and never frees an outstanding Files request buffer.
Scrub erases sensitive metadata/pixels; only provider epoch and a nonwrapping
publication counter may remain. A slow encoder drops capture opportunities and
retains real timestamp gaps.

Arm can complete only after a matching immutable Scene and actual indicator
pixels successfully pass the CPU framebuffer publication path. A queued update,
private preparation, stale scene or metadata-only settle is insufficient. No GPU
vblank/latch or atomic hardware visibility is claimed. The protected Chip0/Net
target becomes a visible stop-ring/square using the existing palette and geometry;
network updates cannot suppress it. Unsupported/clipped/hidden indicator placement
refuses capture. Stop/retirement/restart/expiry fences copies before other waits.

## Required acceptance

Production-module controls cover strict wire/identity/epoch/nonce decoding,
read-only grant construction, bounded coherent copy and concurrent scrub, aspect
ratio/letterbox/night output, one held frame, rate/duration and wrap refusal,
retained reply pressure, stopped/retired owners and stale prepared publications.
Native integration must show actual visible Start/Stop, real changing desktop
frames recovered from the MKV with correct timestamps/gaps, keyboard stop, cancel,
restarts and actual read-fault denial from key6/shared8/background10. Ordinary
program gates must select remaining permitted rows from sealed BOOT facts.
No bank/Display source or host test alone is a working recorder claim.

## Unarmed SDK transport checkpoint

Root implemented strict capture wire data and the atomic64 shared bank, without
Display hooks, grants, indicator activation or a running recorder. Five host
controls compile the actual SDK sources and pass: read-only mutation refusal;
full bounded copy plus scrub/stale-token rejection and non-ABA republish; complete
sensitive-cell scrubbing and epoch advance; malformed source/range rejection;
and wrong peer/rights/alignment/DMA/length grant rejection. The raw host log is
`sexos-evidence/desktop-recorder/bank-host-first.log`. Fixture grants and allocated
host backing are explicitly synthetic, not PKU or BOOT evidence.

The broader acceptance list above remains pending, including concurrent scrub,
wire-negative coverage, counter exhaustion, letterbox/night output, real Display
publication, rate/duration controller, restart/retirement and native isolation.
The unsafe bank constructor requires actual held syscall results and the sole
Display writer; callers must separately validate lifecycle and full session.
