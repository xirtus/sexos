# Gates: the shared QMP key helper types capitals

`desktop_close_gate.QMP.key`, which most runtime gates use to type, passed a
character straight to QEMU's `send-key` as a qcode. Qcodes are lower case, so
any typed capital (the browser download gate's URL had a `D`) failed with
"Parameter 'data' does not accept value 'D'". The helper now sends a single
capital letter as Shift plus the lower-case key, which fixes every gate that
types through it at once. Gate-only change.
