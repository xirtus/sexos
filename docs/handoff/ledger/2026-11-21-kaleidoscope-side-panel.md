# Kaleidoscope side panel (N13e), 2026-11-21

A side panel as in Chrome, Edge and Brave: a page 360 px wide at the right of the window, beside the shown tab.

**Opening it**
- **Ctrl+Shift+B** opens the bookmarks panel: bookmarks, newest first, then the 20 most recent pages (`about:bookmarks`).
  - A link followed there opens in the page beside it, and that page takes the keyboard back.
  - Ctrl+Shift+B again closes the panel. While the panel shows something else, Ctrl+Shift+B switches it to the bookmarks first.
- **Extensions** whose manifest has `side_panel.default_path` (`Manifest.side_panel` in sex-extensions) get a "Side panel" link on about:extensions.
- **`chrome.sidePanel`** is supported: `open`, `setOptions`, `getOptions`, `setPanelBehavior`, `getPanelBehavior`. `open()` shows the extension's panel page.
- **`about:sidepanel?url=…`** opens the panel on any page.

**How it works**
- The panel's page is an ordinary tab kept in the last tab slot. It is left out of the strip, Ctrl+1..9 and Ctrl+PageUp/PageDown.
- While the panel is open, the shown tab and the panel are the two sides of a split view (`Browser::columns`):
  - The page lays out to the width left over, and hears `resize`.
  - A press on the panel gives it the keyboard.
  - Showing another tab puts that tab beside the panel.
  - New tabs go before the panel's slot.
  - Ctrl+W in the panel closes the panel.
  - The two-tab split view (Ctrl+Shift+\) and the panel replace each other.
- If the window has no room (less than 320 px of page beside it), the panel doesn't open, and `[web.panel] on=0 room=0` is logged.

**Code:** apps/web/src/panel.rs; `strip_len`, `panel_slot` and `repair_panel`.

**Serial records**
- `[web.panel] on=1 w= url=`
- `[web.panel] on=0 shown=`
- `[web.panel.follow] shown= url=`

**Gate:** `scripts/program_web_side_panel_gate.py` (CI). The sample extension now has `panel.html`, and its options page calls `chrome.sidePanel.open()`.

**Fixed along the way:** opening a tab while the panel had the keyboard.

**Not yet**
- While the page beside it has the keyboard, the panel's page is painted as it was last, and its timers wait (the same as the waiting half of a split view).
- No panel per tab, and no way to change the panel's width.
