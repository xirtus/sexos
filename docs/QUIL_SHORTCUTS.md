# Quil keyboard shortcuts

Press **Ctrl+Shift+/** in Quil to see the active keymap. Use Up/Down or PageUp/PageDown
to scroll, and Esc or Ctrl+Shift+/ to close the reference. The command palette
(Ctrl+Shift+P) lists every command and switches between Default, Vim and Emacs keys.

| Action | Default key | Emacs key |
| --- | --- | --- |
| Save | Ctrl+S | Ctrl+X, Ctrl+S |
| Save As | Ctrl+Shift+S | Ctrl+X, Ctrl+W |
| Open | Ctrl+O | Ctrl+X, Ctrl+F |
| Open folder | Ctrl+Shift+O | Ctrl+X, Ctrl+D |
| New tab | Ctrl+N | Command palette |
| Close tab | Ctrl+W | Ctrl+X, K |
| Undo | Ctrl+Z | Ctrl+/ or Ctrl+X, U |
| Redo | Ctrl+Y or Ctrl+Shift+Z | Command palette |
| Find | Ctrl+F | Ctrl+S |
| Replace | Ctrl+H | Esc, % |
| Go to file | Ctrl+P | Ctrl+X, B |
| Search folder | Ctrl+Shift+F | Ctrl+Shift+F |
| Command palette | Ctrl+Shift+P | Esc, X or Ctrl+Shift+P |
| File tree | Ctrl+B | Command palette |
| Tracked Git changes | Ctrl+Shift+V | Ctrl+Shift+V |
| Add cursor below | Ctrl+Shift+Down | Ctrl+Shift+Down |
| Next occurrence | Ctrl+D | Command palette |
| All occurrences | Ctrl+Shift+L | Ctrl+Shift+L |
| Toggle line comment | Ctrl+/ | Esc, ; |
| Check current file through Quil Language Service | Ctrl+Shift+J | Ctrl+Shift+J |
| Request Quil Language Service completion | Ctrl+Shift+Space | Ctrl+Shift+Space |

Default editing also supports Ctrl+Shift+Up for a cursor above, Ctrl+Shift+I
for a cursor on each selected line, Ctrl+G for next match, Ctrl+L for a line,
Ctrl+PageUp/PageDown for tabs, Ctrl+Shift+D to duplicate a line, and Ctrl+Shift+K
to delete a line. Ctrl+Z/Ctrl+Y undo and redo edits.

Ctrl+Shift+J starts Quil's native language-service helper with a run-scoped pipe,
sends the current Rust, Python or JavaScript text through LSP 3.17 JSON-RPC, and
shows its diagnostic count and first issue in the status line. The first helper
checks delimiter balance and trailing whitespace in UTF-8 files up to 256 KiB.
Ctrl+Shift+Space requests up to eight keyword completions for the typed prefix.
Use Up/Down to choose, Enter to insert, or Esc to dismiss. Quil refuses a choice
if the document revision or cursor changed after the request. Semantic
completion, navigation, rename and richer language analysis remain future work.

Vim mode keeps the default Ctrl shortcuts for application commands. Esc enters
normal mode, `i` enters insert mode, `dd` deletes a line, `u` undoes, `/` searches,
and `:w` saves. The in-app reference shows its Vim commands below the Ctrl shortcuts.

The desktop handles F11 to close a Quil window; Quil asks before discarding edits.

Ctrl+Shift+V reads `.git/index` inside the opened project's Files grant and
compares tracked files with the index. The panel marks modified, deleted,
conflicted, or too-large-to-check files. Enter opens a selected existing file.
It does not yet show untracked/ignored files, diffs, staging, commits or branches.
