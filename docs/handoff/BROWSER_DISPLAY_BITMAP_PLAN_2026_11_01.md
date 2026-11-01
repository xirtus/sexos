# Round 151: Display bitmap binding readiness

Read-only source review. Actual Browser remains PD15 with content surface300. No successor rows/grants, builds, guests or product edits are part of this preparation.

## Current authority and lifecycle path

- `servers/sexdisplay/src/bitmap_transaction.rs`: `OWNER=15`, `SURFACE=300`; `Stage::EMPTY` starts an already usable store. `receive(owner,sid,request)`, `reset_surface(owner,sid)`, `retire(owner)`, `sample(owner,sid,x,y)` and `poll_reply` independently use those constants. Stage holds staging and published pixel arrays, coverage, a sequence fence, completion identity, pending receipt and permanent retired flag.
- Display has no `APP_CUSTODY` variable: its actual held registry is `surface_lifecycle::Lifecycle`. `main.rs:2802 load_lifecycle_catalog()` queries every held lifecycle slot, registers target/task/surface/coordinator and frame groups, then requires the15/300 entry. `_start` calls it at2925, before the message loop. `surface_lifecycle.rs:125 register` checks nonzero identity and collisions; it does not by itself require Browser family or the canonical coordinator3.
- `main.rs:3334–3350`: lifecycle exit hints trigger capability-status cleanup; ordinary messages are fenced by cached `caller_live(pd)` and `sid_live(sid)`. These are PD/surface phase checks, not task authentication; unregistered PDs are not globally rejected.
- `main.rs:3895–3904`: bitmap dispatch requires15/300 and an active surface with matching owner. It does not compare `msg.caller_task` to the registered task. `Stage::receive` has no task argument. Thus a different task inside the same PD passes this boundary if it can invoke Display; held main-task metadata must not be described as per-request authentication.
- `main.rs:586 bitmap_color` also gates15/300, then clips to actual winning surface/body and calls Stage::sample. Keep clipping and straight-alpha blending unchanged.
- Live create/destroy paths call `reset_surface` at3660/3819. `prepare_app_cleanup` at2822 attests exact target/task and Display role, prepares lifecycle tombstone/removes surfaces, and calls Stage::retire at2837. `finish_display_transactions` renders cleanup before service ACK. Stage receipts are polled at4191 after display transactions.

## Smallest safe next implementation

1. Add a private immutable bitmap binding containing lifecycle slot, target PD, actual registered task, content surface and coordinator. Construct it only in startup from Display's successful held catalog query, checked against `app_catalog::by_surface`, Browser family and exact coordinator3. Require nonzero task; never task==PD. Validate surface fits nonzero u16 because `bitmap_batch::Applied` encodes a u16 surface. Catalog metadata alone is descriptive; the held query is the authority source.
2. Replace the preauthorized EMPTY singleton with one uninitialized optional store, initialized exactly once before the loop through `Stage::new(binding)` (fallible checked constructor). No first request may initialize it, no default15/300 fallback and no public rebinding method. Missing/malformed held metadata stops startup or leaves the bitmap lane unavailable. Audit initialization's approximately18KiB value construction for stack/copy costs; keep backing statically reserved, no runtime allocator.
3. Pass actual `caller_pd`, `caller_task` and sid into bitmap receive; compare the entire immutable tuple before any coverage, sequence, poison, pixels or receipt mutation. The dispatcher must additionally attest the bound held lifecycle slot is LIVE and the surface is active with exact owner. Keep `network`/provider authority unrelated. Use the binding for sampling selection, receipt destination, applied surface encoding and diagnostics; remove independent15/300 selectors.
4. Internal reset uses exact bound owner/surface after existing lifecycle admission; terminal retirement should accept exact target/task from the verified cleanup proof. It must not retire another binding. Preserve the outer lifecycle/SID fence and render-before-cleanup-ACK order.

## Semantics that must survive unchanged

- Same-sequence conflicts poison staging; incomplete/hash-invalid transactions never publish.
- Live destroy/recreate scrubs both pixel arrays and coverage but retains sequence floor and a pending historical publication receipt. That receipt reports past publication, not currently visible pixels. Exact duplicate Commit can requeue the same historical receipt; no republish.
- A full reply queue retains the exact bound destination/surface/sequence and blocks a new sequence. Do not move or reinterpret it when focus/surface state changes.
- Terminal retirement scrubs staging/published data, drops pending historical receipt/completion and permanently rejects future messages; reset cannot unretire it.
- Pixel sampling cannot expose another owner or a retired store. Existing surface/body/framebuffer clipping remains independent of payload geometry.

## Later30/31: separate phase, not active rebinding

Reserve distinct BOOT image identities and typed lifecycle grants first. Each Browser incarnation needs its own immutable bitmap binding, sequence/receipt/tombstone state and backing lifetime. The simplest bounded expansion is one statically reserved Stage per Browser row: two2304-pixel u32 arrays plus144-byte coverage, roughly18.2KiB per Stage before small metadata; three stores require roughly55KiB. Do not overwrite the15 store with30 while its old receipt exists, and do not erase the15 retirement tombstone. If backing is ever shared for budget reasons, require explicit terminal scrub/detach and retain per-incarnation metadata separately; this is a larger design than constructor binding.

Replies use accepted invocation credits to the bound PD; do not copy credits, grants or old pending receipts into a successor. Tests for future identities may use synthetic checked bindings but cannot claim actual30/31 BOOT availability. No process VAS, runtime minting or resurrection of terminal identities.

## Test/call-site impact and scope

- Adapt `tests/display_bitmap_transaction.rs` and `tests/browser_bitmap_delivery.rs` (many `Stage::EMPTY` constructors and receive/reset/retire calls). Keep all existing duplicate, conflict, chunk coverage, alpha, clipping, full-reply, destroy/recreate, historical receipt and permanent retirement controls.
- Add unequal task15/150 positive; samePD/wrongTask, zero task, wrongPD/sid, malformed family/coordinator/oversize surface, missing/unbound state, forged first message, attempted post-retirement reuse, and wrong-task retirement negatives. Failure must leave pixels/sequence/receipt unchanged. A compiled task==PD mutant and removed-task-guard mutant should fail.
- Extract/test actual startup binding and dispatch checks, not merely a model constructor: assert held query precedes construction, exactly one initialization, every bitmap opcode passes actual task before Stage effects, sampling/reply use the binding, and lifecycle proof supplies the retirement tuple.
- Adjacent Display create/text/fill/presentation handlers remain PD-based (`main.rs:3358`, surface lifecycle admit/request_present, text dispatch after3910). This bounded bitmap fix does not establish per-task authorization for those other protocols. Preserve this limitation explicitly; no claim of hostile-code/SMP containment.
