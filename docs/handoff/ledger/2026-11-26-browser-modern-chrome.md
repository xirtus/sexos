# 2026-11-26 — Kaleidoscope Helium-Style Single-Row Candy Lavender Chrome

## Summary

Upgraded the native SexOS browser (**Kaleidoscope**, `apps/web` + `crates/sex-web`) with a Helium/Safari-compact single-row top bar harmonized with SexOS's Candy Lavender aesthetic, unified URL + search omnibox, inline rounded tabs, toggleable clickable bookmarks bar, and frosted lavender right-click context menu.

## Changes

- **`crates/sex-web/src/chrome.rs`** (new) & **`crates/sex-web/src/lib.rs`**:
  - Implemented the 38px single-row Helium-style top bar (`TOP_BAR_H = 38`, `BAR = 38`):
    - Left: compact rounded `←` Back, `→` Forward, `↻`/`×` Reload/Stop buttons.
    - Inline rounded pill **Omnibox** (`~260–340px` wide) with site-info/lock indicator, URL/search input, Candy Lavender focus ring, and inline **Bookmark Star (`★`)**.
    - Inline rounded **Tabs** right next to the omnibox on the same row: colored favicon dot, truncated title, close `×` button, lifted Candy Lavender active pill, subtle flat inactive tabs, tab-group labels, and `+` New Tab button.
    - Far right utility cluster: **Shields** pill (active/off + blocked count), **Bookmarks Bar toggle** button, and **`⋮` Menu** button.
  - Added **Unified Omnibox** resolution (`resolve_omnibox_input`): preserves explicit schemes, adds `http://` for `localhost`/IPv4, `https://` for bare domains, and routes natural-language queries to DuckDuckGo HTML search.
  - Added toggleable 26px **Clickable Bookmarks Bar** (`BOOKMARKS_H = 26`, `chrome_height(show_bookmarks_bar)`) with default quick-access pills (`Home`, `Wikipedia`, `DuckDuckGo`, `Hacker News`, `YouTube`) merged with user-starred pages.
  - Added floating frosted lavender **Context Menu** (`build_context_menu`, `context_menu_hit_at`, `context_menu_contains`) with window-bounds clamping.
  - Added `column_below` in `crates/sex-web/src/lib.rs` for dynamic chrome height.
- **`apps/web/src/main.rs`** & **`apps/web/src/tabs.rs`**:
  - Wired single-row Helium top bar actions, `close_tab_at(index)` for closing any tab from its `×` button, `show_bookmarks_bar` toggle (default `false` for compact single-row view, 1-click toggleable from toolbar or context menu), unified omnibox submission, clipboard URL copy, and right-click / `⋮` floating context menu (with Escape / outside-click dismissal).
  - Preserved all existing shortcuts (including `Ctrl+Shift+B` side panel, vertical tabs, split view, tab groups, and `--selftest` / `--selftest-net`).
- **`crates/sex-web/tests/chrome.rs`** (new) & **`crates/sex-web/tests/transform.rs`**:
  - Added 7 unit/integration tests covering omnibox URL vs search resolution, tab open/switch/close & inline hit-testing, bookmark store & bookmarks bar hit-testing, single-row toolbar hit-testing, context menu clamping & hit-testing, and single-row + bookmarks-bar rendering bounds safety.
  - Updated `transform.rs` ink scan loop to use `300 - BAR` instead of hardcoded `270`.

## Verification

- `cargo test --locked -p sex-web`: **45 passed, 0 failed** (20 unit tests + 25 integration tests across `avif`, `canvas`, `chrome`, `reader`, `svg`, `transform`, `viewport`).
- `cargo check --locked -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem --manifest-path apps/web/Cargo.toml --bin web --target x86_64-sex-app.json`: **passed with 0 warnings in `apps/web` and `sex-web`**.
- Zero kernel changes; zero new external crates; `target/` removed cleanly after verification.
