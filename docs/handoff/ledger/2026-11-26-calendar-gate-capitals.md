# Calendar gate types capital letters with Shift

The Date calendar gate typed `12:34 Review` by sending `R` as a QEMU qcode,
which QEMU refuses. The gate's `keys()` helper now sends capitals as
Shift + the lower-case key. Gate-only change; Date itself is unchanged.
