# Valentine draft undo and redo

- Added bounded character edit history for the To, Subject and Body draft fields. Ctrl+Z reverses an insertion or deletion; Ctrl+Y restores it. A new edit clears redo.
- The history tracks the last saved draft, so undoing a saved edit marks the draft changed and redoing it clears that state. Only one Unicode character is stored per step, avoiding a full body copy for each key.
- A `sex-mail` host test covers Unicode, multiple fields, saved-state transitions and redo invalidation. The Programs QEMU gate saves a draft, undoes and redoes its last edit, checks both receipts, and captures `valentine-draft-undo.ppm`.
- Validation: the focused `sex-mail` host history test passed in an isolated temporary Cargo target, which was removed. Python syntax and `git diff --check` passed. GitHub CI will run the full image build and QEMU gate; neither ran locally.
