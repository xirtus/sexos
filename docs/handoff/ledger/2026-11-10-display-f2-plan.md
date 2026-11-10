# Display F2 plan for the next owner (2026-11-10)

F1 (PR #77) is merged: native EDID mode, padded pitch, buffer-age damage-only
scanout, and a whole-number screen scale (1-3) for text and the pointer, chosen
from the EDID size and changed with Ctrl+Alt+= / - / 0. The x17 R1's 4K panel
should boot at scale 2. QEMU always picks scale 1 (its EDID says about 100 dpi).

## F2: window chrome follows the scale (do first)
On a 4K laptop panel, 28 px title bars are about 3 mm tall.
- Add pure helpers in `sex_pdx::desktop_style`: `step_scale(before, boot, step)`
  (the same clamp Display uses for OP_DISPLAY_SCALE) and `chrome_px(base)`.
- Display (`servers/sexdisplay/src/main.rs`): `FRAME_TOP_BAR_HEIGHT_PX` (28),
  `FRAME_RIM_PX`, the `+8` text inset, title glyph scale, and the bar height
  (`BAR_BG_H`, 50/51) all go through `chrome_px`. `panel_title_fg_at` and
  `surface_title` (the desktop thread's panel title) must follow the same scale.
- Shell (`servers/silk-shell`): read the boot scale with
  `display_boot::physical()` (Shell may query kind 3) and mirror every
  Ctrl+Alt step with `step_scale`. Change the `FRAME_*` consts (around
  `SILK_CHROME_TEMPLATE_DEFAULT`) and `P.bar_height` into functions;
  `wm_work_area()` then follows. Scale the tiling gaps (6/8 px) in `wm.rs`.
  The desktop thread agreed that F2 does the Shell half.
- Spindle: learn the screen scale. Its exact panes are deliberately left out of
  `surface_text_scale` today.
- Gate: extend `scripts/display_native_gate.py`. After Ctrl+Alt+=, check that
  the title bar band doubles in height and that a click on the scaled close
  button closes the window. Source-text host gates pin Display code, so grep
  `scripts/` and `tests/` for any literal you change.

## F3 and later
- Chrome and text still go through per-pixel `composite_pixel`. Turn them into
  span copies too (a full 2560x1600 frame takes ~0.74 s under TCG).
- Multi-monitor groundwork: log every Limine framebuffer, and give Display an
  outputs list (one Store per output) with a per-output scale.
- Intel Tiger Lake modesetting (i915 display is MIT, so it can be ported):
  panel brightness first (CNP+ PCH backlight registers, or eDP AUX/DPCD), then
  external ports. This can only be checked on the x17.

Build notes: cloud containers boot UEFI only
(`SEXOS_GATE_FIRMWARE=/usr/share/OVMF/OVMF_CODE_4M.fd`). QEMU at 2560x1600
needs `vgamem_mb=64`. ABI pins are per file (`scripts/update_abi_hashes.sh`).
