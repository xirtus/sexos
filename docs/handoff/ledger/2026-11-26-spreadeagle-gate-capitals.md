# Spreadeagle gate types capitals and commas

The Spreadeagle gate's `type_line` sent capital letters and commas as raw QEMU
qcodes, which QEMU refuses (`Item 1100`, `Food,Fun`). It now sends capitals as
Shift + the lower-case key and maps `,` and `;` to their qcodes. Gate-only.
