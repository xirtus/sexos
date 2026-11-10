# Approved slim geometry: coordinated implementation (2026-11-10)

Andreas approved the visual concept and shared-F2 plan in this chat on 2026-11-10.
This implements its geometry stage after shared metrics #139 and applied-scale
receipts #141. This entry is an implementation checkpoint, **not acceptance**;
first native build, new pixel oracles, complete host/guest gates and latency/memory
qualification are outstanding. Never substitute the generated concept for a guest
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
50->84 rows for 3x: two 2560-wide legacy scratch arrays add 696,320 static bytes; owned
scene rows add 136 bytes, plus small fixed metric/notice fields. BOOT/service slot
headroom and actual rendering/cache work must be measured before acceptance. No new
blur pass, animation timer, per-pixel allocation or kernel code is introduced.

Geometry diagnostics are emitted only after a completed full CPU publication and
once per changed scale/mode. Browser fixture helpers assert independent 28/26/8/34/24
logical goldens, then use that published geometry for real input and pixel origins;
coverage thresholds remain. Older 8/36/44 offsets and 50/51 strip goldens require
source-bound oracle migration, not relaxed assertions.

Checkpoint checks: production shared metric/state cases, 38 Quil view cases, 19
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
13px Geist clock oracles passed; native2560 scale1→2→1 receipts/ink passed, and a
separate2x clock/geometry capture passed13824pixels. ISO `fd75e3415c457806d87e956be04d5e3e4a84b5f3824877ab37ec506ec93f963f`.
Display ELF sections were text639306/data1269220/bss2461619bytes (4370145total);
BOOT reached and rendered the native desktop. This is a source-bound correctness
checkpoint, not final input/browser/CI/performance acceptance. All guests stopped.
The civil-clock source/health/authority suite retains26passing cases with the
single-line target and adds unchanged-sample/new-target draw-stamp coverage.
