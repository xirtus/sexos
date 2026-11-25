# Spreadeagle function picker

- Ctrl+Shift+I opens GridCraft's live worksheet function catalog in the native grid. Typing searches names, categories and descriptions; Up/Down picks a function; the panel shows its signature and description. Enter starts `=NAME()` with the caret between parentheses so arguments can be entered before commit.
- A host test requires the picker catalog to expose at least 200 functions and metadata, and checks case-insensitive search. The Programs QEMU gate opens the picker, searches SUM, captures `spreadeagle-function-picker.ppm`, inserts the formula, and verifies the edited cell.
- Full image build and QEMU run on GitHub CI; focused host and source checks run locally.
