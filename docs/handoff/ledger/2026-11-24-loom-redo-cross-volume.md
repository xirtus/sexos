# Loom redo across volumes

- Cross-volume Ctrl+Z and Ctrl+Y now keep their history step pending until the background move completes. A cancelled or failed transfer before source removal restores the step to its original stack; occupied destinations are refused before a job begins. If cleanup fails after the destination is committed, Loom asks the person to inspect both locations and does not replay a stale history step.
- The Files QEMU gate, now wired into the Devices CI job, moves `HELLO.TXT` from a FAT12 stick to the SexOS volume, undoes, redoes, captures `files-cross-volume-redone.ppm`, then undoes and checks the original content before unmounting.
- Validation: focused `cargo check -p files --target x86_64-unknown-none --offline`, Python gate syntax, and diff checks locally; full build and QEMU proof on GitHub CI only. Copy redo remains open because the source may change after an undo.
