# Loom (File Manager)

> **Identity Note**: **Loom** (`apps/files`, `/bin/loom`) is the SexOS graphical file manager, named after the weaving frame upon which all fabrics are structured. In Spindle, typing `loom` or `files` opens Loom.

**Loom** provides an intuitive, high-speed interface for navigating persistent storage volumes managed by **sexfiles** and **BDSM**. It features background file operations with cancellation, trash recovery, archive inspection, live preview cards, safe removable media ejection, and capability grant tracking.

---

## Launching Loom

- **From Spindle**:
  ```text
  run loom                # Open Loom in home directory
  run loom /data/photos   # Open Loom directly at a specific folder
  ```
- **From Voyeur**: Press **`Super+D`**, type `loom`, and press **`Enter`**.

---

## Keyboard Controls & Navigation

| Shortcut | Action | Description |
|---|---|---|
| **`Arrow Keys`** | **Navigate Items** | Move cursor selection through files and folders. |
| **`Enter`** | **Open** | Enter selected folder or launch file in its associated default application. |
| **`Backspace`** | **Parent Folder** | Ascend to the parent directory. |
| **`Space`** | **Quick Look Preview** | Open instant floating preview card for images, text, or PDF documents. |
| **`Ctrl+C`** | **Copy** | Stage selected file(s) for copy. |
| **`Ctrl+X`** | **Cut** | Stage selected file(s) for move. |
| **`Ctrl+V`** | **Paste** | Execute background copy or move into the active folder. |
| **`Delete`** | **Move to Trash** | Safely send item to Trash volume (`/.trash`). |
| **`Ctrl+Z`** | **Undo** | Undo the last file copy, move, or trash operation. |
| **`Ctrl+F`** | **Search** | Filter files instantly by name or full-text indexed contents. |
| **`Ctrl+E`** | **Safe Eject** | Flush buffers and safely dismount removable USB storage. |

---

## Key Capabilities

### 1. Zero-Copy Drag and Drop
Loom integrates seamlessly with the **Silk** desktop drag-and-drop engine (`sex_pdx::dnd`). Select a file in Loom and drag it across your screen:
- Drop into **[Jot](jot.md)** to edit text files.
- Drop into **[Kaleidoscope](kaleidoscope.md)** to render local HTML or images.
- Drop into **[Vellum](vellum.md)** to open PDF documents.
- Upon release, **Collar** transfers an unforgeable capability token (`fs::OFFER`) exclusively for the dropped file, preserving least-privilege security.

### 2. Archives & Compressed Bundles
Loom handles `.zip`, `.tar`, `.tar.gz`, `.tar.zst`, and `.7z` archives natively:
- Press **`Enter`** on an archive to explore its internal structure as a virtual folder without extracting.
- Press **`Ctrl+Shift+E`** to extract contents into a destination folder.

### 3. Capability Grant Inspector
SexOS files track which protection domains have been granted custody. Selecting any file and pressing **`Ctrl+I`** reveals:
- File size, creation date, and hash.
- **Active Grants**: Which applications currently hold open read or write capability tokens for this specific file, with options to instantly revoke access.

### 4. Trash & Safe Deletion
Files deleted in Loom are moved to the encrypted Trash store (`/.trash`) and can be restored at any time. When you empty Trash, BDSM zeroes the underlying memory pages immediately.

### 5. Opening Files in Other Applications (Shell Launch Requests)
Pressing **`Enter`** on a document in Loom does not pass ambient file descriptors or invoke ad-hoc Unix IPC. Instead, Loom dispatches a typed shell launch/open request (`OP_LAUNCH` / `sex_pdx::launch`) to **Silk**. Silk launches or raises the associated target application (e.g., **[Vellum](vellum.md)** for PDFs or **[Gaze](gaze.md)** for images) with an explicit, unforgeable single-file custody grant (equivalent to `run vellum @manual.pdf`), preserving least-privilege capability confinement.
