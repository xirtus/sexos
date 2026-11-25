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
- Broader preview types and archive creation formats. ZIP and tar.zst can now
  be created from a selected file or folder; plain tar and tar.gz creation still
  need file-manager controls.
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
  replayable too. Redo for copy and cross-disk move, plus a forced power-cut
  session recovery gate, remain.
