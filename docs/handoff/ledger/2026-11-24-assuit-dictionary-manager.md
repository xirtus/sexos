# Assuit dictionary manager

- Ctrl+Shift+D opens Assuit's custom word list. A adds an arbitrary validated word; Up/Down selects and Delete removes it. Both edits persist in Assuit's own dictionary file, separate from a document's exact-file grant. Failed removals restore the proofing engine's in-memory word.
- A host test adds, lists, removes and reopens a dictionary to verify the removal persists. The Assuit QEMU gate adds a spelling word, opens the manager, captures `assuit-dictionary-manager.ppm`, removes and re-adds it through the new controls.
- Full image build and QEMU run on GitHub CI; focused host, Rust parser and gate syntax checks run locally. A local guest-target check stops in ring's C build because this Mac lacks the SexOS C sysroot (`assert.h`); the GitHub image build provides that toolchain.
