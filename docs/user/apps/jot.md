# Jot (Notepad)

> **Naming Exception**: Under `docs/DESIGN_GUARDRAILS.md` §6, **Jot** is the **sole plain-name exception** across all of SexOS. It keeps its straightforward name because it is built for quick jotting: instant notes, scratch text, and rapid edits without project setup overhead.

**Jot** (`apps/jot`, `/bin/jot`) is the lightweight desktop notepad. Designed for near-instant launch times and minimal resource consumption, Jot provides distraction-free text editing backed by the `sex-textedit` engine.

---

## Launching Jot

- **From Spindle**:
  ```text
  run jot                   # Open an empty notepad
  edit notes.txt            # Built-in shortcut: opens notes.txt with an immediate custody grant
  run jot @todo.md          # Open todo.md with an explicit single-file grant
  ```
- **From Voyeur**: Press **`Super+D`**, type `jot`, and press **`Enter`**.

---

## Editing Controls & Shortcuts

| Shortcut | Action | Description |
|---|---|---|
| **`Arrow Keys`** | **Navigate** | Move text cursor freely throughout the note. |
| **`Home / End`** | **Line Bounds** | Jump to start or end of current line. |
| **`Ctrl+S`** | **Save** | Save the note directly to storage. |
| **`Ctrl+Z`** | **Undo** | Undo recent keystrokes. |
| **`Ctrl+F`** | **Find** | Search for keywords within the active note. |
| **`Ctrl+P`** | **Markdown Preview**| Toggle formatted markdown rendering for headings, lists, and code blocks. |
| **`Ctrl+C / X / V`** | **Clipboard** | Copy, cut, and paste text. |
| **`Ctrl+W`** | **Close** | Close Jot; prompts to save if changes are uncommitted. |

---

## Working with Other Applications

### Drag and Drop
Jot fully supports Silk desktop drag-and-drop (`sex_pdx::dnd`):
- Drag a text file from **[Loom](loom.md)** onto Jot's window to open it immediately.
- Drag highlighted text from **[Kaleidoscope](kaleidoscope.md)** or another application directly into Jot to insert it at your cursor position.

### Sandboxed Storage
Unless given a specific file with `edit FILE` or `@file`, Jot stores all personal notes safely within its confined application vault at `/data/jot/`.
