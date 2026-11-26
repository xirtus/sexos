# Kaleidoscope keyboard reference

Kaleidoscope now has an offline `about:shortcuts` page and Ctrl+Shift+/ opens it.
The page and `docs/apps/kaleidoscope-shortcuts.md` list the browser's existing
controls, including the Chastity consent shortcut. No new capabilities or storage
grants are involved.

The browser QEMU gate opens the page with the shortcut, checks the navigation
record, and captures a changed pixel surface as `web-shortcuts.ppm`. Full builds,
host tests, and QEMU checks run on GitHub CI.
