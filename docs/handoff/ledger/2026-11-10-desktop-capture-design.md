# Native screenshot: bounded authority and publication plan

This topic implements a real `capture <file.png>` command in the first trusted
Spindle incarnation. It saves a native-resolution PNG through that Spindle's
existing Files route; other apps receive no capture backing or new authority.
There is no framebuffer mapping, screen pointer export, new syscall, runtime cap
mint, Display file access, or capture from an uncommitted raster target.

## BOOT mechanism and bounds, before wiring changes

One fixed 16 MiB BOOT extent is paired through existing `paired_at`: Display has
QUERY_VIEW/READ/WRITE and the first catalog Spindle has QUERY_VIEW/READ, each with
the exact opposite peer. A 4 KiB metadata page precedes packed XRGB pixels. This
covers native 2560x1600 and smaller supported desktops. Larger native images
exceeding the remaining extent are rejected visibly; they are never silently
scaled or cropped. No ELF static frame bank is added. Expected kernel delta is
one thin BOOT wiring function plus its init call; report exact nonblank LOC in the
PR. This is declared capability/memory mechanism, not kernel screenshot policy.

The one-shot lease is bound to kernel-stamped caller PD/task plus a fresh request
sequence and Display service epoch. Display samples the kernel OP_RESTARTS count
fail-closed and publishes epoch=count+1; BEGIN/RELEASE must name that exact epoch.
Same-task service restarts scrub the old bank before installing their new epoch.
The public epoch remains after release; all sensitive metadata/pixels are scrubbed.
App rebirth mints a fresh NEXT_RUN_TASK after required retirement ACKs. Display validates the current live incarnation, bounds, exact Region
identity/rights/peer and single active lease. The command first displays its
whole-desktop capture intent. Arbitrary programs and other Spindles are denied.

## Coherent capture and release

Accepting a request schedules a full normal Display publication. Only after its
successful commit does Display synchronously copy `front_pixels()` into the
capture bank and publish metadata/receipt. The front includes cursor composition;
apply the publication's night-light level during the copy so PNG colors match
visible output. The temporary front borrow ends before any subsequent prepare.
No long-lived compositor pointer or slice crosses IPC.

Once published, the bank remains immutable until the matching owner/task/sequence
acknowledges or the lease is cancelled. Spindle encodes and writes in bounded
chunks without retaining a borrowed frame slice over an IPC turn. Completion,
Escape cancellation, failure and lease timeout invalidate metadata and scrub the
entire extent. Display must scrub before its existing lifecycle cleanup ACK lets
the PD be reused; a new incarnation cannot inherit the prior screenshot. The
BOOT Region itself is PD-held and is not falsely described as physically revoked
by this logical session protocol. All tasks within a PD share that authority.

PNG encoding uses bounded streaming stored-DEFLATE, CRC32 and Adler32; no full PNG
or second full-resolution frame lives in Spindle. File output is driven by the
main loop, with a single Files-region owner, precise short-write/error handling
and a final durable Files sync before reporting success. Partial output is named
and reported as incomplete; it is never called a saved screenshot.

## Required evidence

Host tests cover bad Region identity/rights/peer, wrong PD/task, stale sequence,
busy/oversize admission, failed publication, ACK ownership, timeout, restart and
complete scrubbing. Streaming PNG output must decode independently and match
pixel golden data, including chunk boundaries and odd dimensions. A native QEMU
gate must perform the actual person-triggered command, recover the saved PNG from
the guest filesystem, decode it, and compare captured pixels to the corresponding
completed desktop frame. Receipt-only or host-only API tests are insufficient.

Recording remains a later complete feature over this capture custody: explicit
profile/cadence, visible start/stop, bounded backpressure and a userspace encoder.
It is not claimed by the screenshot PR.

Host validation now covers 17 production client/server/PNG cases, four compiled
negative controls, independent CRC/zlib/RGB decoding, and full service custody
host gates. Native PNG extraction/decode gates remain required before completion.
Kernel wiring delta: six nonblank lines (seven total added lines), no syscall.
