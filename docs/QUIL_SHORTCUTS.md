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
| Show document symbols | Ctrl+Shift+M | Ctrl+Shift+M |
| Go to a declaration in the open file | Ctrl+Shift+G | Ctrl+Shift+G |
| Describe a declaration in the open file | Ctrl+Shift+H | Ctrl+Shift+H |

Default editing also supports Ctrl+Shift+Up for a cursor above, Ctrl+Shift+I
for a cursor on each selected line, Ctrl+G for next match, Ctrl+L for a line,
Ctrl+PageUp/PageDown for tabs, Ctrl+Shift+D to duplicate a line, and Ctrl+Shift+K
to delete a line. Ctrl+Z/Ctrl+Y undo and redo edits.
Ctrl+D starts with the word at the cursor, then adds each remaining occurrence,
wrapping through the document and skipping matches already selected.

Ctrl+Shift+J starts Quil's native language-service helper with a run-scoped pipe,
sends the current Rust, Python or JavaScript text through LSP 3.17 JSON-RPC, and
shows its diagnostic count and first issue in the status line. A bounded picker
lists up to 16 issues; use Up/Down and Enter to jump, or Esc to close it. The first helper
checks delimiter balance and trailing whitespace in UTF-8 files up to 256 KiB.
Ctrl+Shift+Space requests up to eight same-file declaration or keyword
completions for the typed prefix.
Use Up/Down to choose, Enter to insert, or Esc to dismiss. Quil refuses a choice
if the document revision or cursor changed after the request. Semantic
completion, cross-file navigation, rename and richer language analysis remain future work.

Ctrl+Shift+M asks the same service for document symbols and opens an outline.
Use Up/Down and Enter to jump to a declaration; Esc closes it. The bounded
first service recognizes Rust functions, structs, enums, traits, modules and
implementations; Python functions and classes; and JavaScript functions and
classes. It does not parse full language syntax or resolve semantic references.

Ctrl+Shift+G sends a `textDocument/definition` request for the one cursor. The
bundled helper follows an identifier to a unique declaration head in the same
open file. It refuses ambiguous names and other-file locations. Imports,
scopes, member references and full semantic definition lookup still need a
language server with broader analysis.

Ctrl+Shift+H sends a `textDocument/hover` request and shows the recognized
declaration kind, name and line in the status bar. It uses the same unique
same-file declaration rule; it does not infer types or documentation.

Vim mode keeps the default Ctrl shortcuts for application commands. Esc enters
normal mode, `i` enters insert mode, `dd` deletes a line, `u` undoes, `/` searches,
and `:w` saves. The in-app reference shows its Vim commands below the Ctrl shortcuts.

The desktop handles F11 to close a Quil window; Quil asks before discarding edits.

Quil writes a checked recovery draft every two seconds for edited tabs up to
4 MiB. On the next launch it restores up to eight untitled tabs, numbered in
the tab bar. Reopening a named file restores its draft. Save a recovered tab
to give it a file name. Confirming Close Tab or Quit discards its draft.

Ctrl+Shift+V reads `.git/index` and `.git/HEAD` inside the opened project's Files grant and
compares tracked files with the index. The panel marks modified, deleted,
conflicted, or too-large-to-check files and shows the current branch or detached
HEAD when valid. `N` marks a bounded file outside the index; Git ignore rules
are not evaluated, so these are candidates rather than definitive untracked
status. Right opens a read-only line diff for a
modified or deleted UTF-8 file whose indexed blob is loose; Up/Down scroll it.
The diff is bounded to 64 KiB and 300 lines per side. Enter opens an existing file.
In the tracked-changes panel, S stages one modified tracked UTF-8 file up to
64 KiB or one deleted tracked file.
U undoes only Quil's most recent stage, and refuses if another Git actor has changed
the index. Quil writes a checked loose blob and replaces a v2/v3 index through
Files' staged rename. Indexes with extensions, conflicts, untracked/ignored
files, broader unstage controls, commits and branches need later work.
