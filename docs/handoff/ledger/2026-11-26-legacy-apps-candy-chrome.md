# 2026-11-26 — Legacy GUI Apps Candy Lavender Chrome, HyprMod Settings & Dolphin/Finder Files

## Summary
Upgraded SexOS's legacy pixel-buffer GUI apps from raw square grey `fill_rect` controls to a unified, anti-aliased **Candy Lavender** widget kit (`sex-theme::candy_ui`), upgraded **Settings (`apps/settings`)** with a HyprMod-inspired Candy Lavender Appearance & Preferences GUI, and upgraded **Files (`apps/files`)** with a Dolphin/Finder Candy Lavender layout.

## Shared Candy UI Kit (`crates/sex-theme`)
- Added [`crates/sex-theme/src/candy_ui.rs`](../../../crates/sex-theme/src/candy_ui.rs) (`#![no_std]`, pure integer half-pixel SDF anti-aliasing, zero heap, zero new external crates):
  - `CandyPalette` (`dark()`, `light()`, `high_contrast()`, `for_mode(Mode)`) derived from `sex_theme::candy_lavender`, WCAG AAA/AA verified.
  - `fill_rounded_rect`, `draw_candy_toolbar`, `draw_candy_panel`, `draw_candy_button` (`Normal`, `Primary`, `Active`, `Danger`, `Subtle`, `Disabled` × `Rest`, `Hover`, `Pressed`), `draw_candy_tab`, `draw_candy_selection_row`, `draw_candy_badge`, `draw_candy_toggle`, `draw_candy_swatch`.

## Upgraded Apps
1. **Mesh — System Monitor & PD Capability Graph (`apps/monitor` + `crates/sex-mesh`)**:
   - Note: in this codebase, the PD capability graph app is `apps/monitor` ("Mesh", Grants tab; there is no separate `apps/pd-graph` directory).
   - Replaced flat dark-grey chrome with Candy Lavender frosted toolbars, anti-aliased rounded tab pills, rounded selection pills, rounded `End program` danger pill button, live hover/pressed pointer tracking (`layout::HoverTarget`), and rounded Candy chart cards.
2. **Settings / Tatami (`apps/settings`)**:
   - Added [`apps/settings/src/candy_gui.rs`](../../../apps/settings/src/candy_gui.rs): full pixel-buffer Candy Lavender GUI inspired by HyprMod.
   - Categorized sidebar (`Appearance`, `Windows & Tabs`, `Keybinds`, `Profiles`, plus all existing `sex-config` System pages).
   - **Live Preview Card** + **Instant Apply** for Theme mode pills, 6 Candy Accent swatches (`Candy Lavender` default `#A78BFA`, `Orchid`, `Mint`, `Sky`, `Peach`, `Slate`), Frosted Lavender Glass (`accessibility.reduce_transparency`) & Animations toggles, Window Corner Radius / Tab Bar / Tiling defaults (Hyprland / KDE), and KDE + Hyprland Keybinds reference/profile switcher.
   - One-click **Named Profiles** (`Candy Lavender`, `Midnight Orchid`, `Hyprland Compact`, `KDE Classic`) plus header **Undo** and **Reset** Candy pill buttons.
3. **Files (`apps/files/src/thumbs.rs`)**:
   - Upgraded grid view to a Dolphin + Finder Candy Lavender layout: frosted **Places / Favorites sidebar** (`Home`, `Documents`, `Downloads`, `Pictures`, `Devices / Media`), top Candy toolbar (`<` `>` `^`, breadcrumb path pill, `Icons`/`List` tabs, `+` new tab), **Folder Tab Bar**, rounded Candy file cards, and frosted `N Folders, M Files` status bar.
4. **Flash (`apps/flash`)**:
   - Upgraded top toolbar, filter capsule pill, bottom control bar, all 7 square grey control buttons (`Effects`, `Filter`, `Shutter`, `Timer`, `Burst`, `Video`, `Open in Gaze`), and filmstrip cards to rounded Candy Lavender chrome while preserving all `[flash.control]` coordinates.
5. **SexShop / Store (`apps/store`)**:
   - Upgraded header/footer toolbars, update badge pill, category tabs, featured package cards, selection rows, and `< Back` / `Install` / `Upgrade` / `Remove` buttons to rounded Candy Lavender chrome.
6. **Calendar (`apps/calendar`)**:
   - Upgraded header toolbar, view tabs (`Month`, `Week`, `Day`, `Clock`, `Calendars`, `Sync`), alert banner pill, Clock/Calendars cards, rounded calendar color swatches, and sidebar agenda selection rows to Candy Lavender.
7. **Gossip (`apps/gossip`)**:
   - Upgraded header toolbar and feed/item selection rows to Candy Lavender.

## Verification
- `cargo test -p sex-theme -p sex-mesh`: **21/21 passed** (7 `sex-theme` WCAG & pixel AA/state tests + 14 `sex-mesh` layout/hover/chart/canvas tests).
- `bash scripts/settings_host_gate.sh`: **PASS** (all existing Settings, config, shortcuts, and shell host tests).
- Host unit tests for `apps/settings/src/candy_gui.rs` + `model.rs`: **11/11 passed** (named profiles, undo/reset, live preview & swatch/toggle hitbox registration).
- `cargo check -p monitor -p settings -p flash -p store -p calendar -p gossip -p files --target x86_64-sex-app.json -Z build-std=core,alloc`: **PASS (0 errors)**.
- Zero kernel changes; untouched `apps/web/`, `crates/sex-web/`, `servers/silkbar/`, `servers/silk-shell/`, `servers/sexdisplay/`.
- `target/` removed from worktree before commit.
