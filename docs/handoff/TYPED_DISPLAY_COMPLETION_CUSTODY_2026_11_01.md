# Round 169 — publication-fenced typed completions

Status: **qualified**. Source `78227603f512a1a6c531542b4779c73168d628c3`,
following qualified Round 168. Architecture remains GLOBAL_VAS addressing,
BOOT-only immutable capability genesis, sex-pdx authority and PKU enforcement.
No Unix/POSIX model, per-process address space, runtime capability mint,
new kernel policy or task-addressed reply transport is introduced.

## Behavior

Typed text Commit and bitmap Commit/Clear used to become immediately replyable.
Main polled these replies even when preparation/publication failed. A newer text
Begin could also erase a completion retained under reply backpressure.

Completion custody now separates **unreleased**, **eligible** and delivered work.
Every newly sealed completion forces a Full output obligation, independently of
pixel damage. This includes identical/empty text, empty Clear, offscreen content
and a live bitmap reset that already erased its pixels. Only the just-finished
successful Full CPU publication, after SceneLease release and without intervening
intake, makes pending receipts eligible. Failed, absent or partial publication
and a held scene release nothing. This is a conservative settlement boundary;
it does not assert intermediate or occluded bytes became visible.

Eligible replies retain their exact owner/SID/sequence through every nonzero
legacy transport status. Reply backpressure requests no additional rendering and
never replays payload mutation. New multipart text work cannot overwrite either
pending phase. Each owner gets its own retry opportunity. Live bitmap reset
scrubs pixels/geometry but retains receipt custody and the sequence fence;
terminal exact-task retirement cancels every phase before cleanup ACK.

Text stages retain the actual admitted caller task for multipart correlation and
retirement. Continuations must match it; zero/crossed tasks cannot alter an
existing transfer. Kernel syscall29 still routes to the **PD's shared reply
queue**, preserving its existing reply-credit rules. This adds no exact-task
reply isolation. Browser's held exact-task admission and other text surfaces'
existing PD ownership policy remain unchanged. Typed SIDs above 65535 reject
before mutation because their Applied wire receipt cannot encode them.

## Verification and provenance

- All **46** frozen registered host checks and **1,132** Python tests pass.
  Retained stdout logs record **3,198** successful Rust executions, including
  repeated SDK fixtures; this is not a unique or exhaustive test count.
- Actual text Stage 20 cases and bitmap Stage 22 cover pre-release silence,
  pending/newer Begin retention, reset/retirement and exact retries. Existing
  Quil 34, Linen 56, Browser bitmap 26 and asset 9 integration cases use explicit
  simulated publication boundaries rather than treating packet admission as
  output. Their host callbacks are not native pixel proof.
- Actual Main/Scene 16 cases and 11 compiled defect controls plus two source
  controls exercise Full qualification, both stage families, held-scene
  withholding, independent owners and terminal cancellation. Compositor 49,
  cursor 5/2compiled controls and frame 38 cases remain. Frame controls reject
  nine runtime defects, four ownership/compiler defects and seven source defects.
- Qualified current cohort: **11 boots / 70 captures**, comprising one sealed
  build boot plus 10 workflow boots:16 clock/date,34 connected/cold-recovery and
 20 Browser captures across 1024×768/e1000 and 1280×800/e1000e. All frozen fresh CLI
  checks and full clock pixel API replays pass. Eleven BOOT inventories include
  actual task/key and private-frame genesis/held checks. Aggregate release logs
  are additional service-boundary witnesses, not sender-to-visible-pixel proof.
- All **1,224 frozen files /12,256,106 bytes** match exact Git blobs and manifests.
 581/586 prior production files are unchanged; five files (four Display files and Browser Main) change and
  none are added. Kernel and all 67 ABI input bytes remain identical to168.
  Nine ISO ELFs contain **325,157 decoded instructions /0 matched FP-vector**.

The first frozen candidate `06dc938b` attempted all 46 host checks but failed
`service_custody` and `linen_text_delivery`: older client fixtures still called
the previous Stage API. Development gates also found old bitmap receipt
expectations; a bare Browser fixture invocation lacked its extraction environment.
These attempts are retained **UNQUALIFIED**, including the diagnostic sealed boot.
Its coordinator stopped before any native workflow guest.

Candidate `22a5c362` passed all host/Python gates and all four compact workflows
(35 retained captures). Its wide normal clock run exposed a real Browser race:
ordinary viewport handling abandoned a transport-admitted old text Commit. The
old Commit arrived after resize; Display correctly retained its completion, while
the replacement Begin had been sent prematurely and could not complete. T input
and the Ready clock snapshot both arrived, but no matching clock Applied did.
The failed one-capture trace and all earlier passes remain diagnostic evidence.

Browser now preserves its immutable active text batch across live viewport
changes, invalidates its render cache, and coalesces the newest desired layout
behind the exact historical Applied. No extra input delay or runner synchronization
masks the race. The actual Outbox/Stage regression covers delayed Commit, three
viewport updates and retained Full replies; injecting the old cancellation compiles
and fails the exact test. Main call-site wiring rejects two source defects. Browser
fixture 21 cases and three compiled controls run through the registered gate.
Terminal Close, explicit reset fences and known transport rejection retain their
existing cancellation behavior.

The current source was resealed as `78227603` and its entire cohort independently
qualified. Diagnostic attempts comprise eight executed boots (two sealed builds
and six B native boots) and 36 retained captures; current qualification adds
11 boots/ 70 captures, for 19 executed boots and 106 retained capture artifacts
across this round. Repeated screendumps to one filename are not counted as extra
retained captures. No failed evidence is overwritten or accepted by rerunning an
unchanged flow; no acceptance transfers from a prior candidate.

Sealed ISO SHA256: `7aeba7aa436c44a988b503f958cf189e58aea5cfb4a5e154da1acc967d9373f0`.
Kernel SHA256: `26bace17d464d1ea04667a6ff1dc47514d76be472342ca4ed396000f62a20f89`.
ABI67 SHA256 remains `3b98b23398b7a309a74d5d10526e9234fa372ff448c79f51aa4f4b9f44fdc36a`.

The first packaging preflight expected only the former two static symbol rows;
its assertion rejected the newly measured text/bitmap rows. The exact failed
helper/log are UNQUALIFIED. The corrected packager verifies all four actual ELF
symbols; native evidence was neither changed nor rerun for this metadata fix.

## Cost and limits

Display ELF is 288,680 bytes (+1,960). Its16 text stages occupy 9,088 bytes
(+128 total,568 each); three bitmap stages remain 55,992. Static scene 59,584 and
frame-store metadata 88 are unchanged. Each boot still reserves 8 MiB physical
buddy backing for two geometry-sized private frames. Post-seal kernel heap is
11,851,408 used / 256,584,048 free; app-private inventory is 2465 pages. These are
not disjoint total-RAM measurements; physical free frames remain unmeasured.

Measured local prologues are unchanged: `_start` 4,696, `raster_batch` 2,344,
`render_prepared` 1,304 bytes, each plus 48 register-push bytes. They exclude
transitive callees, interrupts and dynamic stack. Full settlement may cost more
than a damage-only repaint; no speedup or hard real-time claim follows.

Raw archive: **24,161,453 bytes / 4,332 members**, SHA256
`b27c0a926b5c59684b118981f9b1451c7b3c5cce84e96fc14b3d62182addbabf`; packager and Root independently reopened every member and
verified its byte count/hash. Both qualified and explicitly marked diagnostic
sources/logs are retained. [Raw evidence](assets/sexos-completion-custody-2026-11-01-raw.tar.gz).
[Desktop preview](assets/sexos-completion-custody-2026-11-01.png) is a lossless
conversion of the qualified compact connected-session capture, not an extra boot
or screenshot.

Producer intake still mutates canonical state before output. This slice is
completion-effect custody, not transactional source rollback, a causal input
stream, complete epochs or deterministic replay. CPU copy/SFENCE is not a GPU
latch or atomic hardware flip. No current 1800s soak, Alpha, daily-driver,
full-all-app reserve, hardware/SMP, hostile isolation or physical-erasure claim.
Obligations remain **2 Open / 28 Partial / System Proven false**.

## Next substantive slice

Round 170: project sealed pending typed text into the owned scene, then commit
canonical Surface text only after successful publication and lease release.
Retain data on failure; reuse the actual UTF8/selection/layout routine. Define
or fence a later legacy text write so an earlier typed candidate cannot override
it. Preserve lifecycle cancellation, authority checks and current receipt scope.
Bitmap/bar/legacy transactionality, causal capture, interrupted output, hardware
latching and full replay remain further work. Under the active 80–200 plan,
accepted 80–169 is 90/121; 170–200 remain 31 substantive rounds.
