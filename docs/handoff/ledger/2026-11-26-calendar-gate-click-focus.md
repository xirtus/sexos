# Calendar gate can focus the remaining Date window by clicking it

After the first of two Date windows closes, 24 Tab steps did not reach the
second window, and the gate stopped with "bounded focus traversal failed".
The gate now falls back to clicking the window's title bar (from its
`[shell.tile.apply]` geometry) and waits for `[shell.focus.set]` on it.
Gate-only change.
