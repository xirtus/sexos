# Quil untitled recovery

- Assigned eight bounded, collision-free recovery identities to unnamed tabs.
  Edited tabs up to 4 MiB use Quil's existing checked, staged draft write.
- Restored those drafts on launch, labeled the recovered tabs, and removed a
  draft after Save or confirmed discard. A Files grant or storage failure leaves
  the tab editable and reports that its recovery slot is unavailable.
- Added a focused slot identity test and an Editor QEMU power cut test that
  reboots the same disk and checks the recovery receipt and screenshots.
- Validation: focused host test and targeted Quil compile locally; full build
  and QEMU run through GitHub CI. No local target or disk image retained.
