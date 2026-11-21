# 2026-11-21 Kaleidoscope split view (workstream N13c)

- Ctrl+Shift+\ shows the current tab and the next one side by side, as Edge's split
  screen and Vivaldi's tab tiling do; pressing it again goes back to one page. Works
  with or without the vertical tab strip; each half needs 320 px.
- Each half lays out to its own width (pages hear `resize`). The shown half has the
  keyboard and a blue line under the bar; a press on the other half hands it over.
  Pages get pointer coordinates in their own half; frames are clipped to it.
- Showing a tab outside the pair, opening or closing a tab ends the split.
- Limit: the waiting half is drawn as it last was (its timers, media and frames
  pause until it has the keyboard), like any background tab.
- Gate: `scripts/program_web_vertical_tabs_gate.py` (CI) covers both layouts.
