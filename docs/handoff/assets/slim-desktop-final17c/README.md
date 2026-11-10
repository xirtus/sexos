# Actual native slim desktop checkpoint

These are QMP captures of the real SexOS desktop, converted from guest PPM pixels
without retouching. Source `17c7f700185635428b0193979bd60cd7b55f2e48` includes
master `3e075b59` (native screenshot capture, encoder and dormant source pin).
ISO SHA-256: `213007e199bb3cda516abe5ad08c7b8ed54bc99e4153a073e74c3bdc86cde34b`.
The source tree was clean during the successful build; compiler/header details are
recorded in `build-inputs.json`. No Rust target was copied from another worktree.

Both views pass the strict complete 3,456-pixel Geist Sans clock oracle and published
28/26/8/34/24 geometry checks. This UI candidate omits FFmpeg/libc++ libraries, so
these captures do not qualify media or the complete C capability suite. Native app
body text remains Geist Mono and panels remain square pending the next approved
Sans-label/whole-panel-mask stage. These are implementation evidence, not final UX
full approved-UX acceptance. Pointer, source-custody and isolated performance
qualification is recorded in the adjacent JSON evidence and implementation ledger.

![1280 by 800](1280x800.png)
![1024 by 768](1024x768.png)

The isolated performance comparison uses a fresh matched master 3e075b59 image,
60 actual layout toggles per image in three alternating boot pairs, and a separate
same-image noise control. Its p95 ratio is 0.942720 against the unchanged 1.05 limit;
this is the specified QEMU observed-pixel trace only. Actual 2x control hits, palette
erasure, pointer resize/title drag, and native PNG extraction also pass. Current-head
CI remains required before merge; no final daily-driver or full-UX claim follows.
