# Valentine keyboard shortcuts

Press **Ctrl+Shift+/** for the in-app reference. Escape or the same shortcut
closes it. The reference works while editing a draft without discarding it.

| Action | Shortcut |
| --- | --- |
| Choose and open a local message | Up/Down, Enter |
| Filter sender, subject and preview | Type; Escape clears the filter |
| Reload local messages | Ctrl+R |
| Cycle the local folder and nested folders (up to four levels) | Ctrl+G |
| Start an unsent draft | Ctrl+N |
| Edit an opened unsent draft | Ctrl+E |
| Move between draft fields | Tab or Enter in To/Subject |
| Insert a line in the draft body | Enter |
| Save a draft | Ctrl+S; Escape also saves changes before leaving |
| Undo or redo a draft edit | Ctrl+Z / Ctrl+Y |
| Show account status | Ctrl+M |
| Choose an attachment in the opened message | Ctrl+Alt+A |
| Save the chosen attachment through Loom | Ctrl+Alt+S |
| Scroll an opened message | Mouse wheel |
| Close Valentine | F11 |

Valentine currently reads local `.eml` files in its Files grant. Ctrl+Alt+S asks
Loom where to save a decoded attachment, refuses an existing filename, and reports
success only after Files confirms the write. If a write fails, Valentine attempts
to remove the incomplete file and reports when that cleanup fails. Ctrl+G cycles
through its top level and nested folders (up to four levels and 64 folders);
drafts save in the current folder. Account status explains when live mail is
unavailable.
