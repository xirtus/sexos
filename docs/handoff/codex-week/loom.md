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
  writer and one-step undo. Ctrl+Shift+N also packs plain tar directly; tar.gz
  stays in the picker because Ctrl+Shift+B belongs to local Places. Archive
  creation and extraction now sync the hidden
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
- Trash now stores and syncs each restore path before moving its item, then
  syncs the moved items before reporting success or offering undo. A failed
  note, move, or final sync attempts rollback and reports unconfirmed storage
  if rollback cannot be verified. A stale restore note also reserves its name.
  Origin-note creation and contents share one Files request; mounted volumes
  without stable object IDs keep notes on uncertain rollback for recovery.
  Restore and undo still need the same durability audit; directory moves on
  SexFS still lack a checked no-replace promotion.
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
  reattaches the same USB disk and captures the restored tabs. Ctrl+Y redoes
  one undone batch rename, with checked source/destination moves on SexFS and a
  QEMU screenshot of the redone folder; removable volumes cannot offer atomic
  collision checks without stable IDs, so their batch rename redo is refused.
  Ctrl+Y also replays undone single-item trash moves, with metadata and trash
  origin notes restored. Ordinary rename and empty-folder creation are
  replayable too, as are cut-and-paste moves across and within volumes.
  Cross-volume undo and redo wait for their background transfer before changing
  history. Redo for copy and for multi-item trash, cut and copy groups remain.
- Quick look: F3 opens a read-only, bounded UTF-8 text preview with line
  scrolling; the QEMU gate captures the actual preview. F3 also shows a
  selected PNG, JPEG, GIF or WebP at window size inside Loom, using the bounded
  native image decoder. Escape returns to the folder without starting another
  app. Picture inputs over 16 MiB are refused. The text preview shows the first
  32 KiB only and rejects binary or control-bearing files; Enter still opens
  the chosen app. Richer document previews remain.
