# 2026-11-26 — Interactive SilkBar Dropdowns, Status Popovers, Right-Click Menus & KDE/Hyprland Hotkeys

## Summary
Implemented macOS/KDE-style interactive SilkBar dropdown menus, anchored frosted Candy Lavender status popovers, desktop/window right-click context menus, right-aligned keyboard shortcut hints, and additional KDE/Hyprland hotkeys (`Alt+F4`, `Alt+F10`, `Super+PgUp`/`PgDn`/`Grave` tab cycling).

## Key Changes
- **Candy Lavender Theme Tokens (`crates/sex-theme/src/lib.rs`)**:
  - Added frosted Candy Lavender menu/popover palette constants (`DARK_MENU_*`, `LIGHT_MENU_*`) with WCAG AA/AAA contrast verification in unit tests.
- **Shared Menu Protocol, Layout & Controller (`crates/sex-pdx/src/silkbar_menus.rs`, `crates/sex-pdx/src/lib.rs`)**:
  - Added `OP_SILKBAR_MENU` (`0x2FC`), `MenuKind`, `TopBarTrigger`, `MenuAction`, `RowSpec` (with right-aligned shortcut hints and section dividers), 2-word wire codec, shared screen-clamped `Layout` + `Hit`, and pure `Controller` state machine (including macOS-style top-bar hover-switching and Escape/outside-click dismissal).
  - Kept `top_bar_menu_rects` (`File`, `Edit`, `View`, `Window`) strictly left of `290 * scale` so pixel `(300, 14)` in `scripts/virtio_gpu_gate.py` remains untouched frosted SilkBar glass.
- **Display Frosted Candy Lavender Rendering (`servers/sexdisplay/src/silkbar_menus.rs`, `frost.rs`, `window_menu.rs`, `main.rs`)**:
  - Added `frost::Panel::SilkBarMenu` and frosted Candy Lavender renderer with specular top highlight, orchid rim, hover pills, right-aligned shortcut hints, and interactive Volume/Wi-Fi/Battery/Clock popovers.
  - Harmonized `window_menu.rs` and `draw_tray_menu` with Candy Lavender tokens while preserving all `tray_gate.py`, `window_decorations_gate.py`, and `virtio_gpu_gate.py` invariants.
- **Shell Input Dispatch & KDE/Hyprland Hotkeys (`servers/silk-shell/src/silkbar_menus_glue.rs`, `desktop_keys.rs`, `wm.rs`, `main.rs`)**:
  - Wired left System pill dropdown (About SexOS, Kaleidoscope, Spindle, Tailor AI, Flash Webcam, PD Graph, Files, Settings, Refresh Desktop, Reboot, Shutdown), contextual `File`/`Edit`/`View`/`Window` dropdowns, right-side Wi-Fi & Battery popovers, top-bar hover-switching, desktop background right-click context menu, and Escape/outside-click dismissal.
  - Added `Alt+F4` (close window) and `Alt+F10` (toggle maximize) to `desktop_keys.rs` and `Super+PgUp`/`Super+PgDn`/`Super+Grave` tab cycling to `wm.rs`.
- **Host Unit Tests (`tests/silkbar_menus.rs`)**:
  - Added 12 host tests covering wire codec, `(300, 14)` gate preservation, menu items & shortcut hints, hover-switching, outside-click & Escape dismissal, volume/battery popover controls, desktop/window context menu clamping & dispatch, and KDE/Hyprland hotkeys.

## Verification
- `cargo check -p sexdisplay -p silk-shell -p silkbar --target x86_64-sex.json -Z build-std=core,alloc -Z build-std-features=compiler-builtins-mem`: PASS
- `cargo test -p sex-theme` & `cargo test -p silkbar-model`: PASS
- `rustc --test tests/silkbar_menus.rs`, `tests/tray.rs`, `tests/wm_layouts.rs`: PASS
- `python3 scripts/tests/test_passive_wait_observer.py` (9 tests) & `python3 scripts/tests/test_self_wait_source.py` (16 tests): PASS
- Zero kernel changes; `target/` deleted before commit.
