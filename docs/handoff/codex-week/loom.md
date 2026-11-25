# Loom daily-driver gaps

Loom is the SexOS file manager; SexFiles is its storage service. Compared with
Finder and Dolphin, Loom already has batch rename with preview and undo,
archive browsing and extraction, trash and restore, thumbnails and preview
cards, background jobs, and search.

Remaining gaps:

- Independent folder tabs. This change adds them with Ctrl+Shift+T to open,
  Ctrl+PageUp/PageDown to switch, and Ctrl+Shift+W to close.
- Text-mode split panes now show two independent folder tabs side by side; the
  active tab receives navigation keys. A graphical split grid and places sidebar
  remain.
- Broader preview types remain. ZIP, plain tar, tar.gz and tar.zst can now be
  created from a selected file or folder with native controls.
- Folder tabs now restore from a versioned session file under the granted
  root, with a recoverable previous copy. An encrypted-home unlock and missing
  removable-volume restore policy still need a full session audit.
- Clearer file version history.
- The thumbnail grid and disk usage map now follow Tatami's light, dark and high
  contrast tokens. Pictures retain a dark photo well because their RGB cache
  already contains that well; this avoids stale transparency halos across themes.
- The in-app controls have a standalone reference in `docs/apps/loom-shortcuts.md`.
  Ctrl+Y now replays undone batch renames and trash moves, with metadata and trash
  origin notes restored. Ordinary rename and empty-folder creation are now
  replayable too, as are cut-and-paste moves across and within volumes.
  Cross-volume undo and redo wait for their background transfer before changing
  history. Redo for copy, plus a forced power-cut session recovery gate, remain.
