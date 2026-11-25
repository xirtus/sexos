# Loom: prove tab recovery after power loss

- Folder navigation now saves the active tab's path immediately after startup recovery; startup still loads the previous session before enabling these writes. The Files guest gate cuts QEMU power after Loom confirms the staged and synced two-tab `docs` session save. It boots the same system and USB disks, reopens Loom, checks the active `docs` tab and the other root tab, and captures `folder-tabs-powercut-restored.ppm`.
- This tests recovery after the save receipt. A power cut during the earlier write phase is covered by the session's previous-copy fallback in code but still needs fault injection. Loom's redo behavior remains an open gap.
