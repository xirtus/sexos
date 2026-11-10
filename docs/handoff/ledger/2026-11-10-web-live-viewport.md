# Web uses its actual program window

The ordinary Web program had a fixed 410 × 330 raster and ignored the existing
Shell-authenticated resize notices. Maximizing the surrounding window did not
increase the document viewport. This is separate from the built-in Browser pane.

This topic uses Window's existing size and pixel grant, validates dimensions and
signed raster products, updates the document viewport before its resize event,
and reflows/repaints on size changes. The physical surface is separate from the
iframe document swap. Child scratch is bounded by the grant capacity and clipped
to the visible document without overwriting the toolbar. Tiny windows also clip
the toolbar target to the actual surface. No kernel changes or new authority.

Window keeps its newest authenticated size even if resource waits fill the held
input queue; Web reconciles that size and does not replay stale held dimensions.
The browser fixtures now render references at the actual complete paint size and
use source-selected program insets. They retain their pixel thresholds and verify
the whole visible canvas, with the real cursor moved out of it where appropriate.
The animation/media initial timing phases do not maximize the window.

Validation at the initial source checkpoint:

- All 1,166 Python host regressions passed (67.134 seconds).
- Three direct tests of the production surface/iframe helper passed before this
  checkpoint: grant bounds, overflow/empty/tiny, exact clipping and buffer refusal.
- Rust DOM resize and tiny-toolbar renderer tests, optimized target build, and
  native maximize/restore plus existing JS/frame/pointer gates are pending. The
  new native gate requires actual Shell input, increasing JS resize counts and an
  independent RGB oracle throughout the document, including right/bottom edges.

Build inputs: base master 3e075b59, isolated codex/web-live-viewport worktree.
Only the verified decoder-only FFmpeg lib/include/include-c were copied (359
files); no Rust target artifacts were copied. The input hash manifest and host log
are outside the repository under sexos-evidence/web-live-viewport. This checkpoint
is not evidence that the native resize gate has passed or that YouTube plays.
