# Spreadeagle and Assuit: find and replace, column widths, number formats, paragraph styles

**`apps/spreadeagle`, `apps/assuit`.** The next office slice from the Craft UI audit
(`docs/handoff/CRAFT_UI_AUDIT_2026_11_15.md`).

- **Spreadeagle:**
  - Ctrl+H opens find and replace. Tab or Enter moves to "Replace with", and Enter replaces
    every match in the workbook (GridCraft's `edit.replace`).
  - Dragging a column header's right edge sets the column's width: a blue guide follows the
    pointer, and the width applies on release (`home.columnWidth`).
  - Ctrl+Shift+~ and Ctrl+Shift+1 to 6 give the selection Excel's number formats: General,
    Number, Time, Short Date, Currency, Percentage, Scientific.
- **Assuit:**
  - Ctrl+H opens find and replace (WordCraft's `edit.replaceAll`).
  - Ctrl+Shift+S opens a list of the Styles gallery's paragraph styles on the caret's style.
    Arrows choose, and Enter applies the style (`para.style`).
  - The status line names the caret's paragraph style.
- **Tests:**
  - New host tests `replace_widths_and_number_formats` and `replace_and_paragraph_styles`.
  - The Spreadeagle gate replaces "food" with "meals", shows B2:B4 as currency, and drags
    column A wider.
  - The Assuit gate replaces "assuit" everywhere and makes the title a Heading 1 from the list.
  - Both gates now send Shift chords as separate key events. `send-key` with Shift could leave
    Shift held for the next chord.
