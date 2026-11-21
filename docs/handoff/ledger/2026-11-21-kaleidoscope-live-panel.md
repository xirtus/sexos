# Kaleidoscope live side panel and split half (N13g), 2026-11-21

The side panel's page, and the other half of a split view, now keep running while the page beside them has the keyboard.
- **Timers:** their timers, animations and fetches are served, and what they draw shows straight away.
- **Before:** they were painted as they last were, and their timers waited until they were shown.

How it works:
- `Browser::waiting_half()` names that tab, and its timer counts in `next_due`.
- `tick_all` runs its `tick` through `Browser::visit(i, f)`. That swaps the tab into the browser's fields (`active` and `pane()` follow), runs `f`, and swaps the shown tab back.
- While visiting, `draw()` paints only that tab's column, and `about:sidepanel` addresses are ignored.

What still waits:
- Tabs outside the split or panel still pause in the background, as before.
- The visited tab's new code compiles under the shown tab's code cache partition (switching partitions on every tick would read the cache from Files each time).

Gate: `program_web_side_panel_gate.py`. The sample extension's `panel.js` counts three ticks of a 300 ms interval while the options page beside it has the keyboard. The vertical tabs gate (split view) still passes.
