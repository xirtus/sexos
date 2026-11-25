# Vellum annotation region drag

- Branch: `codex/vellum-drag-region`, stacked on `codex/vellum-form-pointer-caret` (#732).
- Gap: selecting a markup region required repeated arrow presses after Ctrl+R.
- Change: while Vellum's region tool is active, a left-button drag across the rendered page sets a rectangle using the current zoom, scroll, and page origin. Reverse drags normalize; regions are clamped to the page. Enter still creates the highlight through the existing PDF editor.
- Host proof: `dragged_region_normalizes_reverse_drag_and_stays_on_page` covers reverse direction, page edges, and a very small page.
- Guest proof: the Documents QEMU gate drags the real page, checks the region receipt, captures `pdf-region-drag.ppm`, then highlights, saves, and verifies the annotation count.
- Limits: direct drag initiation without Ctrl+R and richer markup appearance controls remain open. Full image and QEMU evidence are pending GitHub CI.
