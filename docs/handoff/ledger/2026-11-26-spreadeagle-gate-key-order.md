# Spreadeagle gate keeps key order between QEMU's two key paths

`send-key` presses go through QEMU's delayed key queue (80 ms holds), while
the gate's Ctrl+Shift chords use `input-send-event`, which skips that queue.
After `Space, Enter` the gate's next Ctrl+Shift+O could reach Spreadeagle
before them, land in the still-open filter menu and be ignored. The chord
helpers now pause briefly first so queued presses land in order. Gate-only.
