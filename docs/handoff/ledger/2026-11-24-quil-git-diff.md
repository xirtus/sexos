# Quil tracked file diff

- Added Right from Quil's tracked Git changes panel to show a read-only line diff for modified or deleted UTF-8 files. It reads an authenticated loose blob from the project's existing Files grant; no shell or Git process runs. Packed blobs and binary/large files get explicit messages.
- Bounded decompression, object SHA-1, blob header/length, 64 KiB text and 300 lines per side. Host tests check separate hunks, tampered object refusal, and binary/size limits. The Editor QEMU gate installs a real loose object, opens the diff and captures `quil-git-diff.ppm`.
- Validation: focused host test and Python syntax check locally; full image build and QEMU gate only in GitHub CI. No local target or disk image is kept.
