# Loom: prove tab recovery after power loss

- The Files guest gate now cuts QEMU power after Loom confirms a staged and synced two-tab session save. It boots the same system and USB disks, reopens Loom, checks the active `docs` tab and the other root tab, and captures `folder-tabs-powercut-restored.ppm`.
- This tests recovery after the save receipt. A power cut during the earlier write phase is covered by the session's previous-copy fallback in code but still needs fault injection. Loom's redo behavior remains an open gap.
