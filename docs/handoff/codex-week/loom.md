# Loom daily-driver gaps

Loom is the SexOS file manager; SexFiles is its storage service. Compared with
Finder and Dolphin, Loom already has batch rename with preview and undo,
archive browsing and extraction, trash and restore, thumbnails and preview
cards, background jobs, and search.

Remaining gaps:

- Independent folder tabs. This change adds them with Ctrl+Shift+T to open,
  Ctrl+PageUp/PageDown to switch, and Ctrl+Shift+W to close.
- Split panes show two independent folder tabs side by side in text lists or
  graphical tile grids. Ctrl+G changes the view; clicking either grid pane
  focuses its tab and clicking a tile selects it. Ctrl+PageUp/PageDown changes
  active tab by keyboard. The inactive grid pane uses previews already cached
  from a visit there; uncached items show their SexOS file icons until focused.
  A short or narrow window falls back to the text split list. Ctrl+Shift+B now
  opens a compact local Places list with the granted root, previous folder and
  available SexOS Home, Downloads, Documents and Disks folders, in single or
  split views. A persistent favorites editor and graphical sidebar remain.
- Broader preview types and archive creation formats. ZIP and tar.zst can now
  be created from a selected file or folder. Ctrl+Shift+F opens a format picker
  for ZIP, plain tar, tar.gz and tar.zst, all using the same staged streaming
  writer and one-step undo. Archive creation and extraction now sync the hidden
  complete output before promotion and the visible rename before reporting
  success. A failed final sync reports unconfirmed storage instead of offering
  undo. Archive files on SexFS and encrypted home now use no-replace checked
  promotion; a late name collision preserves the other file. A failed
  encrypted-home flush is also reported. More preview types and checked
  directory promotion on removable volumes remain open.
- Folder tabs now restore from a versioned session file under the granted
  root, with a recoverable previous copy. An encrypted-home unlock and missing
  removable-volume restore policy still need a full session audit.
- Clearer file version history.
- Text folder lists support contiguous Shift+Up/Down/Home/End selection.
  Ctrl+Space toggles a row in a noncontiguous text-list selection; Ctrl+arrows
  move focus without losing the marks. The same batch trash, copy and move
  paths consume these marks in folder order. Plain arrows clear the selection.
  Delete sends every selected item to trash in one recoverable action, and one
  Ctrl+Z restores the batch. Ctrl+X now cuts a selected range, and Ctrl+V moves
  it to another folder on the same disk with collision-safe names; one Ctrl+Z
  reverses the group. Ctrl+C now copies up to 64 selected list items through
  one background job, with reserved collision-safe names, cleanup on failure,
  copied Loom metadata, and one-step undo.
  Background copies now verify staged size and hash by reading each `.part`
  file back before promotion, cleaning up a mismatch or read failure. The
  final post-promotion sync must also succeed before the job reports success.
  A failed sync attempts copy rollback and reports unconfirmed storage state
  when cleanup or durability cannot be established.
  Ctrl-click toggles a grid tile in both single and split layouts; Ctrl+Space
  and Ctrl+arrows provide a keyboard path. Selected tiles use the accent fill,
  while a separate focus ring tracks keyboard focus. The same batch operations
  and one-step undo apply in both views. Grid drag of a selected group and
  cross-disk batch move remain open. Cross-disk batch cut still needs a
  source-trash transaction and a distinct undo path.
- The thumbnail grid and disk usage map now follow Tatami's light, dark and high
  contrast tokens. Pictures retain a dark photo well because their RGB cache
  already contains that well; this avoids stale transparency halos across themes.
- The in-app controls have a standalone reference in `docs/apps/loom-shortcuts.md`.
  Folder navigation now saves the active tab's new path after startup recovery.
  The GitHub CI gate cuts VM power after a completed two-tab session save,
  reattaches the same USB disk and captures the restored tabs. Redo remains open.
