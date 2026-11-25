# Spreadeagle: choice-list validation

- Ctrl+Alt+L gives the selected cells a literal list of 2–30 comma-separated choices through GridCraft's native data-validation model. A blank prompt clears the rule for the selection; both changes are undoable workbook commands.
- A stop-style list rejects an unlisted typed value before it replaces the cell. The focused host test covers invalid and accepted entries, undo/redo, a partial clear, and XLSX reopen. The Programs QEMU gate captures `spreadeagle-validation-prompt.ppm` and `spreadeagle-validation-list.ppm`, checks a rejected entry, and verifies the rule after restart.
- A native dropdown, range-backed lists, and paste/fill validation remain open gaps.
