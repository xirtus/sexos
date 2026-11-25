# Date power-cut event proof

- Extended the calendar QEMU gate to add a local event, wait for the staged-save commit receipt, capture Date, and kill QEMU without closing the app.
- The gate reboots the same disk, checks Date loaded the event, opens its day to verify the title and time, and captures `date-after-powercut.ppm`.
- The proof is for a completed guest save, not for physical media flush ordering or an interrupted write before the receipt.
- Validation: Python syntax and `git diff --check` locally. Full image build and both QEMU boots run only on GitHub CI; the Productivity proof artifact supplies the screenshots.
