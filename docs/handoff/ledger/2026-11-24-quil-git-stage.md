# Quil tracked text staging

- Added S in Quil's tracked Git panel to stage one modified, tracked UTF-8 file no larger than 64 KiB. Quil writes an authenticated loose blob, patches only that entry in a checked v2/v3 index, and replaces the index through Files' lock and rename. It rejects conflicts and indexes with extensions.
- Added U to undo only Quil's last stage. It rechecks the exact index bytes before replacing them, so another Git actor's update is not lost. A Git object left unreachable after undo is harmless and can be collected later.
- Host tests cover blob and index hashes, preserving other entries, and refusal of invalid files. The Editor QEMU gate stages and undoes `demo.rs`, then captures `quil-git-staged.ppm` and `quil-git-unstaged.ppm`.
- Validation: focused host tests and targeted x86 Quil check locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
