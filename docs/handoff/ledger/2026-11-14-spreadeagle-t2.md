# Spreadeagle T2: sort, filter, freeze panes, charts and ODS formulas

**`apps/spreadeagle`, `crates/sex-office`.** The second Spreadeagle slice from `docs/handoff/APP_PARITY_PLAN_2026_11_14.md`, all on GridCraft's engine commands.

- **Selection:** Shift+arrows stretch a range (tinted, its name in the formula bar); Esc drops it. Sort, filter and charts work on it, or on the region around the cursor.
- **Sort:** Ctrl+Shift+A sorts A to Z and Ctrl+Shift+D Z to A by the cursor's column (`data.sortAscending` / `data.sortDescending`), keeping a header row on top. Undo puts it back.
- **Filter:** Ctrl+Shift+L turns the AutoFilter on or off (`data.filter`); Ctrl+Shift+E shows only the rows whose cursor column matches the cursor's cell (`data.filterBy`). Hidden rows are skipped by the grid and by Up, Down and Page keys; the header row gets filter buttons.
- **Freeze panes:** Ctrl+Shift+F freezes the rows above and the columns left of the cursor, or unfreezes (`view.freezePanes`); frozen rows and columns stay while the rest scrolls, behind a darker line. Kept in the xlsx.
- **Charts:** Ctrl+Shift+C adds a column chart of the selection beside it (`insert.chart`). GridCraft's chart crate lays it out and rasterises the shapes; Spreadeagle draws its text with the grid's own font (sex-font) and floats it over the cells at its anchor. Kept in the xlsx.
- **ODS:** GridCraft imports ODS data only. `apps/spreadeagle/src/ods.rs` reads the formulas from `content.xml` (repeated rows and columns included), turns OpenFormula into Excel spelling (`of:=SUM([.B2:.B4])` to `=SUM(B2:B4)`, `;` to `,`, sheet references) and puts them back as part of the opened file, so an .ods recalculates. Ctrl+S on an .ods, .xls or .xlsb writes an .xlsx beside it.
- **sex-office:** `Canvas::tint` and `Canvas::blit`.
- **Tests:** 14 host tests in `cargo test -p spreadeagle` (sort with undo, filter on and off, freeze saved, chart drawn and saved, ODS open with its formulas and save as xlsx, OpenFormula translation) plus `tint_and_blit_stay_inside` in sex-office; `apps/spreadeagle/make_sample_ods.py` writes the hand-made ODS fixture. `scripts/program_spreadeagle_gate.py` now also selects A1:B4, sorts it, charts it, filters to one row and back, freezes panes, saves, and finds the chart and panes on the next open.
- **Next:** filter by more than one value (a picker), chart types and moving charts, column resize, copy and paste, ODS writing.
