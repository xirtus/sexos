# Valentine draft power-cut proof

- Extended the native mail QEMU gate to compose a draft, wait for the three-second idle staged-save receipt, capture the open composer, and kill QEMU without a Close event or Ctrl+S.
- The gate reboots the same disk, checks the saved body through Spindle, opens the draft in Valentine, verifies its subject and captures `valentine-autosaved-reboot.ppm`.
- This proves recovery after a completed autosave in the guest. It does not claim that text entered after the last idle save survives power loss or that physical media flush ordering is proven.
- Validation: Python syntax and `git diff --check` locally. Full image build and both QEMU boots run only in GitHub CI; the Programs proof artifact supplies screenshots for the PR.
