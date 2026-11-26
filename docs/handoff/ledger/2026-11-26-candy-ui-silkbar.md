# Candy window frames, four window buttons and SilkBar motion (Gemini UI)

Lands Gemini's `ui/candy-polish-and-silkbar` branch (PR #735) on current master.

- `servers/sexdisplay/src/chrome_candy.rs`: candy-tinted window frames that match
  the SilkBar, four clear window buttons (close, minimise, maximise, float) with
  always-visible glyphs and a hover ring.
- `servers/sexdisplay/src/silkbar_anim.rs`: SilkBar hover capsules, workspace pill
  transitions and the tray menu slide. Zero cost when settled; snaps when motion
  is off.
- The pointer now draws over the SilkBar (white outline, dark interior). The
  recording marker is still painted after the cursor in `raster_batch`, so it stays
  exact on top.
- Shell passes the window's candy tint in bits 16..19 of the surface chrome message
  `arg2` (Display ignores values above `CANDY_COUNT`). This is an internal
  Shell-to-Display field, not a syscall ABI change.

Test updates for the new look:
- `tests/display_bitmap_transaction.rs`: content fill now uses `base_col` (dark-mode
  candy interior).
- `scripts/virtio_gpu_gate.py`: bar expects the candy Light SilkBar (221,220,224);
  the close light is checked for red dominance because it carries its glyph.
- `scripts/tests/test_recording_indicator_pixels.py`: repaints the marker after the
  cursor, as production does.
