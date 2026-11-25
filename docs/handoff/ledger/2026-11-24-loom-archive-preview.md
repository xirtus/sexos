# Loom archive member preview

- Gap: A person could browse an archive but had to extract a member to inspect a text note. Finder and Dolphin offer quick inspection while browsing.
- Change: F3 previews a UTF-8 member directly from zip, tar, and other supported archive views in Loom. It refuses unknown-size or over-32-KiB members and bounds decompressed bytes again at the sink. The existing inert text parser rejects binary and display controls. No member is written to the user's folder.
- Tests: Loom core host tests check bounded streamed bytes and safe preview text. The Files QEMU gate previews `docs/alpha.txt` inside `docs.zip`, captures `archive-text-preview.ppm`, then closes the preview and continues archive operations. Full builds and QEMU run only on GitHub CI.
