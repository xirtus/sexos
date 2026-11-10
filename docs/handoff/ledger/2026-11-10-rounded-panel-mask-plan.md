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
