# 2026-11-17 Rounded window corners and soft shadows (WS6, first slice)

Windows on the SexOS desktop now have rounded corners (10 px times the UI
scale, anti-aliased) and soft drop shadows. The focused window floats with a
deep shadow (28 px, pushed 8 px down) that falls on the wallpaper and on the
windows under it; other windows rest with a short ambient shadow on the
wallpaper, so the gaps between tiles read as depth instead of flat lines.

- `crates/sex-graphics/src/window_shape.rs`: the shape math (coverage, row
  inset, hit mask, rounded-rect shadow distance), integer only, shared with
  the Shell for hit masks.
- `servers/sexdisplay/src/decor.rs`: the pass `raster_row` runs after a row is
  composited. It touches only pixels in shadow bands and corner arcs; a corner
  pixel outside the arc shows the next window down or the wallpaper. Partial
  row redraws give the same pixels as full frames (tested). Damage rectangles
  grow by the shadow reach so a moving window leaves no shadow behind.
- Plain look: `OP_MOTION` arg2 bit 0 = square corners and no shadows (redraws
  the whole screen). The Shell does not send it yet; the animations slice adds
  the switch.
- Not decorated: the cursor, the Shell's modal launcher, and a window that
  fills the screen.
- Gate: `scripts/display_decor_host_gate.sh` (CI host job). Older display
  harnesses stub `decor` like the other overlay passes.

Next in WS6: Shell hit masks for the see-through corners, fade/scale open and
close, vsync pacing and a frame-time gate, launcher/overlay blur.

## Follow-up (same day): look switch and hit masks

- Super+Shift+A now steps through three looks: full (animations, rounded
  corners, shadows) -> still (no animations) -> plain (square corners, no
  shadows). `servers/silk-shell/src/animations.rs`; Display gets plain as
  `OP_MOTION` arg2 bit 0.
- Shell hit masks: a click in a see-through rounded corner falls through to
  what is drawn there (`hit_test_at` checks `animations::in_shape` with the
  same `window_shape` math Display draws with).

## Fade-in, 60 Hz pacing and the frame-time gate

- Opening windows fade in (with their shadow) over the motion tween's 180 ms
  while they rise; `decor::opened`/`fade`, cleared when animations go off.
  Closing windows still vanish at once (Display drops a destroyed surface's
  pixels; a close fade needs a retained snapshot, next).
- `servers/sexdisplay/src/pacing.rs`: animation frames on a 60 Hz grid
  (16/17/17 ms) from the timer (virtio-gpu has no vblank); between frames
  Display sleeps until the next one is due instead of spinning. Each paced
  frame logs `[display.frame.time] ms=N p99=M`.
- `scripts/frame_time_gate.py` (Desktop CI job): 4 cores, workspace slides,
  p99 < 20 ms enforced on KVM (falls back to TCG without the threshold when
  the runner has no KVM or cannot boot SexOS on it). Saves desktop.png and
  prints it base64 in the job log.
