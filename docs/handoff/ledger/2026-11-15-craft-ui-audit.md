# Craft UI audit (V16), and the editing keys it found missing in Spreadeagle and Assuit

**`docs/handoff/CRAFT_UI_AUDIT_2026_11_15.md`, `apps/spreadeagle`, `apps/assuit`.**

- **Audit:** each creative and office app beside its storytold Craft base and the leaders:
  - Spreadeagle and GridCraft
  - Assuit and WordCraft
  - Batik and PhotoCraft
  - Gestalt and VectorCraft
  - Splicy and FilmCraft
  - Moan and SoundCraft

  Each gap is fixed here or handed to its owner. It also lists six findings that cut across every
  app: no system clipboard for std programs, keys without Alt and pointers without modifiers, one
  type face in `sex-font`, no shared command palette, the command-id and record convention, and
  performance budgets.
- **Spreadeagle**, on GridCraft's own key map:
  - Ctrl+C, Ctrl+X and Ctrl+V on ranges, with the copied cells outlined.
  - Delete and Backspace empty the whole selection.
  - Ctrl+D fills down, Ctrl+R fills right.
  - Ctrl+Shift+T is AutoSum.
  - Ctrl+B, Ctrl+I and Ctrl+U style the selection. Bold and underline are drawn; italic is kept
    in the file.
  - Ctrl+F finds across sheets, and Enter goes to the next match.
  - F2 edits the cell.
- **Assuit:**
  - Ctrl+F opens a find prompt; Enter selects the next match and the status reads "match i of n".
  - The status line counts words.
  - Copy and paste through WordCraft's clipboard is now tested.
- **Tests:**
  - New host tests `copy_paste_fill_sum_style_and_find` (Spreadeagle) and
    `find_selects_matches_and_copy_pastes` (Assuit).
  - The Spreadeagle gate now finds "rent", copies it, pastes it into A8 and undoes the paste.
  - The Assuit gate now finds "assuit" twice, copies the match, and pastes it on page 2.
