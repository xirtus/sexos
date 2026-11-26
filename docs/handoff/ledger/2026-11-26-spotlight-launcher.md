# 2026-11-26 — OLauncher / macOS Spotlight Frosted Candy Lavender App Launcher & Search

## Summary
Implemented a centered, frosted Candy Lavender **OLauncher / macOS Spotlight-style App Launcher & Search overlay** (`640x440` logical px, screen-clamped) across `sex-theme`, `sex-pdx`, `sexdisplay`, and `silk-shell`, triggered by `Super+Space`, `Super+D`, `Alt+Space`, or the SilkBar System dropdown (`Spotlight Search`).

## Key Changes
- **Candy Lavender Spotlight Theme Tokens (`crates/sex-theme/src/lib.rs`)**:
  - Added dark & light frosted Spotlight input well, tile, selected highlight, category pill, and active orchid pill tokens (`DARK_SPOTLIGHT_*`, `LIGHT_SPOTLIGHT_*`, `ORCHID_PILL_ACTIVE`, `ON_ORCHID_ACTIVE`), all verified against WCAG AA/AAA contrast ratios.
- **Shared Spotlight Protocol, Catalog, Search, Calculator, Layout & Controller (`crates/sex-pdx/src/spotlight.rs`, `lib.rs`, `silkbar_menus.rs`)**:
  - Added Shell -> Display PDX opcodes `OP_SPOTLIGHT` (`0x2FD`) and `OP_SPOTLIGHT_QUERY` (`0x2FE`, packing up to 24 ASCII query bytes across 3 `u64` words).
  - Curated 10-app SexOS catalog in a 5x2 grid: **Tailor AI**, **Kaleidoscope**, **Flash**, **Spindle**, **Files**, **Quil**, **Settings**, **PD Graph**, **Audio**, and **Store** — each mapped to a real catalog/program launch target with distinct pastel/vibrant icon badge colors and monograms.
  - Added live case-insensitive substring & fuzzy subsequence filtering (`filter_catalog`) across app names, IDs, and keywords.
  - Added fixed-point `#![no_std]` inline arithmetic evaluator (`eval_calc`: `+`, `-`, `*`, `/`, `%`, parentheses, unary `-`, decimal output like `7/2 -> = 3.5`, division-by-zero safe).
  - Added 5 category filter pills (`All | Apps | Media | AI & Dev | System`), screen-clamped `Layout`, `Hit` testing, and pure `Controller` state machine (2D arrow-key grid navigation, `Tab`/`Shift+Tab`/`Ctrl+Left/Right` category switching, `Enter`/click launch, `Esc`/outside-click dismissal).
  - Added `"Spotlight Search"` (`Super+Spc`) to the SilkBar `System` dropdown menu (`MenuAction::OpenSpotlight`), preserving `(300, 14)` and all `virtio_gpu_gate.py` / `tray_gate.py` invariants.
- **Display Frosted Candy Lavender Renderer (`servers/sexdisplay/src/spotlight.rs`, `frost.rs`, `main.rs`)**:
  - Registered `frost::Panel::Spotlight` (`PANELS = 9`) and rendered the frosted Candy Lavender backdrop, top specular sheen, orchid rim, rounded search input with vector magnifying glass icon + live caret + glowing inline calculator result badge, category filter pills, 5-column Candy app icon grid with soft lavender selection highlight, and footer shortcut bar (`Arrows Select   Enter Open   Tab Filter   Esc Close`).
- **Shell Integration & Hotkeys (`servers/silk-shell/src/spotlight_glue.rs`, `wm.rs`, `desktop_keys.rs`, `silkbar_menus_glue.rs`, `main.rs`)**:
  - Wired `Super+Space` and `Super+D` (`WmAction::Spotlight`) in `wm.rs`, `Alt+Space` (`Action::Spotlight`) in `desktop_keys.rs`, and `MenuAction::OpenSpotlight` in `silkbar_menus_glue.rs` while keeping `Super` tap alone on `WmAction::Launcher`.
  - Kept `servers/silk-shell/src/main.rs` changes to 6 minimal hook lines so passive-wait & self-wait CI fingerprints remain intact.
- **Host Unit Tests (`tests/spotlight.rs`, `tests/silkbar_menus.rs`, `tests/wm_layouts.rs`, `tests/window_groups.rs`)**:
  - Added 7 comprehensive Spotlight tests covering wire codec, live search & Backspace, inline calculator, category switching, 2D grid arrow navigation, Enter & tile-click launch dispatch, Esc/outside-click dismissal, screen clamping, and `Super+Space`/`Super+D`/`Alt+Space` hotkeys.

## Verification
- `cargo check -p sexdisplay -p silk-shell -p silkbar --target x86_64-sex.json -Z build-std=core,alloc -Z build-std-features=compiler-builtins-mem`: PASS
- `cargo test -p sex-theme`: PASS
- `rustc --edition 2021 --test tests/spotlight.rs`, `tests/silkbar_menus.rs`, `tests/wm_layouts.rs`, `tests/window_groups.rs`: PASS
- `python3 scripts/tests/test_passive_wait_observer.py` (9 tests) & `python3 scripts/tests/test_self_wait_source.py` (16 tests): PASS
- Zero kernel changes; zero new external crates; `target/` deleted before commit.
