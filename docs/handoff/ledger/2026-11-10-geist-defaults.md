## Geist remains the desktop default (2026-11-10)

- **Problem:** Display already drew window/panel titles in Geist Sans and larger native
  text in Geist Mono, but compact scale-1 surfaces reverted to the legacy bitmap font.
  Civil/demo clock labels and Bell counts also always used the bitmap ROM. Andreas
  explicitly reaffirmed Geist as the desktop's default; historical pre-Geist screenshots
  are excluded from the UX plan and all visual comparisons.
- **Change:** both scalar and span text paths now try cached Geist Mono at every native
  text scale. Clock and Bell labels try cached Geist Sans fitted into their existing
  cells, with coverage blended over the composed panel pixel. The ROM is only a fallback
  when the font cannot supply a glyph or a cell cannot be cached. Existing title metrics,
  text cells, clipping, selection, pointer targets, window geometry and palette remain.
- **Role limit:** native Shell text delivery is still a fixed-cell pipeline, so Files,
  launcher/panel bodies and the old Browser reader use Geist Mono there. Existing chrome
  titles use proportional Geist Sans; the new status cells use Sans. A proportional
  label/body delivery pipeline is a follow-up, not claimed by this fix.
- **Bounds:** the existing 2,048-slot / 512 KiB glyph cache and raster scratch are reused.
  Mono and Sans cell keys are distinct from proportional title keys. A new glyph/size is
  rasterized once while cached; cache exhaustion retains its existing bounded reset.
  No font file loading, allocation in painting, added blur or new per-frame raster pass.
- **Evidence:** host font/cache/raster tests (7), Display clock tests (5), civil model and
  Display tests (26) plus three rejected defect controls, and clock pixel-oracle tests
  (18) pass locally. The latter now freeze a source/font-bound grayscale Sans atlas and
  compare all clock pixels, including erased gaps; archived ROM-profile replay remains.
  Actual compositor regression and full Python suite are running at this entry's draft.
  Native screenshots and image-bound workflow gates are required before visual acceptance.
- **Test maintenance:** clock layout fixture used a stale six-slot boundary despite nine
  workspaces; it now derives the status start from `WORKSPACE_COUNT`. No geometry changed.
- **ABI / kernel:** none. No kernel growth. Chrome polish is a separate topic.
