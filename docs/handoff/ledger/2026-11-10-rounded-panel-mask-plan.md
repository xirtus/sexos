# Next approved UX stage: rounded panels and natural Geist labels

PR #146 is merged as 39a862eb after all 14 checks and native/input qualification.
This new topic starts from that exact master; the approved plan is now executing. This develops the approved 2026-11-10 concept, preserving
its 28px bar, 26px titles, 13px Geist Sans UI, Geist Mono editor/terminal content,
mint/navy/lavender tokens and restrained edges. No further visual approval needed.

## Current source constraints

- `surface_groups::winner` and `row_winners` are rectangular. Row runs assume the
  selected owner is constant between their edges. Rounding only the scalar shader
  would leave square scanline/bitmap pixels and incorrect corner hit ownership.
- Shell `point_in_surface`, chrome hit-test and focused/z-order fallback must share
  the same shape policy; transparent corners cannot intercept lower windows.
- Surface text and native UI builders are fixed-cell. Changing their glyph face
  alone does not produce natural proportional layout and would corrupt clipping,
  reader/selection geometry and independent pixel evidence.
- `sex-font` is GPL-3-only; the driver-facing dual-licensed `sex-pdx` must not grow
  that dependency. Font metrics belong in native UI app code/shared GPL-3 UI code,
  while the PDX wire remains pure bounded data.
- Existing screenshot/capture, ownership, post-publication scale receipts and
  prepared-scene lifecycle hooks must remain intact. No new kernel mechanism.

## Topic 1: rounded compositor and shared hit mask

Use 12 logical px for floating panel/frame corners, capped by half the actual
width/height. Keep the top bar straight, resize band 8px and targets 24px.
Shared SDK pure integer geometry supplies row spans/coverage and the hit predicate.
Apply shape against original signed frame geometry before viewport clipping; a
partly offscreen window must not acquire a fresh rounded edge at the screen border.

Display snapshots the shape/scale in Scene/FrameGroups, clips exact children through
the parent, and uses the same rounded row spans in scalar, scanline, bitmap and
damage paths. AA edge pixels blend with the actual next visible owner, not a
wallpaper-only stand-in. Layer traversal and scratch storage remain bounded by the
existing surface capacity. Interior runs keep the current fast path; no new blur,
per-pixel allocation or whole-row fallback for unaffected body pixels.

Shell's focused and z-order hit paths reject zero-coverage corners before selecting
chrome/content, then fall through to the real underlying window. Resize/control
rectangles stay unchanged inside the visible mask. Apply to actual independent
panels/modal launcher, not separately to Spindle's fitted child panes.

Expected ownership: SDK `desktop_style::chrome`; Display `surface_groups.rs`, a
small pure rounding/render helper plus `main.rs` wiring; Shell `main.rs` hit paths.
Touch Scene fields only if needed to freeze a policy value. Avoid capture/recorder
request handling and preserve their post-commit observers. Regenerate ABI input
pins because shared SDK source changes; no opcode or BOOT-region changes required.

Gates: independent literal radius/scale/tiny/offscreen cases; scalar/row/bitmap
comparisons with partial overlap and child clipping; retained old-scene scale
controls; actual native corner-underlay and corner click-through, drag/resize,
minimize/restore, clock, PNG capture and HiDPI. Preserve all current mutant and
coverage obligations. Fresh matched A/A plus alternating traces must stay <=1.05
p95; record actual linked and scratch memory. Native screenshots at 1280x800 and
1024x768, plus 2560x1600 at 2x, are required before merge.

## Topic 2: bounded proportional native label transaction

Add a typed UI-label role to immutable text transactions, defaulting legacy calls
to Mono grid. Bind font role/layout parameters into the transaction integrity hash;
a retained old frame cannot silently acquire the latest style. Keep actual owner,
live task, capability and Applied checks. Unknown roles, malformed bounds, stale
sequence/viewport and foreign callers fail closed. Audit/reserve an opcode with the
recorder owner before choosing any number; no ambient font/framebuffer authority.

Natural labels use the bundled Geist Sans face at 13 logical px and 16px line metrics,
including actual advances/kerning. Use bounded prepared line/glyph runs, avoiding
prefix measurement in a per-pixel shader. Shared native UI measurement may depend
on `sex-font`; `sex-pdx` stays dependency-free with respect to GPL-3 UI code. Measure
the resulting font/parser copies in Shell/Browser and verify BOOT layout headroom.
Keep the existing bounded 2 MiB Display raster cache and fallback only for missing
characters or failed raster/cache admission; a bitmap font cannot become UI default.

Plain Mono transactions, editor/terminal cells, caret and byte-anchor semantics
remain exact. A panel opts into proportional layout only together with its own
clipping/wrapping and row/hit geometry; never draw proportional glyphs in old fixed
cells while claiming the migration complete. Longer read-only Browser text needs
proper flow/hit/byte-anchor handling in the same migration or its next scoped PR.

Expected ownership: `text_batch`/delivery metadata, Display text transaction/state
and a dedicated label renderer; native UI measurement library/module. Existing
selection hashes and label/Mono role distinctions get independent adverse tests.
Tests must distinguish WWW/iii and kerning/Unicode/ellipsis, verify full bounds and
complete erased backgrounds, and prove frozen roles/scales and foreign/stale refusal.

## Topics 3+: adopt natural labels in native app families

1. Shell Files/Linen and launcher, then Collar/settings and Bell labels. Replace
   fixed column counts with measured logical widths and explicit selected-row/hit
   rectangles. Keep trusted actions, grants and keyboard order; use natural names,
   clear headings/hints and at least 24px actionable rows. Sources are Shell
   `desktop_content`, `launcher`, and their render/delivery wrappers in `main.rs`.
2. Native Browser status/TLS/calendar/import/image chrome and reader text flow.
   Sources are Kaleidoscope view builders and reader layout plus its actual held
   viewport delivery. Preserve real status/transport provenance and reader byte
   anchors; update whole-frame/font pixel models from actual proportional metrics,
   never reduce coverage or claim tiny fallback text qualifies a complete status.

Each family is its own topic PR with source-bound native before/after, keyboard,
selection/navigation, pointer/viewport, 1–3x scale, full current-head host/CI checks
and the relevant capture/performance controls. Mono remains for editor/terminal
content. Round panels alone cannot be presented as completion of the whole concept.

## Implementation notes from the existing row/hit paths

- Retain the launcher's separate modal intake/capture policy. Ordinary window
  corners fall through z-order; rounded paint does not make an active keyboard
  modal release its outside-panel input fence.
- `RowWinners` currently budgets four edges per surface. AA coverage/opaque spans
  and parent body edges must be accounted for with an explicit fixed capacity;
  fast content runs may cover only fully opaque shape spans. Measure any added
  stack/static scratch rather than assuming a larger edge array is free.
- Prefer bounded radius/row coverage lookup or prepared span data. Do not evaluate
  supersampled circles for every interior pixel on every frame. Radius is capped
  at 36 physical pixels (12 logical at 3x); tiny-frame clamping is explicit.
- Preserve APP_PRESENT accounting: a fully hidden underlay must not be marked by
  speculative shading; actual blended contributors may be marked. Damage repaint
  still writes the entire old/new bounding obligation, including revealed corners.

## First implementation slice

The shared `Rounded` shape uses 4×4 integer area coverage, with immutable lookup
bytes for the three normal physical radii (12/24/36). The table is 2,016 bytes;
clamped tiny frames use the same bounded rule. It adds no font dependency to PDX.
FrameGroups chooses visible owners and opaque runs against held Metrics and the
original signed frame rectangle. Exact children retain their single parent mask.

Display retains fast opaque content rows. Its separate partial-edge path shades
only actual contributing owners, stops at the first opaque layer and folds AA
front colours over those owners. It uses fixed arrays of 32 colours/coverages
(160 declared bytes, excluding compiler frame bookkeeping); no recursive layer
walk or allocation. RowWinners is 6,192 bytes on this 64-bit host, versus the former
4,144-byte layout: +2,048 bytes. Its local edge array grows by 512 bytes. These
are bounded declarations, not a claim about final optimized target stack size.
The existing 2 MiB glyph cache and capture/post-publication hooks are unchanged.
Final linked footprint and real guest timing remain merge prerequisites.

The legacy launcher146 post-compositor pass also uses the mask. Its AA boundary
reconstructs the actual underlay while omitting its own retained layer, avoiding
a second blend over its previously painted colour. The command palette152 keeps
its existing distinct modal input fence. Shell initial surface/chrome/control
hits use the same mask; captured drag/up handling stays intact.

Viewport clipping now intersects signed bounds rather than moving an offscreen
frame's origin. Frame chrome, child-body selection and scalar/row text keep their
original coordinates; clipping does not invent a fresh corner or shift glyphs.
The 28px bar, 26px title, control rectangles and palette remain unchanged.

Host validation adds a separately expressed floating-point area oracle, literal
AA colours for two/three actual overlapping owners and exact APP_PRESENT bits,
foreign-scene-reference refusal, old-scene scale retention and a whole-frame
legacy overlay oracle. Existing scalar/row/bitmap/damage coverage stays intact.
The first sweep retained its eight failures: square-pixel and clipped-origin
expectations needed explicit migration. Its 47 successful cases were not treated
as acceptance; the updated full sweep and native qualification are separate.

The final signed-origin host sweep passes: 58 actual compositor cases, 22 bitmap
transaction cases, 3 independent rounded-owner cases, binding guards, 17 current
and 17 future-catalog bitmap-store cases and all existing compiled negative
controls. The signed glyph migration exposed a real changed-row damage issue:
`max(y,0)` could move a clipped-top row's damage below its visible ink. Damage
now uses the original signed origin too. An independent literal 36..45 row bound
and a full-frame before/after repaint comparison cover that correction. Separate
shared geometry validation passes all 12 cases. Initial failures remain retained
locally; host pixels still do not substitute for native screenshots or timing.

Source review found an initial-hit boundary gap: local corner coverage alone did
not reject a signed frame extending behind the protected bar or beyond the
screen. `Metrics::panel_hit` now requires the current applied workarea/screen as
well as rounded coverage; Shell surface/chrome/light initial selection uses it.
Captured drag/release routing is untouched. Literal negative/outside/bar-crossing
controls at all three scales compare this predicate with actual FrameGroups
selection, and tiny/zero screens fail closed. The shared geometry suite is now
13 cases; all 3 owner/row controls also pass with this agreement checked.

A focused production-compositor control also fills all 32 slots with overlapping
partial corners, requires the last real owner to be visited, then checks an
opaque pixel stops before that owner. Literal colour and APP_PRESENT results
pass. This proves capacity/stop correctness, not worst-case guest timing or
optimized target stack size; those remain pending. The focused current-source
AA/legacy-overlay/signed-row controls pass after the workarea fix and heap154
master refresh; they are not relabelled as another full 59-case sweep.

## Conditional first native candidate and ordering correction

The first clean optimized build was source441362dd, based on merged Web152 and
heap154. Its immutable UI-only ISO SHA256 is
`0c70f79fa97b8365bfbe7b0d2851e54e5f908f1a4d43b400622222eaabc6aa9f`.
Apple Clang21, verified musl headers, the five pinned Limine files and a fresh
worktree-owned Rust guest target were used; no FFmpeg/libc++ qualification is
implied. Native1280x800 and1024x768 captures pass the full3,456-pixel real UTC/
Geist clock oracle. These are first-candidate observations, not final acceptance.

The independent native corner oracle initially sampled old published geometry
while Display was preparing a new private frame. Every failed image was retained;
bounded re-observation keeps the exact coverage/colour checks. The opaque close
circle also needed its independent circle/contrast calculation. Replaying the
retained native before/moved images then matches all144 corner pixels:23 zero,
17 partial and104 opaque. No opaque-control pixels were excluded to get a pass.

Initial pointer qualification exposed a real defect: the normal image's trusted
`[shell.click.real.target]` reports Mesh202 at normalized(458,264) in a transparent
Quil corner, where the visible underlay is Linen. Optional proof-profile click
markers are absent in normal builds and cannot substitute for that event. Shell
previously tested a private hardcoded order and admitted uncreated placeholders:
Mesh/Collar/Bell have initially Visible lifecycle labels and alive booleans before
their frames exist. Display instead selected nonfocused owners by slot order.
The new holes make this existing discrepancy directly reachable.

`desktop_stack` now supplies one immutable catalog+builtin order and constant-time
bounded rank table. It covers every program/native incarnation and does not let
slot reuse/restart change priority. Shell uses the same inventory for initial hits
and invalid-focus fallback. Eligibility now combines held registration/liveness,
scene membership, an actual accepting active-tab frame, or the explicitly owned
mapped standalone100..103 path. A lifecycle label alone cannot make a placeholder
hittable. Child focus aliases require the exact live custody association and a
real parent frame; modal152 retains its separate trusted fence/priority. Neither
ranks nor caller-supplied fact structs establish capability authority.

Independent controls compare three overlapping frame identities under shuffled
slots, slot clearing/reuse, a bound child focus, every catalog program and trusted
versus foreign modal ownership. The actual extracted Shell adapters additionally
refuse uncreated placeholders, inactive tabs, dead/closing/foreign child bindings
and absent parents, while preserving owned mapped standalone input. The four
positive Rust cases run serially (`--test-threads=1`); four compiled mutants and
two source-wiring counterexamples must fail. Initial small suites pass; full
current-source compositor/font/output and new native three-window proofs remain
required before merge.

## Cold-font stack qualification

The frozen441 ELF has a64KiB mapped user stack. Independent disassembly audit found
warm AA/kerning direct chains of57,320 bytes and a conservative cold path of62,968
for the exact bundled glyf fonts, before small outgoing/indirect allowances. A
66,840-byte generic CFF branch is unreachable for these two fonts, and first-strip
font warmup helps, but neither fact is a blanket future-font stack proof. The441
candidate is therefore not labelled fully stack-qualified.

Cold `Font::new` construction is now an out-of-line non-inlined initializer. Warm
font calls borrow immutable cached data, avoiding cold parser temporaries inflating
every glyph/layout frame. Font bytes, glyph output, cache bounds and service stack
budget are unchanged. Rebuilt target call-chain accounting and exact-font/native
pixel controls are required. IRQs use TSS RSP0 and syscall entry switches to the
kernel stack before software pushes; those frames do not use Display's user stack.
Final matched timing must qualify the ordering+cold-init candidate against fresh
master5f946c33 with identical build/runtime inputs, not relabel441 or older runs.
