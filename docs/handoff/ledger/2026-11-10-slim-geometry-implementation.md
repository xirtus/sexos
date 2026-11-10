# Approved slim geometry: coordinated implementation (2026-11-10)

Andreas approved the visual concept and shared-F2 plan in this chat on 2026-11-10.
This implements its geometry stage after shared metrics #139 and applied-scale
receipts #141. This entry is an implementation checkpoint, **not acceptance**;
initial and combined native correctness evidence is recorded below; final complete
host/CI checks and isolated latency comparison remain outstanding. Never substitute the generated concept for a guest
capture, or infer a performance improvement from the smaller bar.

The live bar is 28 logical pixels including its separator; titlebars are 26,
control targets 24, and natural Geist Sans UI is 13 on a 16px line. The 8px resize
band and 6/8px gaps transform with the applied scale. Existing palette tokens remain;
controls get restrained circular marks with a contrasting fine edge. Whole-panel
rounded masks remain the next separate stage. Fixed-cell native body labels still
use Geist Mono until their typed natural-label path lands; editor/terminal cells keep
Mono deliberately.

One model reflows the bar for Shell clicks and Display. It retains nine workspace
targets at ordinary widths; compact layouts expose a recognizable nine-dot Atlas
button with the same keyboard workspace routes. Status, Bell and real UTC/date remain
reachable. The narrowest complete model is 320 logical pixels. Shared fit policy
reduces a requested/manual/EDID scale that cannot fit, rather than crashing Display
or hiding offscreen targets. Smaller unsupported modes log that limit and preserve
keyboard access with the existing bounded model; fitted/visible pointer targets are
not claimed below that supported minimum. Independent overlap sweeps cover logical widths 320..4096 at 1-3x.

Shell derives workarea, frame controls, drag exclusion, minimum sizes, tiling gaps,
child bounds and pointer/caret origins from the applied contract. It waits for the
post-publication scale receipt before adopting it, refreshes native views and program
resize events, refuses scale changes during held-button/drag gestures, and consumes
pointer input during the outstanding handshake while keyboard input remains live.

Existing held Shell-to-native viewport notices carry scale in the high byte of the
packed dimension word; height remains bounded to 4096 and invalid scale is rejected.
Quil, Spindle and the native Browser adopt it only after the authenticated notice.
Document hit layouts and Browser reader state retain their scale. Programs receive
actual pixel viewport dimensions over their existing resize event; no ambient monitor
or framebuffer authority is added. ABI input pins must be regenerated for this change.

Display captures scale in FrameGroups/SceneSnapshot. Scalar/scanline text, bitmap and
program viewport paths, controls, UI labels and cursor helpers consume those frozen
metrics; a later service-global scale cannot change a retained scene's geometry.
The single-line clock formats once per pass from the same admitted civil state and
calendar conversion. It labels UTC, preserves stale/blocked/ended/unavailable states,
and never synthesizes a healthy sample. Full source health/admission logic remains.
The complete 13px time/date advance is at most 125px before trailing padding, fitting
the 136px clock interior without shrinking Geist.

The fixed Sans/Mono cache stays 2 MiB with the existing bounded scratch/slots. UI
font size, baseline and overhang padding scale together. The bar backing bound grows
50->84 rows for 3x: two declared 2560-wide legacy scratch arrays have a conservative
additional bound of 696,320 bytes; owned scene rows add 136 bytes, plus small fixed
metric/notice fields. Linked resident cost must be measured separately because
unused compatibility paths can be eliminated. BOOT/service slot headroom and
actual rendering/cache work must be measured before acceptance. No new
blur pass, animation timer, per-pixel allocation or kernel code is introduced.

Geometry diagnostics are emitted only after a completed full CPU publication and
once per changed scale/mode. Browser fixture helpers assert independent 28/26/8/34/24
logical goldens, then use that published geometry for real input and pixel origins;
coverage thresholds remain. Older 8/36/44 offsets and 50/51 strip goldens require
source-bound oracle migration, not relaxed assertions.

Initial source-only checkpoint (superseded by the combined evidence below):
production shared metric/state cases, 38 Quil view cases, 19
Spindle viewport cases, 33 Browser reader cases, 15 text-readability cases, 13 paint
obligation cases, five clock/layout cases and three bar-layout cases pass. Renderer
composition bodies type-check in their actual extraction fixture; its independent
accepted-pixel oracle is not migrated yet. Some exploratory glyph checks linked the
unchanged sex-font artifact from the accepted font worktree; formal gates will build
that dependency afresh here. No target-build or guest screenshot claim yet.

Future complete candidate builds must supply verified musl headers for real C hello,
not its fallback stub. The known headers are in the integration worktree's
`target/musl/include`; record actual compiler/tool hashes (Apple Clang 21 or Homebrew
23.1.5 with the separate unused-flag fix), ISO/source hashes and build flags. The UI
build may omit FFmpeg/libc++ only if its evidence explicitly excludes media proof.
Kernel growth target remains zero under the 21,500-line guardrail.

Review corrections: Damage captures its bar clipping boundary explicitly and a
completed PublishedFrame retains the Scene bar boundary for transaction coverage.
The 14-case plan suite includes a captured 1x boundary followed by live 3x mutation;
coverage stays bound to the snapshot. Published geometry readers discard incomplete
serial records, with truncated-prefix/CR-only and independent scaled-golden controls.
The separate 13px Sans atlas supports an exact initial-gradient clock oracle; this
is pending native qualification and does not assert arbitrary wallpaper/night-light
pixel coverage. Clock draw stamps now include target geometry so an unchanged civil
sample repainted after scale adoption still produces a descriptive draw observation.

First native checkpoint at source `97d433ce` built successfully from a fresh Rust
target (only bootloader artifacts copied). Actual 1280×800 and1024×768 captures and
provenance are in `docs/handoff/assets/slim-desktop-first97d/`. Both exact3456-pixel
13px Geist clock oracles passed; native 2560 scale 1→2→1 receipts/ink passed, and a
separate2x clock/geometry capture passed13824 pixels. ISO `fd75e3415c457806d87e956be04d5e3e4a84b5f3824877ab37ec506ec93f963f`.
Display ELF sections were text639306/data1269220/bss2461619 bytes (4370145 total);
BOOT reached and rendered the native desktop. This is a source-bound correctness
checkpoint, not final input/browser/CI/performance acceptance. All guests stopped.
The civil-clock source/health/authority suite retains26passing cases with the
single-line target and adds unchanged-sample/new-target draw-stamp coverage.

Host migration and remaining geometry corrections:

- All 55 compositor cases, 22 bitmap cases, binding/store controls and compiled
  ownership/snapshot mutants passed after independent reference fixtures pinned
  the visible 28px boundary instead of confusing it with the 84px backing capacity.
  The first 41/55 failure log is retained in local evidence. No timing claim follows
  from the correctness run (it overlapped another correctness build).
- Scene 23 cases/15 runtime mutants/source fences and the output pipeline with all
  existing acquisition/publication/dirty-plan/source mutants passed. The additional
  captured-boundary paint case is counted explicitly (14 cases, not 13).
- Toolbar contrast checks cover the actual new title region after the 24px controls,
  keep nominal ink contrast>=4.5 and compare AA pixels in all rendering paths.
  A natural 13px lowercase glyph need not contain a fully opaque pixel; the
  independent title stem/TTF and real native readability evidence remains required.
- The command palette previously used old 36/44 offsets and unscaled row metrics.
  Its logical 480x280 box now fits/centers within the shared scaled workarea with
  an 8px logical gap; its view receives explicit scale/top/inset data. Independent
 1x/2x/3x rectangle and visible-selection cases pass. This preserves its existing
  keyboard workflow; the next panel/input stage remains separate.
- Browser TLS/calendar/import/image panel builders also still used28+8 and base
  font scale. They now derive geometry from the authenticated applied scale and
  shared 26px title. Image 1:1 remains physical pixel 1:1; UI scaling affects chrome
  reservation and Fit space. Reader source-byte anchors remain unchanged.
- Linen's public scale field now reports actual text scale (base times UI), matching
  its already scaled layout. Current-source native refresh is required to qualify
  the corrected metadata, rather than editing old captured traces.
- Reviewed Browser wheel source lock changes only the body rectangle transform;
  all exact held association/custody/route locks stay intact. Source-shape and
  compiled main-prefix controls retain pending-scale and foreign-sender refusals.
- The TLS pixel oracle now uses the actual source-selected Geist Mono AA atlas
  instead of assuming a scaled ROM grid. Full canvas/erasure checks and adverse
  receipt/hash controls remain. DEMO 9px clock evidence is explicitly archival and
  cannot qualify a current slim-metric native trace.

Source-bound first97d captures remain first-checkpoint evidence, not qualification
of the later palette/panel corrections or the final merged capture integration.
Whole-panel masks and natural Sans app-body labels are still the next topic.

Further host checks passed before the capture-master refresh: the full Python
suite 1163 cases, Browser view 21 cases, image 32 cases, calendar 19 cases with its existing
measure/epoch controls, plus isolated-process actual panel layouts at UI scales 1/2/3.
Clipboard, cursor, activation and civil host wrappers passed with negative controls.
The Linen delivery fixture was repaired with independent literal origins/heights,
an explicit scale 1 reset, and scale 2/3 retry/readiness checks; its full gate passed
56 outbox cases and 20 production-wrapper cases. These are scoped host results; the
at that earlier checkpoint, the final combined source still required fresh native
and CI qualification.

The ff0 host CI run exposed one remaining stale Quil clipboard fixture: its
explicit document layout omitted the new UI scale. It now pins scale 1; the
actual editor clipboard/undo gate passes with all previous assertions intact.
The palette frame metadata also tracks its submitted centered rectangle directly,
because the protected keyboard overlay has no app-custody bounds entry.

## Combined native checkpoint: source 17c7f700

A clean committed build includes master 3e075b59, including native screenshot,
encoder and dormant image-source-pin integrations. ISO SHA-256 is
`213007e199bb3cda516abe5ad08c7b8ed54bc99e4153a073e74c3bdc86cde34b`.
Apple clang 21 and the verified integration musl headers built this UI candidate.
Its Rust target was initially empty at the first97d build and remains local to this
worktree; no foreign Cargo outputs were copied. FFmpeg/libc++ libraries are absent,
so this does not qualify media or the full C capability suite.

Actual captures are in `docs/handoff/assets/slim-desktop-final17c/`. Both 1280x800
and 1024x768 pass all 3,456 clock pixels. An open palette keeps its selected last
item visible through scales 1→2→3→2→1; complete glyph/erasure checks pass at every
step. The real clock also passes all 3,456/13,824/31,104/13,824/3,456 pixels from those
same paused serial/framebuffer pairs. Native 2560x1600 resolution plus matching
post-publication applied receipts pass 1→2→1 with text ink 2,280/18,616/2,280.

The actual person-triggered `capture shot.png` route saves, syncs, releases and
extracts a valid native PNG. Its generation-bound reference reports bar 28 and
padding 8; all 833,930 stable pixels match the later live scanout exactly. Existing
window layout, directional focus, swap, floating, workspace, scratchpad and launcher
keyboard actions pass on this same image.

A maximized native Browser receives its held real viewport. Actual L/Q/L input
produces HelloPending/Cancelled/HelloPending; each picture passes the complete
922,720-pixel AA glyph, blank canvas and erasure check. This is UI evidence only,
not live TLS transport or public network qualification. The first attempt exposed
a remaining checker-only archived y>=50 bound; the checker now consumes complete
published bar metrics, with independent y=28 acceptance, y=27 rejection, and partial
record refusal controls. A following attempt correctly failed on 52 actual cursor
pixels at x=640..647/y=400..415. The HID zero-init move was rejected; the final test
waits for an accepted nonzero pointer move into the bar before checking all pixels.
No content mask or mismatch allowance was introduced. Both failures are retained.

Window-manager gap checks likewise use the published bar plus the unchanged 8px
logical gap instead of an obsolete 50px bar constant. Final current-head host/CI
and matched fresh-baseline isolated performance checks remain required. Whole-panel
rounding and natural Sans app-body labels remain the next separate topic.

Matched fresh master 3e075b59 section measurements use the same compiler/profile:
Display text/data/BSS totals 4,370,009→4,382,505 bytes (+12,496); Shell totals
1,031,605→1,039,805 (+8,200); SilkBar remains 48,733. `llvm-nm --print-size` finds
neither legacy `BAR_BG_BUF` nor `BAR_BLUR_BUF` in the linked Display image, so the
conservative declaration bound above is not a measured resident-memory increase.
The bounded font cache remains 0x206018 bytes including metadata. Current-source
BOOT, scale transitions and native capture all succeeded within the service layout.
These size results are not rendering-latency evidence.

The remaining native floating-window driver now selects its drag point inside the
actual published titlebar, instead of y=64 (which is outside the new 26px title).
The full launcher workflow's no-ghost, held pane reconstruction and erasure helpers
likewise use complete published chrome metrics, retaining explicit archived defaults
when absent. Erasure samples source-selected fully covered Geist glyph pixels rather
than a ROM signature, retaining the minimum 100 checked pixels per line and known
Linen-overlap exclusion. All 30 launcher adverse controls pass, including retained
ink at an independent literal 34px current body origin. Actual pointer/native
qualification of these drivers remains pending at this checkpoint.

## Completed pointer and isolated latency qualification

The actual scale-2 yellow control at (90,98) minimizes Quil; the real launcher
restores the same frame. Actual clock clicks at (1120,28) open and close the clock
panel. Palette content at scale 2 passes full glyph checks; after real Escape,
17,135 source-selected fully covered pixels contain no remaining launcher ink
(1,001 known Linen-overlap pixels excluded, with every line still above the
unchanged 100-pixel minimum). The first parking attempt was correctly refused as
`corner_poison_after_ready`; its log is retained. The successful attempt parks at
the existing safe bar point rather than bypassing the HID guard.

The actual corner drag changes Quil 692x756→572x676 while preserving tiling gaps.
The actual title drag floats Quil to x=188 and preserves that position when another
program opens. These are native input/correctness checks, not timing claims.

The matched baseline is clean master 3e075b59, ISO SHA-256
`0e68ed67ef7126e745b75a0771b927e60cbb7d2b661de0024980398fe41e3683`.
It was built in a fresh independent Rust target using the same verified musl
headers, Apple clang 21 binary, no-media profile and all five identical Limine
binary hashes. Both images use the same CPU/RTC/resolution and fresh blank disks.
No other agent's build or guest ran during the isolated traces.

A same-image control of 20 real Super+M layout toggles per boot gives p95 ratio
1.01831 (CPU p95 ratio 1.01853). Three alternating baseline/candidate pairs then
supply 60 events per image after six warm-up toggles per boot. Input-to-observed
pixel p95 is 477.074500→449.747459 ms, ratio 0.942720; CPU p95 ratio is 0.933336.
The unchanged <=1.05 regression threshold passes. The observed point is (1000,750),
a blank body point with identical master/monocle colors on both images. This
includes QMP/snapshot polling and is neither a hardware latch, full-frame timing
nor a universal desktop performance claim. Raw samples, build bindings and the
control are retained in `performance.json`; earlier failed/contended evidence is
not replaced. All guests stopped and were reaped before the window was released.

The full refreshed Python suite passes all 1,165 tests. Its first attempt and CI
caught a removed `glyph` compatibility import still used by the readability gate;
that import is restored and the failed logs remain. The full service custody gate,
including clipboard/Quil/IPC sender checks and sexnet service/jobs/TLS/source-custody
negative controls, passes. Final current-head CI is still required before merge.
No further production changes follow runtime source 17c7f700 in these evidence-only
and checker-fix commits. Whole-panel rounded masks and natural Sans app-body labels
remain the next approved, separate topic.


## Current-head CI oracle corrections

CI 26133ad9 exposed two remaining checker assumptions; both original artifacts are
retained. Runtime's legacy fixed 410x330 Web JS checker still sampled y+36, as did
six related legacy program helpers. They now use the completed independent shared
content rectangle while retaining each original visible extent, sample count and
error threshold. Replaying the untouched CI JS screenshot/reference/serial gives
zero mismatches (worst 0) across the same 3,936 pixels at the actual (714,640) origin.
This does not adopt the separate dynamic Web viewport behavior from PR #152.

App-resource's producer-local Display ACK marker follows the syscall return. The
untouched failure trace shows scene committed→kernel accepted role-2 ACK→private
retirement→Display local return marker. Immediate PDX handoff permits this order.
The checker now requires prepared<committed<actual kernel role-2 ACK<retirement and
local return after that kernel ACK, while preserving all other role, identity,
flush, pool-zeroing and ordering requirements. Twenty adverse checker cases pass,
including late-local acceptance and missing/premature/foreign actual-ACK refusals.
The original CI trace passes the corrected causal check; no production lifecycle
or transport code changed. Compact replay hashes/results are retained separately
from the local 17c image cohort in the adjacent ci261 JSON evidence files.


The refreshed master151 CI passes the resource ACK check, then exposes separate
successor pixel helpers still assuming50/36. Quil successor bounds/font placement
and modern Spindle viewport layout now consume completed shared metrics, with
independent1/2/3x fixtures and archived legacy defaults. Current traces cannot use
the historical four-grid fallback when authenticated viewport text is absent.
The untouched054 successor screenshot matches the actual complete dirty status
`quil-object-v1 * DRAFT KEPT` exactly. Auto-draft had superseded READY while the old
bounds check retried; the close gate now follows only a complete newer status of
the exact same dirty document, with all status glyph checks retained. Partial or
console-prefixed intents, a different filename and a clean status cannot retarget
that expectation. Fresh combined artifact/complete native close-chain proof is
required before the next acceptance claim. Master151 merged cleanly; GNU-sed ABI
regeneration verifies82 inputs. The original failure artifacts remain intact.


## Fresh integrated 151 qualification

Clean source a427b95c includes master 151/d92091cb and 82 ABI inputs. The newly built
immutable ISO is 817e08083f5a72f9614c094f3e4294c3339ed961f6b0d8541056d45261469249.
The full native desktop-close gate passes its complete original/successor document,
Spindle/history/observer/Bell/prompt, reserve-exhausted launcher, survivor filename
and minimize/absent/restored glyph chain. The new 1280 capture passes all 3,456 clock
pixels; native 2560 1→2→1 publication/applied receipts and ink 2,280/18,616/2,280 pass.
All guests/builds stopped before the resource-window handoff. Exact new source,
image hashes and actual native capture are in assets/slim-desktop-integrated151/.
This remains a musl/Apple 21 UI-only profile without FFmpeg/libc++, and does not
relabel the earlier matched 3e/17c performance result as a new 151 benchmark.

The final helper tightening refuses missing/partial/console-prefixed or different/
clean document intents rather than qualifying stale dirty pixels. The successful
native pre-close prefix and exact status pixels replay under this stricter check.
The full refreshed Python suite now passes 1,170 cases. Current-head CI remains the
last merge requirement; subsequent whole-panel/Sans app migrations stay separate.
