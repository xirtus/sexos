# 2026-11-21 Kaleidoscope vertical tabs (workstream N13b)

- Ctrl+Shift+, (as in Edge) moves the tabs from the bar into a 200 px strip down the
  left side, as Edge and Brave offer; again puts them back. The choice is kept in
  Files `Kaleidoscope/layout.tsv`. The strip hides itself in windows under 600 px.
- The page is laid out and painted beside the strip: `sex_web::Target` gained a
  `stride`, so the page draws into a column of the window buffer with no copy.
  Pages hear `resize` with the narrower innerWidth; pointer coordinates, frames
  (`viewport::Frame::visible` takes `side`) and picture repaints follow.
- Click a tab in the strip to show it, middle-click to close it.
- Gate: `scripts/program_web_vertical_tabs_gate.py` (CI).
