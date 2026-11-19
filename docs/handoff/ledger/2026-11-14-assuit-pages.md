# Assuit: every page in one scrolling view, and the selection drawn

**`apps/assuit`.** The view was the caret's page only; now it is the whole document.

- **Pages:** every page stacks on the grey desk, one under another, and the view scrolls through them (wheel, caret moves, Page Up/Down). Only pages on screen are drawn, and they are drawn again only after an edit or a zoom: caret moves and selection changes repaint over the cached pictures.
- **Selection:** WordCraft's `selection_rects` gives the highlight rectangles of the selection on each page; the view tints them (the caret hides while text is selected). Shift with the arrows, Home/End and Ctrl+arrows select, as before; dragging with the left button now selects too.
- **Status:** "Page N of M" follows the caret.
- **Tests:** two new host tests (`selection_has_a_highlight`, `long_documents_flow_onto_more_pages`); `scripts/program_assuit_gate.py` now waits for the title line's highlight after Shift+Home, and after fifty Enters for page 2 to be drawn below page 1.
