# Kaleidoscope Shields settings recovery

- Per-site Shields choices now use the browser's staged, synced state replacement and previous-copy recovery. If a save fails, the in-memory toggle rolls back so the view does not claim an unsaved choice.
- The existing Shields QEMU gate turns Shields off for one site, cuts power after the save receipt, reboots the same disk, verifies that site remains off, and captures `shields-recovered-after-powercut.ppm`.
- Full image build and QEMU run on GitHub CI; targeted app and Python gate syntax checks run locally.
