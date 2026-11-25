# Loom keyboard shortcuts

Loom manages files through SexFiles grants. The current folder, selection and any
open prompt determine which keys act. F11 closes the window.

| Action | Shortcut |
| --- | --- |
| Open a file or enter a folder | Enter |
| Go to the parent folder | Backspace |
| Rename the selected item | F2 |
| Select a range in the text folder list | Shift+Up/Down/Home/End |
| Toggle a row in a text-list selection; move focus without clearing marks | Ctrl+Space; Ctrl+Up/Down/Home/End |
| Show size and checksum | Ctrl+I |
| Copy, cut and paste | Ctrl+C, Ctrl+X, Ctrl+V |
| Create a folder | Ctrl+N |
| Move to trash; delete for good after confirmation | Delete; Shift+Delete |
| Undo a file operation | Ctrl+Z |
| Show trash; restore from trash | Ctrl+T; Ctrl+R while in trash |
| Empty trash; eject a removable disk | Ctrl+E in the corresponding view |
| Go home; show removable disks | Ctrl+H; Ctrl+D |
| Show local Places (Start, Previous, and available Home, Downloads, Documents, Disks) | Ctrl+Shift+B |
| Search file names and contents | Ctrl+F |
| Toggle thumbnail grid in one or both split panes; show disk usage | Ctrl+G; Ctrl+U |
| Open a new folder tab; close the current one | Ctrl+Shift+T; Ctrl+Shift+W |
| Switch tabs; show or hide split view | Ctrl+Page Up/Down; Ctrl+Shift+\ |
| Browse an archive; extract it | Enter; Ctrl+Shift+X |
| Create ZIP or tar.zst from the selection | Ctrl+Shift+A; Ctrl+Shift+M |
| Batch rename with preview | Ctrl+Shift+R |
| Compare folders; find duplicates | Ctrl+Shift+C; Ctrl+Shift+D |
| Rate from zero to five stars; edit tags | Ctrl+0 through Ctrl+5; Ctrl+Shift+G |
| Open context actions | Ctrl+Shift+O |
| Lock or unlock encrypted home; change its password | Ctrl+K; Ctrl+Shift+K |

Enter confirms a prompt and Escape cancels it. When a background job is active,
Escape stops it; file-changing shortcuts wait until the job ends. Deletion for
good and emptying trash require a second confirmation. Undo is available for
supported file operations in the current Loom run; redo is not yet available.
Delete moves selected text-list rows to trash as one action, and one Ctrl+Z restores
them. Ctrl+X cuts selected text-list rows; Ctrl+V moves them into another
folder on the same disk, and one Ctrl+Z moves it back. Ctrl+C copies a selected
text-list selection of up to 64 items into this or another folder; Ctrl+V starts
one background job and Ctrl+Z removes the completed group. A taken name receives
`(copy)` or `(copy 2)` before its extension. Copied ratings and tags follow
the new items. Plain arrows clear marks; Ctrl+arrows preserve them. Grid batch selection remains open.
Ctrl+Shift+R opens the batch rename preview.
In split grid view, a click in either pane focuses that folder. Clicking a tile
also selects it; Ctrl+Page Up/Down changes the active folder from the keyboard.
Local Places lists only folders available through this Loom window's Files grant.
Use Up/Down and Enter to navigate; Escape returns to the folder view.
