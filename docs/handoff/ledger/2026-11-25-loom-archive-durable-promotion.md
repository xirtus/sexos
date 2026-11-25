# Loom archive durability

- Gap: archive creation and extraction gave staged output its visible name
  before syncing its bytes, then ignored the final sync result. A power cut
  could leave a visible incomplete archive after a success receipt.
- Change: sync staged output before promotion and sync the visible rename
  before reporting success or offering undo. Failures before promotion clean
  the hidden part and report cleanup uncertainty; a failed final sync reports
  visible output as unconfirmed. Files now returns mounted-volume sync errors
  to the caller instead of logging them and replying OK; it still attempts to
  flush every mounted volume and commits Files' own tree.
- Proof: the Loom QEMU gate packs a ZIP, waits for the success receipt, cuts
  power immediately, reboots the same USB disk, opens the ZIP in Loom, captures
  `archive-powercut-restored.ppm`, then reads every byte of its member on the
  host. A `sex-volumes` host regression injects flush errors across four mounts,
  proving that every mount is attempted and the first error is returned. Existing
  archive round-trip gates still cover creation and extraction.
- Remaining: atomic no-replace promotion for directories on removable volumes
  needs a checked folder move in Files; the current file-only `MOVE_IF_ID`
  refuses external volumes.
