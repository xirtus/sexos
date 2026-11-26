# Spreadeagle: native validation dropdown

- A selected cell with an in-cell list validation shows a dropdown arrow. Alt+Down or the arrow opens a native menu; Up/Down and Enter or a clicked row commits a choice through the validated cell entry path. Esc closes it. The menu reads literal and live range-backed choices from GridCraft, and respects imported rules that suppress the dropdown.
- A focused host test checks literal and live source choices and XLSX reopen. The Programs QEMU gate captures `spreadeagle-validation-dropdown.ppm` after reopen and chooses a value through the keyboard.
- Paste/fill validation remains open.
