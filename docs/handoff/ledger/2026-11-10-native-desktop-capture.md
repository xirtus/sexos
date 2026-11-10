# Real native desktop PNG capture

The first Spindle now supports `capture FILE.png`: a visible whole-desktop action,
with an Escape cancellation hint, followed by an actual PNG file in the current
Files directory. It requests the next normal completed full Display publication.
No framebuffer mapping or pointer is given to Spindle. Other Spindles are refused.

One 16 MiB BOOT extent has a 4 KiB metadata page and packed native XRGB payload.
Display has write authority; the first Spindle has an exact-peer read-only grant.
Native images fitting the remaining extent are supported (2560x1600 tested).
Larger images are rejected visibly, without scaling or cropping. Kernel wiring
adds six nonblank lines through existing `paired_at`; no syscall or kernel policy.
The new wire module is included in the 80 pinned ABI inputs.

Display copies only its successfully committed private front, including composed
cursor and the publication's night-light transform. Its temporary front borrow
ends before another prepare. Shared bank accesses use matching-width SeqCst
atomics, so cancellation scrubbing never aliases a Rust immutable pixel slice.
The client validates the held header around each bounded encoding turn and file
handoff. PD/task, request sequence and kernel-owned Display restart epoch fence
stale requests. Service restarts can reuse task ids; app rebirth instead obtains
a fresh NEXT_RUN_TASK after required cleanup ACKs.

The bank is not overwritten while leased. Release, timeout, app retirement and
Display restart scrub all sensitive metadata/pixels before reuse/cleanup ACK.
Only the public Display epoch remains after ordinary release. The BOOT grant is
PD-held and persists; logical session invalidation is not described as physical
capability revocation or task isolation. Pending authorized RELEASE OK receipts
survive reply backpressure, block a new BEGIN until delivery, and are discarded
when their old owner retires.

Spindle streams stored-DEFLATE PNG in 32 KiB turns; it has neither a second native
frame nor a complete PNG buffer. Files' region has one owner throughout the job.
Short writes and file errors abort. Cancellation revokes the frame promptly but
keeps a pending Files request buffer fenced until its real reply. The final path is
created exclusively before writing, so a destination appearing after the initial
STAT is refused without replacement or removal. PNG bytes stream directly into
that named file, then sync; success requires the matching positive release ACK.
A failed/cancelled operation may leave a partial PNG at the requested pathname.
There is no atomic-rename claim.

## Recorded proof

The clean shipping-code snapshot tested manually was
`558b6d4b2019c842ef0ad165adaababdc450a344` (before the later short cancellation-hint
copy, master #141 merge and final-path exclusive-create correction). ISO SHA256
`5946e0f4f196fd1a436463c7f0d6bfa878fc6315064122603f76a6a5c3020051`;
native-AOT/FFmpeg Web SHA256
`9f1351806ac2b6adc155acfcd2fc6cde1c9b61cb07ceb2378bbabcee33c4b875`;
ABI snapshot SHA256
`8d908d5e79dcacaaed5da3cbc6b94b6f23f61adcb3ea9fa2a88f25f79fe6290f`.

- 1280x800: actual guest PNG 3,073,098 bytes; all 809,844 compared stable pixels exact.
- 2560x1600: actual guest PNG 12,290,603 bytes; all 3,360,050 compared stable pixels exact.
- Both files were recovered from the stopped guest disk with the actual SexFS
  implementation mounted read-only on the host, then decoded independently.
- The desktop stayed live. A generation-bound diagnostic identified only the
  captured terminal frame and bar as dynamic. The checker validates their bounds,
  masks that frame plus 8px chrome extent and bar, and requires at least 50% native
  coverage and exact equality of every remaining pixel to a later QMP scanout.
  Observed coverage was 79%/82%; this is not all-pixel equality to a later frame.
- Copy observations were 5/20 guest ticks; complete 16 MiB release scrubs took
  7 ticks in each run. These single TCG samples are not a responsiveness guarantee.

Evidence, raw logs, saved PNGs, input manifests and compiler versions are at
`/Users/xmacbookm5/Projects/sexos-evidence/desktop-capture/native-558b6d4b/`.
Inputs were privately copied and hashed. The qualified local recipe used Homebrew
Clang 23.1.5, then Apple Clang 21.0.0 plus verified musl 1.2.5 headers for Chello;
Web stayed byte-identical through that second stage. No sibling Rust cache changed.

Twenty-one production-code host cases cover authority, geometry, publication failure,
stale epochs/tasks, restart/retirement, timeout, complete scrub, provider ACK queue
backpressure, queue retry, EOF revocation, Files buffer fencing and short writes.
Four compiled negative controls are rejected. Independent CRC/zlib/RGB decoding
checks every pixel of a 129x173 streamed fixture and an exact one-pixel PNG golden.
Full service custody host gates pass. Native PNG is now in normal runtime CI;
screen recording, audio capture, arbitrary-app capture grants and 4K support are
not established by this screenshot feature.
