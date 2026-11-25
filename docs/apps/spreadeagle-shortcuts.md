# Spreadeagle keyboard shortcuts

Press **Ctrl+Shift+/** in Spreadeagle for the same reference in the workbook
window. Press it again, or Esc, to return to the grid.

| Shortcut | Action |
| --- | --- |
| Ctrl+S | Save the workbook |
| Ctrl+Shift+S | Save As; the extension selects XLSX, ODS, CSV, TSV, HTML or PDF |
| Ctrl+P | Export PDF beside the workbook |
| Ctrl+G | Go to a cell, range, or sheet address |
| F2 | Edit the selected cell |
| Ctrl+Z, Ctrl+Y | Undo, redo |
| Ctrl+C, Ctrl+X, Ctrl+V | Copy, cut, paste |
| Ctrl+F, Ctrl+H | Find, find and replace |
| Ctrl+Shift+=, Ctrl+- | Insert or delete rows or columns |
| Ctrl+Shift+A, Ctrl+Shift+D | Sort ascending or descending |
| Ctrl+Shift+J | Sort a selected table by two to eight columns in priority order, such as `A asc, C desc` |
| Ctrl+Shift+L, Ctrl+Shift+E, Ctrl+Shift+O | Toggle filter; show rows matching the selected cell; choose column values or All rows |
| Ctrl+Shift+K | Filter the current column by `above average`, `below average`, `top 2`, `bottom 2`, `top 50%`, `= Food`, `<> Food` (`!=` also works), text commands `contains`, `starts`, `ends`, `notcontains`, or a numeric rule such as `> 200`; average and ranked filters use numeric cells (including formula results), ranked filters include ties, `*` and `?` are wildcards with `=` and `<>`; two text/numeric comparisons can be joined with `and` or `or` |
| Ctrl+Shift+F, Ctrl+Shift+C | Freeze panes; insert a column chart |
| Ctrl+Shift+M | Choose a column, line, bar, pie or doughnut chart for the selection |
| Ctrl+Shift+Q | Edit a selected chart (or chart at its anchor): `remove 2` to drop the second series while keeping at least one, `data A1:B4` to change its source range, `switch` to exchange rows and columns as series, `type doughnut`, `title text`, `clear title`, `hole 10`–`hole 90` on doughnuts, `angle 0`–`angle 359` on pie or doughnut, `legend right`, `table on/off`, `x title text`, `y title text`, `clear x title`, `clear y title`, `labels on/off`, or `gridlines on/off` |
| Ctrl+Shift+I | Search the function catalog by name, category or description; Up/Down chooses, Enter inserts `=NAME()` with the caret between the parentheses, Esc closes |
| Ctrl+Shift+U | Toggle wrapped text in the selected cells; automatic row heights grow to show the lines |
| Ctrl+Shift+W | Review warnings from importing the current workbook |
| Ctrl+PageUp, Ctrl+PageDown | Previous or next sheet |

The status line also shows common operations. While entering a formula, Enter
commits it and F2 edits the current cell without replacing its contents.
Drag the right edge of a column header to change its width, or the bottom edge
of a row header to change its height. Ctrl+Z undoes either size change.

With AutoFilter on, click a column header's filter button or press Ctrl+Shift+O
on that column. Use Up and Down to highlight a value, Space to mark several,
Enter to apply, and Esc to close the menu. Existing selected values show checked;
`(blank)` selects empty cells and formulas that display empty text, alone or
alongside other checked values;
press / or Ctrl+F in the menu to search its values (case-insensitive). Search
narrows the list without losing checked choices; Enter applies the highlighted
match, and Backspace edits the query. Esc clears a query before closing the menu.
Space can be typed in a search. Ctrl+Space toggles a highlighted search result.
When the preview has no match, an exact value from the full filter range appears
if it exists; Enter applies it. Enter on the untouched
**All rows** row leaves an active filter in place. Move to a value and press
Enter to replace it, or press Space on **All rows**, then
Enter, to clear that column's filter.
Press C in the value menu to enter a condition for that column. A condition can
return no rows; All rows restores them.
When import reports lost or unsupported features, a warning panel opens over the
workbook. Up and Down review the warnings; Enter or Esc closes the panel. The
shortcut reopens it during the current session.
