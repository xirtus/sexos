# Cufflinks: native Bluetooth, slice W1 (2026-11-14)

Cufflinks is SexOS's own Bluetooth stack: no BlueZ, no Linux bridge, no Tuxedo. W1
delivers the transport, the Intel firmware load, controller bring-up, discovery and
pairing. Plan row: `APP_PARITY_PLAN_2026_11_14.md` section W.

## 1. What W1 delivers

| Piece | Where | What |
|---|---|---|
| Host stack core | `crates/sex-bt` (no_std + alloc, `GPL-2.0-or-later OR MIT`) | `hci` codec (commands, events, ACL), `controller` bring-up state machine and scan commands, `report` (inquiry results with/without RSSI, extended inquiry results, LE and LE extended advertising reports, AD/EIR structures), `l2cap` (basic frames, fragmentation/reassembly, signaling: LE connection parameter update, information/echo, refusing dynamic channels), `smp` + `crypto` (LE Secure Connections: P-256 ECDH on `p256`, AES-CMAC on `aes`, f4/f5/f6/g2; numeric comparison and Just Works, both roles), `ssp` (classic Secure Simple Pairing event flow, link keys), `intel` (USB part table, Read Version legacy + TLV, firmware names, Secure Send plan, reset, DDC, loader state machine), `pack` (firmware pack lookup), `host` (all composed: packets in, packets out, notes for the log) |
| USB transport | `servers/sexusb/src/bt.rs` + hooks in `usb.rs`/`main.rs` | Finds interface class 0xE0/0x01/0x01, configures interrupt IN + bulk IN/OUT, HCI commands on the control endpoint (bmRequestType 0x20, bRequest 0), events split from the interrupt stream by length, ACL on bulk, Intel bootloader Secure Send on bulk OUT |
| Ring protocol | `crates/sex-pdx/src/bt_hci.rs` | Two single-producer rings of typed HCI packets (`01` command, `02` ACL, `04` event, `81` command on bulk OUT) |
| Server | `servers/cufflinks` | Intel firmware load, bring-up, scan, pairing, boot-log lines |
| Firmware | `scripts/fetch_bt_firmware.sh` | linux-firmware at the Wi-Fi commit, SHA-256 pinned |
| Gate | `scripts/bt_host_gate.sh` (CI host job) | all of the tests below |

Not booted yet. Cufflinks builds (workspace member, `build_cufflinks` stage, binary in
`iso_root/servers/cufflinks`) but BOOT does not start it: that needs a BOOT row / PD for
Cufflinks and the region grants in kernel `boot_memory.rs` (pair `bt_hci::SLOT` between
SexUSB and Cufflinks, as `usb_net` is paired with SexNet; give Cufflinks the firmware
pack at `FIRMWARE_SLOT`). `kernel/` was off limits for this slice. Until then SexUSB
drives the controller anyway, asks it for its version once and logs the first events, so
the x17 shows the transport working.

## 2. Numbers

| What | Value |
|---|---|
| `sex_pdx::bt_hci::SLOT` (shared ring, SexUSB + Cufflinks) | 1003 |
| `sex_pdx::bt_hci::REGION_ID` | 0x4402, 128 KiB (4 KiB header, 2 x 30 slots of 2 KiB) |
| `sex_pdx::bt_hci::FIRMWARE_SLOT` (read-only pack for Cufflinks) | 1004, region id 0x4403 |
| New syscalls / PDX ops | none |
| Firmware files | `ibt-19-0-4.{sfi,ddc}` (AX201), `ibt-0041-0041.{sfi,ddc}` (AX210, x17 R1), `ibt-0040-0041.{sfi,ddc}` (AX211) |

USB ids live in `sex_bt::intel::PARTS` (sex-hwdb covers PCI only): 8087:0025, 0026 (AX201),
0029 (AX200), 0032 (AX210 / Killer AX1675x), 0033 (AX211 / AX1675i), 0aaa (9560).
Names: legacy parts use `ibt-<hw_variant>-<hw_revision>-<fw_revision>`; TLV parts use
the CNVi/CNVR top type and step, packed and byte-swapped (`0x410` step 0 -> `0041`).

## 3. Boot-log lines

SexUSB (today, without Cufflinks running):

- `[sexusb.bt.route] region=0|1`
- `[sexusb.bt] vendor=0x8087 product=0x0032 slot=… iface=0 intr_ep=0x81 bulk_in=0x82 bulk_out=0x02 route=0`
- `[sexusb.bt.probe] opcode=0xfc05 sent=1` (Intel Read Version) or `opcode=0x1001` (others)
- `[sexusb.bt.event] code=0x0e length=… head=…` for the first four events
- `[sexusb.bt.error] …`, `[sexusb.bt.command.fail] …`, `[sexusb.bt.gone] slot=…`

Cufflinks (once booted):

- `[cufflinks.start]`, `[cufflinks.transport] ok=1 slot=1003` or `missing=bt_hci slot=1003 region=0x4402`
- `[cufflinks.controller] vendor=0x8087 product=0x0032 part="Intel Wi-Fi 6E AX210 / Killer AX1675x (TyP)"`
- `[cufflinks.firmware] variant=0x17 bootloader=1 file=ibt-0041-0041`
- `[cufflinks.firmware] missing=ibt-0041-0041.sfi pack=0` (no pack granted) or `file=… bytes=… ddc=1`
- `[cufflinks.firmware] state=operational loaded=1 boot=0x… fragments=…` (or `loaded=0` when already running)
- `[cufflinks.ready] addr=xx:xx:… hci=… lmp_subversion=… manufacturer=2 le=1 ssp=1 ext_scan=1`
- `[cufflinks.device] addr=… type=le|classic name="…" rssi=-48 class=0x… appearance=0x…`
- `[cufflinks.connected]`, `[cufflinks.pair] addr=… compare=123456 accepted=auto`, `[cufflinks.pair] … paired=1`, `[cufflinks.encrypted]`, `[cufflinks.refused]`, `[cufflinks.failed]`

## 4. Tests (`bash scripts/bt_host_gate.sh`)

- `crypto`: RFC 4493 AES-CMAC vectors; Core spec sample data for f4, f5, f6, g2; the spec's P-256 debug key; ECDH agreement; off-curve keys refused.
- `hci_goldens`: byte-exact commands (reset, event masks, inquiry, legacy and extended LE scan), events (Command Complete/Status, User Confirmation, LE Connection Complete, LTK Request, Intel vendor), ACL/L2CAP framing, fragmentation, signaling replies, report layouts (EIR, inquiry-with-RSSI arrays, LE reports, truncation).
- `intel_firmware`: synthetic .sfi through a scripted bootloader that checks every command: exact (opcode, bulk, fragment type, length) sequence for RSA and ECDSA headers, 252-byte splitting, boot address from Write Boot Params, Intel Reset, DDC records; already-operational skips; legacy AX201 naming (`ibt-19-0-4`), AX211 naming; damaged files and failed download results. With fetched firmware: the real `ibt-0041-0041.sfi` (both headers), `ibt-19-0-4.sfi` and the .ddc files parse.
- `end_to_end`: scripted controller with an LE peer running `sex_bt::smp` as responder: bring-up sequence, scan finds "Silk Keyboard" (classic EIR) and "Cuff Mouse" (LE extended report) by name and RSSI, LE connect, peer's Security Request, LE SC numeric comparison (both sides show the same number), LTK agreement, encryption; Just Works with a headless peer; user says no; classic SSP flow and stored link keys.
- `sex_pdx::bt_hci` ring test.

## 5. x17 R1 receipt checklist (for Andreas)

The x17's Killer AX1675x Bluetooth is USB 8087:0032 on an internal port.

1. Boot log has `[sexusb.bt] vendor=0x8087 product=0x0032 … intr_ep=0x81 bulk_in=0x82 bulk_out=0x02`. If it is missing, look for `[sexusb.device] … vendor=0x8087` and send its line (and whether Windows/Linux shows Bluetooth enabled in the BIOS).
2. `[sexusb.bt.probe] opcode=0xfc05 sent=1` and one `[sexusb.bt.event] code=0x0e …` line: the controller answered Intel Read Version. Send the `head=` bytes: they show bootloader vs operational (after Windows/Linux warm boots the firmware may still be loaded).
3. No `[sexusb.bt.error]` loops, and USB keyboard/mouse still work.
4. Once Cufflinks is booted (next slice): `[cufflinks.firmware] … loaded=1`, then `[cufflinks.ready] addr=…` matching the address Windows shows, then `[cufflinks.device]` lines for a phone in pairing mode and an LE mouse/keyboard nearby.

## 6. Next

- **W1b (kernel owner)**: BOOT row for Cufflinks, pair `bt_hci::SLOT` with SexUSB, grant the firmware pack (put the `ibt-*` files in the Wi-Fi pack or a `firmware/bt.pack` built like `make_wifi_firmware_pack.py`; `sex_bt::pack` reads either magic). Entropy grant instead of RDRAND. A QEMU gate needs a USB Bluetooth device model (QEMU's `usb-bt-dongle` was removed upstream), so the gate stays host-side plus x17 receipts.
- **W2 (HID and BLE GATT)**: L2CAP dynamic channels (PSM 0x11/0x13 HID control/interrupt, credit-based LE channels), SDP for classic HID, ATT client + GATT discovery, HID over GATT (HOGP) reports into `servers/sexinput` (a new `sex_pdx` op or the USB HID report path), bonds stored in the settings store, LE address resolution (IRK) and key distribution, passkey entry, the desktop pairing prompt (replace `accepted=auto`), reconnect to bonded devices, a Tatami/SilkBar Bluetooth page.
- **W3 (A2DP into Bell)**: AVDTP signaling and media channels, SBC encoder/decoder (host goldens), AAC through FFmpeg, AVRCP for play/pause, an audio sink path into `servers/sexbell`'s mixer, HFP later; ACL flow control by Number of Completed Packets (W1 sends one command at a time and does not count ACL credits).
