# Loom split folder lists

Ctrl+Shift+\\ shows the active folder tab and the next tab side by side in the
list view. Ctrl+PageUp/PageDown selects the tab receiving navigation keys. Both
folders remain inside the existing Files grant, and the ordinary tab session
continues to persist their paths and positions. The split display itself is
temporary and closes automatically if only one tab remains.

The host formatter test checks that a long left name cannot obscure the right
pane. GitHub CI's Loom QEMU gate navigates one folder independently, returns to
the other tab, and captures `folder-split.ppm`.
