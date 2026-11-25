# Vellum form text selection

- Branch: `codex/vellum-form-selection`, stacked on `codex/vellum-form-pointer-navigation` (#723).
- Gap: text form editing gained a caret but could not select and replace a span.
- Change: Shift+Left/Right and Shift+Home/End extend a highlighted character range. Plain arrows collapse it, Ctrl+A selects all in text fields, and typing/Backspace/Delete replace or remove the selection. Editable combo Ctrl+A retains its clear action.
- Host proof: `form_selection_replaces_forward_and_reverse_unicode_spans` covers reversed range endpoints, Unicode boundaries, replacement, and select-all removal.
- Guest proof: the Documents gate selects the last name in a real PDF field, captures `pdf-form-selection.ppm`, types its replacement, then saves and checks the expected field value.
- Limits: pointer placement and drag selection in the text bar remain open. Full image and QEMU proof are pending GitHub CI.
