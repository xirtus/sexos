# Frosted glass behind menus and panels (S6b)

- `servers/sexdisplay/src/glass.rs` (new, pure, host-tested): two box-blur passes
  each way (radius 6 px times the screen scale, clamped edges, rounded means), a
  blend toward the theme's popup colour at 200/255, then each pixel's WCAG
  luminance is kept in the band where every ink on the popup (body, headings,
  secondary text, Close) reads at 4.5:1 or more: it is moved further toward the
  tint, by the least of 32 steps, when the desktop under it is too bright (dark
  theme) or too dark (light theme). Pixels come through a small `Pixels` trait;
  the caller lends the scratch (the rows above and below the panel between the
  passes, and two lines), so nothing allocates.
- `servers/sexdisplay/src/frost.rs` (new): the frame side. The window menu, the
  quick settings and calendar, the Bluetooth panel, the program tray's menu,
  tooltip and notification card, and the Alt+Tab switcher (rounded corners kept)
  draw glass inside their opaque rims; rows, hover and text are drawn on it as
  before. Tray menu text now blends its edges with what is under it.
- Feedback: on a partial frame (damage, strip) `raster_batch` paints the body
  again under every shown popup, grown by the blur margin (12 px times scale),
  before any overlay is drawn, so the blur never reads last frame's glass.
- Off (opaque fill as before, receipt `[display.glass] panel=.. glass=0
  reason=..`): the high-contrast theme, Reduce transparency, and a popup whose
  blur does not fit the scratch or that reaches over the bar (`reason=oversize`;
  interiors up to 2048 px wide at scale 3). On: `[display.glass] panel=window_menu glass=1 radius=6 alpha=200
  rect=WxH+X+Y tint=.. floor=.. ceil=..`, once per show.
- New setting `accessibility.reduce_transparency` (id 47, on the Accessibility
  page). The Shell sends it in bit 32 of the `OP_DESKTOP_LOOK` theme word
  (`sex_pdx::desktop_look::REDUCE_TRANSPARENCY`); bits above 32 are still refused.
  `[shell.theme]` gains `reduce_transparency=0|1`.
- Not glass: the Super+W overview (it dims the whole desktop), the screenshot
  hint, the lock screen, and the legacy launcher panel (surface 0x92: a
  compositor surface whose own fill is the body under it).
- Tests: `tests/glass_panels.rs` (flat colour stays, step edge smooth and
  monotone, margin and bounds, tint endpoints, luminance table, ink contrast over
  216 backgrounds in Light and Dark for every accent, high contrast and Reduce
  off, oversize falls back untouched, the wire bit), `tests/desktop_look.rs`.
  Gate: `scripts/glass_panels_gate.py` re-computes the window menu's glass from
  the desktop under it and checks the screen follows the blur, not a flat tint,
  with readable labels. `bt_panel_gate.py` takes the panel's middle luminance as
  its background instead of its commonest colour. No ABI change.
