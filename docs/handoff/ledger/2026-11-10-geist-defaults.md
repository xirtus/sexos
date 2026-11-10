## Geist remains the desktop default (2026-11-10)

- **Problem:** Display already drew window/panel titles in Geist Sans and larger native
  text in Geist Mono, but compact scale-1 surfaces reverted to the legacy bitmap font.
  Civil/demo clock labels and Bell counts also always used the bitmap ROM. Andreas
  explicitly reaffirmed Geist as the desktop's default; historical pre-Geist screenshots
  are excluded from the UX plan and all visual comparisons.
- **Change:** both scalar and span text paths now try cached Geist Mono at every native
  text scale. Clock and Bell labels use proportional 9px Geist Sans with natural
  advances and kerning. The clock keeps its 80x22 hit box and two 9px caption rows;
  Bell centers its count inside the existing status module. Coverage blends over the
  composed pixel with the existing contrast-ink helper. The ROM is only a fallback
  when the font cannot supply a glyph or a cell cannot be cached. Existing title metrics,
  text cells, clipping, selection, pointer targets, window geometry and palette remain.
- **Role limit:** native Shell text delivery is still a fixed-cell pipeline, so Files,
  launcher/panel bodies and the old Browser reader use Geist Mono there. Existing chrome
  titles and status captions use proportional Geist Sans. A proportional
  label/body delivery pipeline is a follow-up, not claimed by this fix.
- **Bounds and measured cache fix:** the fixed 2,048-slot arena grows from 512 KiB to
  2 MiB (+1.5 MiB of Display static backing, never thread stack). At the new maximum
  90x135 cell, 95 printable glyphs occupy about 1.15 MiB. A host probe of eight mixed
  ASCII passes found 17 resets with the old arena and repeated warm raster work around
  16.5–17.8 ms per 95-glyph pass. The 2 MiB arena had zero resets, with warm lookup
  batches below the probe's 1 us resolution. These are host lookup/raster measurements,
  not guest frame times. A real-cache regression checks eight warmed mixed-paragraph
  passes and ordinary 6x9/12x18 text with no further resets. Other larger working sets
  retain the bounded reset policy; no universal warm-cache or frame-rate claim is made.
  Scratch height grows from 96 to 144 so #77's maximum 3x screen/5x text zoom stays
  Geist in its 90x135 cell (+24,960 bytes of fixed Display scratch). Each draw still
  clears only its actual rectangle. Mono cells, 9px captions and 17px titles have
  distinct size/family keys. There is no font-file loading or paint-time allocation.
- **Evidence:** host font/cache/raster tests (7), Display clock/caption tests (8), civil model and
  Display tests (26) plus three rejected defect controls, and clock pixel-oracle tests
  (18) pass locally. The latter now freeze a source/font-bound grayscale Sans atlas and
  compare all clock pixels, including erased gaps; archived ROM-profile replay remains.
  The 55-case compositor regression and 1,142-test Python suite passed for the initial
  correction; both are being repeated after the proportional-caption refinement.
  Fifteen readability tests now expect compact Geist coverage rather than ROM pixels.
  Clock tests exhaust every time of day and a superset of dates from 2020 through 2106
  for width bounds, and check Bell count clipping. UI host tests run one thread, matching
  Display's single-task glyph cache. New native screenshots and exact clock/grid pixel
  proofs are required for this final caption version before visual acceptance.
- **Visual review:** the first 5x7 cell-fitted Sans clock was rejected as too faint,
  despite exact pixel matches. It is superseded by natural 9px captions and higher
  contrast ink. Provisional captures remain diagnostics, never final UI evidence.
- **Test maintenance:** clock layout fixture used a stale six-slot boundary despite nine
  workspaces; it now derives the status start from `WORKSPACE_COUNT`. No geometry changed.
- **ABI / kernel:** none. No kernel growth. Chrome polish is a separate topic.
