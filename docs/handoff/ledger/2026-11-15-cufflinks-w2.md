# Cufflinks Bluetooth, boot start and slice W2

**Boot (W1b).** BOOT starts Cufflinks as domain 52 (Senses organ, key 4; limine.cfg module `servers/cufflinks`), pairs `sex_pdx::bt_hci` (slot 1003, region 0x4402) between SexUSB and Cufflinks, grants SexUSB a wake route to Cufflinks (slot 1005, `bt_hci::OP_LINK`), Cufflinks a Files route, and the firmware pack (slot 1004, the Wi-Fi pack grant made generic). 27 kernel lines, budget unchanged. Without a controller Cufflinks logs `[cufflinks.idle] controller=none` and sleeps in `sys_wait_self`. Gate: `scripts/cufflinks_idle_gate.py` on the boot gate's serial log (CI boot job).

**W2.** `crates/sex-bt`: `att` (ATT client codec), `gatt` (HID over GATT: service/characteristic/descriptor discovery, report map with Read Blob, report references, CCCD writes, boot-protocol fallback), `hid` (report-map parser, boot reports), `hidp` + `l2cap::Channels` (classic HID control/interrupt channels, boot protocol), `bonds` (LE LTK + IRK + identity address, classic link keys; CRC-checked stored form), SMP identity key distribution and `crypto::ah` (RPA resolution). Input goes Cufflinks -> `bt_hci` HID mailbox -> SexUSB (merged with USB keyboards/mice) -> Input. Bonds persist in `/system/bluetooth.bonds` through Files (`sex_pdx::bt_bonds`, op 0x2C9, Cufflinks only).

**Not yet.** Desktop pairing prompt (a bounded pairing window stands in: first two minutes after a controller is ready, only with no bonds; TODO in `servers/cufflinks/src/pairing.rs`), passkey entry, SDP/report protocol for classic HID, `ibt-*` files in the boot firmware pack.

**Gate**: `scripts/bt_host_gate.sh` (adds `att_gatt`, `hid_peers` and the `bt_bonds` store test). Doc: `docs/handoff/CUFFLINKS_BLUETOOTH_2026_11_14.md`.
