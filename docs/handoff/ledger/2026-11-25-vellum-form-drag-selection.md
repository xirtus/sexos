# Vellum: drag select form text

- A primary-button drag across a focused PDF text field's form bar selects the bounded character range, including reverse drags. The existing selection highlight and replacement keys use that range; a click still places the caret. A field change or closing form mode ends the drag.
- A focused Unicode test checks geometry-to-character selection and replacement. GitHub CI's real-PDF edit guest gate drags over the name, captures `pdf-form-drag-selection.ppm`, replaces the selected surname, and verifies the committed field value after restoration.
- Editable combo boxes still need pointer text selection distinct from their choice menu.
