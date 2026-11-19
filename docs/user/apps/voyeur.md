# Voyeur (Search & Launcher)

> **Identity Note**: **Voyeur** (`apps/voyeur`, `servers/silk-shell/src/voyeur.rs`) is the SexOS universal search overlay, command palette, and spotlight launcher, named under Andreas's themes ("sees everything").

**Voyeur** is the central search and navigation hub of SexOS. It seamlessly unites program launching, indexed file search, inline mathematical calculation, settings shortcut discovery, open window switching, and emoji lookup into a single, lightning-fast keyboard overlay.

---

## Summoning Voyeur

Voyeur can be invoked from any application or workspace at any time:
- **`Super`** (quick tap alone)
- **`Super+D`**
- **`` ` ``** (backtick)

Press **`Escape`** to dismiss the overlay and return focus to your previous window.

---

## What You Can Search & Execute

### 1. Launching Applications
Start typing the name or description of any software:
- Typing `loom` or `files` brings up **[Loom](loom.md)**.
- Typing `edit` or `code` highlights **[Quil](quil.md)**.
- Typing `net` or `browser` highlights **[Kaleidoscope](kaleidoscope.md)**.
- Press **`Enter`** to launch the top result immediately.

### 2. Instant Math with Abacus
Voyeur evaluates math expressions on the fly without needing to launch a dedicated window:
- Type `25 * 40` → answers `1000`.
- Type `sqrt(144)` → answers `12`.
- Press **`Enter`** to copy the evaluated result straight to your clipboard.

### 3. Jumping to Tatami Settings
Skip navigating menus:
- Type `wifi` → jumps directly to Tatami's Wi-Fi network panel.
- Type `grants` → jumps to Tatami's Privacy & Capability Grants manager.
- Type `display` → opens screen resolution and scaling controls.

### 4. File & Document Search
Voyeur connects to the **Loom** indexed content service:
- Type document titles or keywords to discover matching PDFs, notes, pictures, and source files across your storage volumes.
- Press **`Enter`** to open the selected document in its default viewer.

### 5. Window Switching
If an application is already running on another workspace, typing its name in Voyeur presents an active window switch action:
- Selecting an open window instantly shifts your screen to that workspace and raises the window to focus.

### 6. Emoji Lookup
Type colon syntax (e.g., `:rocket:`, `:heart:`, `:check:`) to find emoji and copy them directly into your active editor.

---

## Frecency Ranking

Voyeur incorporates a frecency-weighting algorithm (frequency + recency). As you use SexOS daily, Voyeur automatically promotes the applications, documents, and settings pages you open most frequently, ensuring your most vital workflows are accessible in one or two keystrokes.
