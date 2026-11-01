# Round 156: independent Browser bitmap-store lifetimes

Display reserves a separate static bitmap Stage for each canonical Browser row.
The collection maps an exact held binding to its filtered catalog index; packets
cannot select an empty store, manufacture a binding or rebind retired storage.
Startup must install every expected binding before inbox dispatch. Each store
keeps independent staging/published pixels, sequence floor, historical receipt
and permanent retirement metadata.

Full on one destination does not stop the other stores' bounded reply attempts.
Equal sequences on different surfaces remain distinct. Terminal cleanup erases
only the exact owner/task, drops its pending receipt and retains its tombstone;
reset cannot revive it. The existing Stage implementation, clipping, alpha and
render-before-cleanup-ACK ordering are unchanged.

| Configuration | Static bitmap backing |
| --- | ---: |
| Current actual Browser count 1 | 18,664 bytes |
| Prior single Stage | 18,664 bytes |
| Explicit host-future count 3 | 55,992 bytes |

Initialization updates only existing Stage metadata, avoiding an 18KiB temporary
or assignment of a new Stage. Per-pixel lookup scans only installed Browser
bindings. Actual production still has **one Browser 15** and no new row, task,
source grant or activation authority.

## Verification

Source commit: `6cb45e0d31e207a52a0883e36a22149922210da2`.
All **43 frozen checks** pass: **3,050 logged Rust executions plus 22 nested cases**
(**3,072 total**) and **1,051 Python tests**. The collection fixture runs 10 current
and 10 host-future cases plus two runtime ambiguity controls; three compiled
mutants reject fixed-index aliasing, first-store-only polling and reset-before-
rebind. The malformed host SID case bypasses only the SDK copy's duplicate-SID
const assertion to test the collection's own rejection; production guards remain
unchanged. The existing 44 compositor and 17 Stage cases pass. Binding checks
retain 5 metadata+3 actual LIVE-helper cases, two compiled mutants and seven
wiring controls, including missing/duplicate startup stores and wrong dispatch
association.

Eight boots and **112 registry-verified captures** pass both desktop sizes,
full 8 app chains, independent rehearsal replays, connected TLS image/document
workflows, cleanup and parent-bound cold recovery. Every boot records exactly
one installed 18,664-byte static store before dispatch. The original source grant
records and late BOOT readback remain valid. Writer 29/task 29 generation 55, 761 bytes
recovers into fresh Spindle 12; Up recalls `echo alpha29`, with no standalone
cold-artifact replay claim. Multiple Browser identities and equal-sequence/Full
interleavings are host fixtures, not newly installed native successors.

All 582 production and 618 runner entries, 1,200 distinct files, match committed
Git blobs, frozen copies and source binding. Nine ISO-extracted ELF scans inspect
327,612 decoded instructions with zero inspected vector/FP matches. This static
scan establishes no runtime register/cache erasure proof.

ISO SHA256:
`b0de446ddbd825ccc10bf3042b53f4c71d0c2af3d38e8a5d766f17c720495824`.
Kernel SHA256:
`3cba88b80c1685cba6f0b0b265d32d5f5cf4e3f1826fa84c2c3c5ceac91bdef8`.
The 66-input ABI source hash remains 73a2f412.

The [evidence](assets/sexos-browser-bitmap-stores-2026-11-01-evidence.json)
and [raw archive](assets/sexos-browser-bitmap-stores-2026-11-01-raw.tar.gz)
retain qualified inputs, artifacts and controls. All 1,567 archive members were
reopened and checked by size and SHA256. Archive 14,686,669 bytes; SHA256:
`7d642758ffa49f6f85f3d68e934e131a1a79345e2922f58685f8173df6972ab0`.

Next 157 installs BOOT-reserved Browser 30/31, their checked Net/Bell routes and
existing activation chain, plus a native Browser launcher action and selected-
consumer source evidence. Distinct stores, source associations and captured input
are now prerequisites in place, rather than a mutable current-Browser selector.
GLOBAL_VAS remains addressing only; BOOT alone creates capabilities; typed
sex-pdx authorizes and PKU enforces. No process VAS or runtime capability mint is
introduced. Inventory remains **2 Open, 28 Partial, System Proven false**; new-image
1800-second acceptance, native unequal-task allocation and broader protocol gaps
remain outside this qualification.
