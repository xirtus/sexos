# Cufflinks Bluetooth, slice W1

**`crates/sex-bt`, `servers/cufflinks`, `servers/sexusb/src/bt.rs`, `crates/sex-pdx/src/bt_hci.rs`.** Native Bluetooth host stack (no BlueZ, no bridge): HCI codec, controller bring-up, classic inquiry and LE (extended) scanning with name/class/RSSI/AD parsing, L2CAP fixed channels and signaling, LE Secure Connections pairing (P-256 ECDH, AES-CMAC f4/f5/f6/g2 checked against the spec's sample data) and classic Secure Simple Pairing, Intel firmware load (Read Version legacy/TLV, Secure Send RSA/ECDSA, Intel Reset, DDC) for AX201 (8087:0026, `ibt-19-0-4`), AX210/Killer AX1675x (8087:0032, `ibt-0041-0041`) and AX211 (8087:0033, `ibt-0040-0041`). SexUSB drives USB HCI interfaces (0xE0/0x01/0x01) and bridges packets through `sex_pdx::bt_hci` (slot 1003, region 0x4402; firmware pack slot 1004 reserved).

Cufflinks builds (`build_cufflinks` stage) but is not started by BOOT yet: it needs a BOOT row and the region grants in the kernel. SexUSB probes the controller's version itself meanwhile (`[sexusb.bt.probe]`, `[sexusb.bt.event]`).

**Gate**: `scripts/bt_host_gate.sh` (CI host job, after the Wi-Fi gate). Firmware: `scripts/fetch_bt_firmware.sh` (SHA-256 pinned; the gate skips the real-image test when offline). Doc: `docs/handoff/CUFFLINKS_BLUETOOTH_2026_11_14.md`. Ledger: BRIDGE_LEDGER tables 2 and 3.
