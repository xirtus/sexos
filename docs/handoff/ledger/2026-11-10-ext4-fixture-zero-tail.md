# ext4 fixture import keeps a sparse zero tail (2026-11-10)

- **Failure:** Host regressions on the release-key PR (#122), CI run 42815579564,
  failed `ext_random_trees_from_mke2fs`: seed `0x2fa692b7`, ext4 1 KiB blocks,
  256-byte inodes, `inline_data,^flex_bg`. The generated source
  `/n12-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx/n23-xxxxxxxxxxxxxxxxxxxxxxx` has
  4097 bytes and its final byte is zero; the mounted image reported 4096.
- **Independent diagnosis:** reproduced with mke2fs 1.47.4. Before any SexOS read,
  e2fsprogs `debugfs stat` reports that inode's size as 4096 and `debugfs dump`
  also returns exactly 4096 bytes, matching SexOS. The importer lost the all-zero
  final partial block in the resulting extent-backed inode.
  This is malformed fixture population relative to its source, not evidence of
  truncation by the SexOS reader. No production filesystem code changes.
- **Fixture repair:** after importing a generated tree, explicitly declare each
  file's intended logical size with debugfs. A missing final zero block remains
  sparse; every original source byte is still compared, including the zero tail.
  Test tool discovery also honors PATH so real host tools can be used on macOS.
- **Regression:** replay the reported seed alongside fresh random trees across
  ext2, ext3, ordinary ext4 and inline-data ext4. The regression requires that the
  4097-byte zero-tail file was exercised, reads the entire original content, then
  reads across EOF at offset 4095: two bytes returned, final zero preserved, and
  bytes beyond EOF untouched. Replay and random tests use separate temporary
  paths so parallel tests cannot overwrite each other's images.
- **Validation:** all 10 ext4 host tests pass with real mke2fs/debugfs; 20 repeated
  replay-plus-random runs across four variants pass (160 generated trees). The full
  volumes host gate passes all 38 tests with real mkfs/fsck/mtools on PATH. CI remains
  required. Hardware/filesystem write safety claims are unchanged.
