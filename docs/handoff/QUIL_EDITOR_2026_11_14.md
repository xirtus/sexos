# Quil, the code editor: state and next slices (2026-11-14)

Workstream O of `APP_PARITY_PLAN_2026_11_14.md` (PR #217). Quil is the code editor; Jot stays the notepad.

## Done in this PR (O1, O2, O3)

- `crates/quil-core` (no_std, host-tested, 32 tests): the engine apart from any window.
  - `buffer`: a piece table over the file as opened, read lazily in 64 KiB blocks (64 cached), plus an append-only add buffer. Opening reads nothing; edits cost a few pieces. Line numbers come from a per-block newline index filled as blocks are read and in idle steps. Snapshots of the piece list make undo exact and cheap. Chunked `find`/`rfind` work on files of any size.
  - `editor`: any number of selections; typing, deleting, Ctrl+D next occurrence, select all occurrences, Ctrl+Shift+Up/Down column cursors, split into lines, indent/outdent, toggle comment, duplicate/delete/move/join lines, copy/cut/paste with one line per cursor. Typing groups into one undo step; compound changes (`begin_change`) undo as one.
  - `vim`: normal, insert, visual, visual line, visual block (Ctrl+V makes a cursor per line), command line. Counts, `d c y > <` with motions (`h j k l w b e W B E 0 ^ $ gg G f t F T ; , { } % n N`) and text objects (`iw aw i" a" i( a( i[ i{ ip`), `x X D C s S Y p P J r ~ u Ctrl+R . * #`, `/` and `?`, `:w :q :q! :wq :x :N :s :%s`. Works with several cursors.
  - `keymap`: named commands (the palette lists them all), VS Code keys by default, Emacs keys (`C-x` prefix, Escape as Meta). SexOS keys have no Alt, so VS Code's Alt chords are Ctrl+Shift.
  - `fuzzy`: fzf-style scoring for Go to File and the palette (10k paths ranked well under a second on the host).
  - `project`: folder walk (skips `.git`, `target`, `node_modules`...), file tree, search and replace across the folder (10k files under 1 s on the host).
  - `highlight`: a small lexer per language family until tree-sitter (O4); `workspace`: tabs.
- `apps/quil` (program `quil`, built into `iso_root/programs/quil`): draws all of it with Geist Mono through `sex-font`. Files up to 32 MiB are read into memory (undo survives saving); larger ones stay on disk and save by writing beside the file and renaming over it (undo history resets after such a save). Mouse: click, drag-select, wheel, tab bar, tree.
- Gate `scripts/program_quil_editor_gate.py` (CI job "Quil code editor"): Go to File, three cursors typing, Vim `dd` + `:w`, project search, and a 200 MB file (opened in 2 ms, edited at both ends, saved; hash checked on the host).

Serial markers are `[quil.code.*]`, apart from BOOT Quil's `[quil.*]`.

## Not yet

- BOOT Quil (`servers/quil`) still runs and is also titled Quil. Retiring it (O1's last part) touches many gates and Shell's `edit` routing; do it in its own PR, keeping document exchange.
- Tabs are drawn by Quil itself; move to Weft's tab widget when workstream M lands (Andreas asked for tabs across apps).
- Tab key and F-keys are kept by Shell, so Quil indents with Ctrl+] / Ctrl+[.
- No system clipboard between programs yet; Quil's clipboard is its own.

## Next slices

| Slice | Plan |
| --- | --- |
| O4 | tree-sitter through sex-libc for highlighting, folding, outline, bracket matching; replace `highlight` |
| O5 | `crates/sex-lsp` over F2 streams; rust-analyzer first |
| O6 | gitoxide: gutter, stage hunks, commit, blame, side-by-side diff, 3-way merge |
| O7 | terminal panel on sex-vt with tasks and problem matchers |
| O8 | debugger: needs a kernel debug grant (~150 lines, with workstream J) |
| O9 | Wasm extensions on sex-wasm |
| O10 | SSH remote folders (sex-ssh) |
| O11 | assistant panel, off by default |
| O12 | Jot polish (with the F thread, which owns Jot) |
