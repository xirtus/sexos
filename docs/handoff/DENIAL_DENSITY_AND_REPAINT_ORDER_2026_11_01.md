# Round 163: denser public errors and ordered Spindle repaint

**Candidate B passes its bounded host and native qualification.** All 43 frozen host checks and 1,124 Python cases pass; host logs record 3,139 successful Rust test executions, including reused SDK cases rather than that many unique tests. Nine boots and fresh replays qualify 152 captures. The final archive has been reopened and verified.

Candidate B makes two production changes: it reduces Net's public denial outbox from 4,040 to 1,352 bytes at the current 28 owners, and fixes a Spindle repaint ordering defect exposed by candidate A's wide desktop run. The catalog remains 28 apps and 40 actual mains within the 64-entry bound: 16 Quil, nine Spindle and three Browser instances, allowing 15, eight and two reopens after their initial instances. No capability genesis, runtime admission or addressing model changes.

## Public denial storage

Each denial slot now holds its exact owner/domain and task, latest DENIED sequence floor and pending sequence in 16 bytes instead of 48. Zero denotes an empty identity or sequence because admission excludes it. The immutable protocol partition and the pending sequence's relation to its floor reconstruct the typed DENIED or STALE word. A different pending request is rejected before any floor update, so Full cannot change a retained reply. Duplicate correlation, stale floors, partition capacity, fair retry and exact-identity retirement remain bounded by the existing rules.

The B boot smoke measures 84 slots and **1,352 bytes**, below the unchanged 4 KiB bound. A prospective 64-owner host fixture measures 3,080 bytes; the old representation measures 9,224. That fixture changes descriptive sizing only. It does not create 64 BOOT apps or authorize additional callers.

The registered checker uses the actual SDK encoders and a SHA-pinned copy of the old 48-byte R161 implementation. Its scoped controls cover 86 normal case executions, three differential configurations, eight rejected runtime mutants and one rejected comparator-sensitivity control. The comparison enumerates 115,248 depth-four histories over its stated alphabet (460,992 operations), then performs 602,484 operations in deterministic long traces at 28 and 64 owners. These are finite compatibility checks plus invariant reasoning, not a formal proof of every possible history, concurrency or SMP behavior. The completed frozen host gate includes these controls; the aggregate execution count above is reported separately.

## The failure that required candidate B

Candidate A failed at Spindle29's recall capture. The oracle tried 376 times over 90.001 seconds and still found 608 mismatching pixels. The final screenshot exactly matched older paint9's empty prompt; expected paint11 contained `echo alpha28`. This was a persistent guest ordering defect, not a reason to relax the pixel checker.

Completing a batch could leave `active=None` while an older desired batch remained in `latest`. New input then admitted paint11 directly, but a later poll sent stale paint9 after it. B clears the superseded, unstarted latest batch before admitting new input in that gap. The regression observes the old receipt order `7,11,9` and requires `7,11`.

The retained host proof passes ten tests, including two new tests and 540 matrix executions across nine subjects, four panes, five payload lengths and three admission variants: no Full, Full at clear, and Full at the final operation. Those are not 540 unique scenarios: for a zero-byte payload, clear is also the final operation. The exact old implementation fails the new runtime control and the fixed implementation passes. Clearing buffered bytes here is not evidence of physical-memory, register or cache erasure.

A's failed wide evidence stays separately **UNQUALIFIED**, with its raw screenshots, trace, failure manifest, diagnostics and original frozen source/image. It contributes no accepted B boots or captures. No successful A Browser or compact result is transferred to B.

## Binding and qualification

Source: `502785381c2d5c61de422e936696dc9804200089`.
ISO: `3cbb893f9c757a3097430cbfb53cc131edc2836482dd24507b20c15ccd89bc04`.
Kernel: `4bef299f42a0a2f89dc2bda76ee717623f9a213341939268be908168006e1f7c`.
Source-binding SHA-256: `723bf6971b727ba4691664f8015dd5fb572b024b5739bce7472da5e4e5559c3b`.
ABI: 66 inputs, `ff1d1346b449ed2efe6e5cffbc9c545b27d4d351979ad890fac43ffaa9725ba8`.
Architecture: `4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`.

The source audit covers 1,211 blobs totaling 12,016,277 bytes. Of 582 production blobs compared with R161, 580 are unchanged; only `apps/spindle/src/display_delivery.rs` and `servers/sexnet/src/source_denial.rs` differ. Kernel bytes remain identical, while the ISO changes. Nine ISO ELF scans decode 323,819 instructions with zero FP/vector matches. Source comparison and a static scan do not establish behavioral equivalence or runtime erasure.

The completed B cohort contains one sealed-build boot, two allocated-Close Browser profiles, two 20-second Alpha rehearsals and four connected session/recovery boots. All nine boots and fresh replays pass: **152 qualified captures**, comprising Browser20, Alpha49 per profile and connected17 per profile. Browser evidence includes six allocated Closes. Both Alpha profiles complete all 16 Quil and nine Spindle instances and finite exhaustion. No A captures or boots enter these totals.

Across the nine boots, the post-seal kernel heap reports 11,851,216 bytes used out of 268,435,456 bytes. App-private backing totals 2,465 pages, or 10,096,640 bytes. These categories are not established as disjoint physical totals; physical free frames remain unmeasured. The native public denial-storage receipt is 1,352 bytes for 28 owners and 84 slots.

[Raw evidence](assets/sexos-denial-density-2026-11-01-raw.tar.gz): **18,880,975 bytes**, SHA-256 `ca5942c8b5fa655b9198680c927f74ba765a32661d770bd0e3e24ba8876acfd5`. All **2,900 members** were reopened and size/SHA verified. The archive includes B's frozen source/image, all qualified raw evidence and replays, the pinned reference and bounded comparison proof, and the separate unqualified A-wide evidence with its original frozen image.

Two packaging attempts stopped on incorrect log filenames. Root corrected only `frozen-python.log` to `python.driver.log` and `production-equivalence.log` to `production-equivalence.driver.log`; no acceptance predicate was relaxed. Both failed helper revisions and terminal traces are retained inside the final archive as harness diagnostics. They are distinct from A's actual guest repaint defect. The successful packager SHA-256 is `f3b4a6165b3c96b282f67b17ca0d7bfd89954b81d7402a18d1cef89d8f80cc1f`.

Native evidence covers the measured layout, ordinary authorized source clients, desktop rendering and cleanup. Unsupported-caller denial semantics remain host/model evidence unless separately tested natively. R162's 1,800-second acceptance belongs only to source `10b6eb7a5cd2a27080e509a46000a1ee6b307e04` and ISO `36b66ad37b361bedc496984f750374f8baa89ec543a4b1259facb394eed94ebc`; it does not transfer to B.

Architectural inventory remains **2 Open / 28 Partial / System Proven false**. **EPOCH_COMMIT** and **DETERMINISTIC_REPLAY** remain open; all 28 other obligations remain partial. GLOBAL_VAS, BOOT-only authority genesis, typed sex-pdx and PKU remain unchanged. Current-image 1,800-second acceptance, physical hardware, SMP and general daily-driver qualification remain unproven. No hard realtime or physical-erasure claim is made.

Next164: launcher counts derived from exact held grants. Next165: date-caption clarity and formatting once per render. Subsequent work develops the substantive Display epoch design: define the admitted work boundary, atomic visibility and replay evidence needed for those open obligations. An epoch counter or framebuffer copy alone cannot close them. This follows the active 80–200 plan's dependency ordering, not the historical roadmap extending to219.
