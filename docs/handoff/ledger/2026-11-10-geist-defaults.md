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
- **Bounds:** the existing 2,048-slot / 512 KiB glyph cache and raster scratch are reused.
  Mono cells, 9px captions and 17px titles have distinct size/family keys. A new glyph/size is
  rasterized once while cached; cache exhaustion retains its existing bounded reset.
  No font file loading, allocation in painting, added blur or new per-frame raster pass.
- **Evidence:** host font/cache/raster tests (7), Display clock/caption tests (8), civil model and
  Display tests (26) plus three rejected defect controls, and clock pixel-oracle tests
  (18) pass locally. The latter now freeze a source/font-bound grayscale Sans atlas and
  compare all clock pixels, including erased gaps; archived ROM-profile replay remains.
  The 55-case compositor regression and 1,142-test Python suite passed for the initial
  correction; both are being repeated after the proportional-caption refinement.
  Thirteen readability tests now expect compact Geist coverage rather than ROM pixels.
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
