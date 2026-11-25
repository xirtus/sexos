# Vellum form text pointer caret

- Branch: `codex/vellum-form-pointer-caret`, stacked on `codex/vellum-form-selection` (#727).
- Gap: text fields had keyboard caret and selection controls, but clicking their value in Vellum's form bar did not place the caret.
- Change: map a click in the focused text field's bar value to the nearest character boundary, clamp to the text's ends, and clear any selection. Choice menus retain their own click path.
- Host proof: `form_bar_click_places_caret_at_nearest_character_boundary` covers before, within, and beyond the value, including overflow-safe clamping.
- Guest proof: the Documents gate clicks character boundary five in the actual PDF name field, captures `pdf-form-pointer-caret.ppm` after inserting a character there, restores the expected value, saves, and verifies the PDF.
- Limits: drag selection and editable-combo text pointer placement remain open. Full image and QEMU evidence are pending GitHub CI.
