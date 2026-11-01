# Display epoch implementation: next substantive slice

Implementation plan based on the Display source in candidate `a84043d0` (unchanged from Round 163). Revalidate call sites after the planned clock work. This document records work still to implement; it closes neither EPOCH_COMMIT nor DETERMINISTIC_REPLAY.

## Required final state and actual gap

ARCHITECTURE.md:43–53 requires *all* state to derive from `(epoch_id, input_stream, capability_graph)`, including hardware snapshots, with `snapshot → transform → swap → commit` and no out-of-boundary in-place mutation. Sections1.4/1.5 require epoch-gated UiIR visibility and per-sender causal ordering; the success criterion additionally requires tear-free presentation, deterministic drops, interruption recovery and replay from epoch zero. `spec/architecture.sexspec` correctly leaves both obligations open.

Current Display does not meet this contract:

- `servers/sexdisplay/src/main.rs:120–125` holds mutable global text stages, bitmap stores, frame groups and surfaces; handlers directly modify these, bar/theme/focus/cursor state. Typed text commits replace surface contents; `bitmap_transaction.rs:253–255` replaces published pixels immediately. Their staging prevents partial individual transfers, not cross-operation/cross-sender epochs.
- `main.rs:3029` drains/checks UiIR records but explicitly does not render them. Its stream epoch is not the Display transaction boundary.
- `main.rs:3617,4195–4232` includes mid-drain render, full/damage/strip redraws and cursor save-under writes against the live framebuffer. A render pass can have many external writes; a successful return is not an atomic hardware flip.
- `prepare_app_cleanup` fences exact attested dead task identities and clears surfaces/stages/bitmap/cursor references. `finish_display_transactions` presently requires a successful full render before cleanup ACK. Preserve this safety property while replacing the implementation.

## Next implementation: complete Display scene isolation, with explicit limited scope

Implement a bounded Display transaction engine, not an epoch-labelled repaint counter. Suggested new `servers/sexdisplay/src/scene_epoch.rs` and `scene.rs`, integrated through Main and `surface_lifecycle.rs`, `text_transaction.rs`, `bitmap_transaction.rs`/`bitmap_stores.rs`, and `surface_groups.rs`.

1. **Snapshot:** capture a bounded immutable input batch plus previous committed scene. Retain actual IPC caller PD/task, full arguments, exact held catalog/binding identity, lifecycle-query results, clock/tick observations and any copied shared payload. Keep authority decisions at sex-pdx; a recorded tuple cannot grant anything during live execution. Bound queue intake before consumption so accepted work is never silently lost. Record batch cutoffs, overflow/backpressure and render/drop decisions.
2. **Transform:** apply inputs to an inactive candidate only. Explicit scene state includes all surfaces and inline text/selection, committed bitmap bytes/geometry, bar/theme/focus, frame groups, cursor/overlay state and visibility/retirement generations. Partial typed transfers and reply state are bounded private protocol state, also advanced only by recorded boundary inputs. Renderer takes an immutable scene reference and never consults mutable global stores or live producer buffers.
3. **Swap:** after validation, select the complete prepared scene/frame slot for commit; retain the old good slot. No pending receipt may be generated merely because some candidate fields changed. Candidate failure leaves the committed scene and externally visible replies unchanged.
4. **Commit:** one output owner performs framebuffer publication and releases eligible effects. Full, damage, top-strip, cursor-only, startup and primary-framebuffer paths must all use this boundary; no bypass remains. Damage rendering must be pixel-equivalent to the full committed scene and must use the correct prior-frame generation.

Allocate scene/frame backing at BOOT or as bounded Display-owned static storage; never allocate new runtime authority or expose another capsule's backing. Measure the added image, stack and physical-page budgets before rollout. Rendering into private backing avoids speculative framebuffer writes. A CPU copy to the current framebuffer is still a non-atomic presentation backend: this first slice establishes Display logical scene isolation, **not** tear-free or complete observable epoch compliance. Hardware latch/flip completion is a separate required stage.

## Ownership, causality, close and receipts

Keep immutable `(held slot,target,task,surface,coordinator)` bindings and permanent retirement tombstones. Attach ownership to every candidate operation; do not rediscover a “current app” at commit. Preserve each sender's order, including create/text/bitmap/destroy and typed transaction sequence floors. Never sort by PD or opcode to manufacture determinism. Existing IPC lacks a unified causal sequence spanning IPC and UiIR; a recorded intake order is a replay input, not proof that cross-channel ordering already meets the architecture. Migrate producers to an explicit sequenced envelope/UiIR operation stream in a subsequent bounded step, retaining BOOT-held access.

Typed text/bitmap completion must remain distinguishable from presentation. A staged commit becomes a retained candidate operation; existing Applied means committed content, with `pixels=unconfirmed`, and must not escape before that content's logical commit. Preserve immutable pending replies on Full and historical idempotence without replaying mutations. OP_APP_PRESENT requires publication of the matching committed scene; it must not join a newer viewport/current payload accidentally.

Authoritative death fences subsequent input immediately in transaction intake order. The cleanup candidate removes the subject and all references. ACK only after the removal frame is published **and** old scene/frame/cursor/protocol snapshots can no longer redraw or reference it; scrub retained owned copies before releasing cleanup custody. Never clear an old slot while a renderer holds it. A failed publish/Full ACK retains the exact pending cleanup, never resurrects the subject. This is especially important once multiple scene/frame slots exist.

## Meaningful proof and remaining release blockers

Compile the actual reducer/renderer with finite input, authority and output-boundary doubles. Test failure before swap, publish failure, Full replies, retirement between chunks, old-frame cursor restore, forged/mismatched task, partial bitmap/text, mixed-sender order and resource exhaustion. Replay identical recorded batches into fresh state; compare canonical scene **and protocol state**, effects and pixel bytes, not just a counter. Mutants should expose early framebuffer writes, premature Applied/ACK and current-owner rebinding. Native close/relaunch, all panes, TLS/bitmap/selection and clock proofs then exercise the integrated sole output path.

Before complete Display epoch qualification: real UiIR rendering operations, unified causal sender sequences, deterministic admission/drop policy, complete input capture and replay, bounded snapshot lifetime, BOOT buffer grants, vblank/triple-buffer/latch completion and interrupted-output recovery must exist. Logging only metadata/hashes cannot reconstruct pixels. A replay recorder containing private document/network bytes needs its own sanctioned custody/retention boundary; diagnostic serial is not that authority.

Before system-wide closure: kernel scheduling/interrupt/device completion order, memory/fault/reclamation transitions, every service state and external I/O, authority-result observations and BOOT genesis must enter the same explicit replay model. Complete replay must reconstruct epoch zero and compare all observable state/effects; Display-only replay cannot close either full-system requirement. Keep GLOBAL_VAS, immutable BOOT genesis and sex-pdx authority unchanged throughout.

## Round166 checkpoint and next implementation boundary

[Round166](DISPLAY_SCENE_OWNERSHIP_2026_11_01.md) qualifies Display-owned
composition inputs and bounded lease release. It does not implement the remaining
transactional engine described above. The next boundary is BOOT-sanctioned frame
backing sized from actual geometry, with checked extent/budget and typed held
authority, followed by one prepared-output publication owner for every rendering
path. Avoid reserving two maximum8192×4320 images without a measured memory plan.
Keep CPU framebuffer publication's non-atomic behavior explicit until hardware
latching exists. Candidate/effect intake and complete replay remain subsequent
required work, preserving causal input order, Full uncertainty, exact owners and
cleanup custody. Current source still computes legacy blur scratch; eliminating
that measured redundant work can improve responsiveness alongside this boundary.

## Round167 checkpoint and next batch boundary

[Round167](PRIVATE_FRAME_PUBLICATION_2026_11_01.md) qualifies geometry-sized
BOOT-held private backing, disjoint preparation/abort, one checked CPU output
function and post-publication presentation bookkeeping. Each current rendering
invocation still has its own scene/publication. Round168 must coalesce one
post-drain content/cursor/clock update batch into one captured scene and one
publication, preserving partial-baseline correctness and exact cleanup effects.
Avoid recapturing mutable globals between passes of the same batch or publishing
an intermediate cursor/content frame before its strip update. Transactional
intake/effect commit, unified causal/replay inputs, interrupted-output recovery
and hardware latching remain required afterward; two private frames and SFENCE
close none of these complete-system obligations.
