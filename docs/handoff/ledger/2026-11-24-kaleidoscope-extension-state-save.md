# Kaleidoscope extension state recovery

- Extension enabled/disabled choices and runtime rules/content-script registration now use staged, synced state replacement with a previous copy. A failed enablement save rolls back the in-memory toggle and shows an error. Removing an extension also removes its previous and staged runtime state, so reinstall cannot revive it.
- The extension QEMU gate saves a dynamic rule and turns the extension off, then hard-stops QEMU. On the same disk it checks that the extension remains off with both rules loaded, verifies blocked resources now load, and captures `extension-off-after-powercut.ppm`.
- Full image build and QEMU run on GitHub CI; targeted browser app and gate syntax checks run locally.
