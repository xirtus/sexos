# Assuit shared clipboard

- Ctrl+C publishes selected text from WordCraft to SexOS's Shell clipboard. Ctrl+X publishes it before cutting, so a failed transfer leaves the document untouched. Ctrl+V reads the current focused-window clipboard and passes its text to WordCraft; the engine keeps its rich fragment only when the system text still matches.
- Prompt keys do not publish document text. Shell accepts transfers only while Assuit is focused immediately after the person's key press.
- A focused host test passes text between two documents, rejects a stale internal fragment, and checks undo. The QEMU Assuit gate copies selected text into Jot, captures `assuit-clipboard-in-jot.ppm`, then pastes it back into Assuit. Full build and QEMU execution run on GitHub CI.
- The shared service currently carries text. Rich interchange with other programs and clipboard images remain open.
