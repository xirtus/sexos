# Loom daily-driver gaps

Loom is the SexOS file manager; SexFiles is its storage service. Compared with
Finder and Dolphin, Loom already has batch rename with preview and undo,
archive browsing and extraction, trash and restore, thumbnails and preview
cards, background jobs, and search.

Remaining gaps:

- Independent folder tabs. This change adds them with Ctrl+Shift+T to open,
  Ctrl+PageUp/PageDown to switch, and Ctrl+Shift+W to close.
- Split panes and a full graphical places sidebar.
- Broader preview types and archive creation formats.
- Folder tabs now restore from a versioned session file under the granted
  root, with a recoverable previous copy. An encrypted-home unlock and missing
  removable-volume restore policy still need a full session audit.
- Clearer file version history.
- The thumbnail grid and disk usage map now follow Tatami's light, dark and high
  contrast tokens. Pictures retain a dark photo well because their RGB cache
  already contains that well; this avoids stale transparency halos across themes.
