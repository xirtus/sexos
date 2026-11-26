# Frame-time meter, focus priority hint, resize coalescing (Gemini #698, part)

From Gemini's `perf/desktop-smoothness` (#698), rebuilt on current master.

- **Frame-time meter**: `pacing::FrameMeter` keeps the last 64 render times. Display
  logs `[display.frame.meter]` every 64 frames (at most 32 lines), and Super+Shift+F
  toggles a small HUD under the SilkBar with the last frame, P95, P99, slow count and
  a 48-bar sparkline. Shell sends it as `OP_MOTION` arg2 bit 1. The HUD costs nothing
  when off.
- **Focus priority hint**: syscall 104 gains op 2 (`sex_pdx::sched::set_priority_hint`).
  An interactive domain (Display, Shell, Input, USB, SilkBar) names one domain to run in
  the interactive class; 0 clears it. Shell names the focused window's program domain on
  every focus change. `URGENT_STREAK` still bounds it, so normal tasks are not starved.
  **ABI change: syscall 104 op 2.**
- **Resize coalescing**: Shell replaces a resize still queued last for the same window,
  so a fast corner drag delivers the latest size instead of every intermediate one.

Left out of #698 on purpose:
- The zero-copy pixel_region flip with offsets. Master's front/back pixel pages (#499)
  already do this.
- Wheel scroll coalescing. It broke the exact one-queued-per-wheel contract in
  `tests/browser_wheel_route.rs`.
- Non-volatile reads of program pixel regions (another domain writes them concurrently).
