# Vellum form text caret

- Branch: `codex/vellum-form-pointer-navigation`, stacked on `codex/vellum-widget-focus` (#712).
- Gap: a focused text field or editable combo could only append and erase from the end.
- Change: keep a character-indexed caret in the form bar. Left/Right, Home/End, Backspace, Delete, and typing edit there; focus and choice selection place the caret at the end.
- Host proof: `form_caret_edits_unicode_at_character_boundaries` covers insertion, both deletion directions, display position, and the 200-character limit.
- Guest proof: the Documents QEMU gate edits the real form name in the middle, captures `pdf-form-caret.ppm`, then saves and verifies the unchanged expected PDF field value.
- Limits: text selection and pointer placement within text remain open. Full image and QEMU evidence are pending GitHub CI.
