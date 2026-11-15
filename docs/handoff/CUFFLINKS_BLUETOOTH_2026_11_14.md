# Cufflinks: native Bluetooth, slices W1, W1b and W2 (2026-11-14/15)

Cufflinks is SexOS's own Bluetooth stack: no BlueZ, no Linux bridge, no Tuxedo. W1
delivers the transport, the Intel firmware load, controller bring-up, discovery and
pairing. W1b starts it at boot. W2 adds keyboards and mice (classic HID and HID over
GATT), bonds kept on disk, and a bounded pairing policy in place of the desktop prompt.
Plan row: `APP_PARITY_PLAN_2026_11_14.md` section W. W2 sections: 7 to 10 below.

## 1. What W1 delivers

| Piece | Where | What |
|---|---|---|
| Host stack core | `crates/sex-bt` (no_std + alloc, `GPL-2.0-or-later OR MIT`) | `hci` codec (commands, events, ACL), `controller` bring-up state machine and scan commands, `report` (inquiry results with/without RSSI, extended inquiry results, LE and LE extended advertising reports, AD/EIR structures), `l2cap` (basic frames, fragmentation/reassembly, signaling: LE connection parameter update, information/echo, refusing dynamic channels), `smp` + `crypto` (LE Secure Connections: P-256 ECDH on `p256`, AES-CMAC on `aes`, f4/f5/f6/g2; numeric comparison and Just Works, both roles), `ssp` (classic Secure Simple Pairing event flow, link keys), `intel` (USB part table, Read Version legacy + TLV, firmware names, Secure Send plan, reset, DDC, loader state machine), `pack` (firmware pack lookup), `host` (all composed: packets in, packets out, notes for the log) |
| USB transport | `servers/sexusb/src/bt.rs` + hooks in `usb.rs`/`main.rs` | Finds interface class 0xE0/0x01/0x01, configures interrupt IN + bulk IN/OUT, HCI commands on the control endpoint (bmRequestType 0x20, bRequest 0), events split from the interrupt stream by length, ACL on bulk, Intel bootloader Secure Send on bulk OUT |
| Ring protocol | `crates/sex-pdx/src/bt_hci.rs` | Two single-producer rings of typed HCI packets (`01` command, `02` ACL, `04` event, `81` command on bulk OUT) |
| Server | `servers/cufflinks` | Intel firmware load, bring-up, scan, pairing, boot-log lines |
| Firmware | `scripts/fetch_bt_firmware.sh` | linux-firmware at the Wi-Fi commit, SHA-256 pinned |
| Gate | `scripts/bt_host_gate.sh` (CI host job) | all of the tests below |

Booted since W1b (section 7). Before W1b SexUSB drove the controller alone, asked it for
its version once and logged the first events; it still does that in a build without
Cufflinks (`[sexusb.bt.route] region=0`).

## 2. Numbers

| What | Value |
|---|---|
| `sex_pdx::bt_hci::SLOT` (shared ring, SexUSB + Cufflinks) | 1003 |
| `sex_pdx::bt_hci::REGION_ID` | 0x4402, 128 KiB (4 KiB header, 2 x 30 slots of 2 KiB) |
| `sex_pdx::bt_hci::FIRMWARE_SLOT` (read-only pack for Cufflinks) | 1004, region id 0x4403 |
| Cufflinks domain / key | 52 (`bt_hci::CUFFLINKS_PD`), Senses key 4 |
| SexUSB -> Cufflinks route | slot 1005 (`bt_hci::NOTIFY_SLOT`), op 0x2C8 (`OP_LINK`) |
| Cufflinks -> Files (bonds) | `SLOT_STORAGE`, op 0x2C9 (`sex_pdx::bt_bonds`), `/system/bluetooth.bonds` |
| HID mailbox | `bt_hci` header bytes 512..1024: 32 records of 16 bytes (Cufflinks -> SexUSB) |
| New syscalls | none |
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

## 6. Next (as planned after W1; W1b and W2 status in sections 7 to 10)

- **W1b (kernel owner)**: BOOT row for Cufflinks, pair `bt_hci::SLOT` with SexUSB, grant the firmware pack (put the `ibt-*` files in the Wi-Fi pack or a `firmware/bt.pack` built like `make_wifi_firmware_pack.py`; `sex_bt::pack` reads either magic). Entropy grant instead of RDRAND. A QEMU gate needs a USB Bluetooth device model (QEMU's `usb-bt-dongle` was removed upstream), so the gate stays host-side plus x17 receipts.
- **W2 (HID and BLE GATT)**: L2CAP dynamic channels (PSM 0x11/0x13 HID control/interrupt, credit-based LE channels), SDP for classic HID, ATT client + GATT discovery, HID over GATT (HOGP) reports into `servers/sexinput` (a new `sex_pdx` op or the USB HID report path), bonds stored in the settings store, LE address resolution (IRK) and key distribution, passkey entry, the desktop pairing prompt (replace `accepted=auto`), reconnect to bonded devices, a Tatami/SilkBar Bluetooth page.
- **W3 (A2DP into Bell)**: AVDTP signaling and media channels, SBC encoder/decoder (host goldens), AAC through FFmpeg, AVRCP for play/pause, an audio sink path into `servers/sexbell`'s mixer, HFP later; ACL flow control by Number of Completed Packets (W1 sends one command at a time and does not count ACL credits).

## 7. W1b: boot start

- Kernel (27 lines, guardrail budget unchanged at 22,700; kernel 20,199 lines): `init.rs`
  spawns `servers/cufflinks` as domain 52 with Senses key 4, like Denim; `boot_memory::
  init_bluetooth` pairs the `bt_hci` region (SexUSB writable, Cufflinks writable), grants
  SexUSB slot 1005 -> Cufflinks and Cufflinks `SLOT_STORAGE` -> Files, and grants the
  firmware pack (the old `wifi_firmware` grant is now `firmware_pack(pd, slot, region)`).
  Log: `[kernel.spawn.cufflinks] id=52 key=4 organ=senses`, `[boot.bt.hci] usb=5
  cufflinks=52 files=11 slot=1003 bytes=131072 runtime_mint=0`.
- `limine.cfg`: `boot:///servers/cufflinks` in all four entries, after Denim.
- Idle: with no controller Cufflinks logs `[cufflinks.idle] controller=none` once and
  blocks in `sys_wait_self`. SexUSB sends `OP_LINK` when a controller comes or goes
  (`[sexusb.bt.notify] up=1 sent=1`), which wakes it; it touches Files only once a
  controller is ready, so a machine without Bluetooth boots exactly as before.
- Gate: `scripts/cufflinks_idle_gate.py --log serial.log` (CI boot job, after the main
  boot gate): spawn, region pairing, `[cufflinks.idle]`, no panic or restart loop, and the
  scheduler's per-second active masks after idle may name domain 52 at most once. QEMU has
  no Bluetooth controller model (`usb-bt-dongle` is gone), so the controller path stays
  host-tested plus x17 receipts.
- Firmware: BOOT grants `firmware/wifi.pack`, but the pack does not carry the `ibt-*`
  files yet: the installed boot partition (32 MiB, `make_boot_image.py`) has about 1 MB
  free and the three Intel BT images are 2.2 MB. Until a bigger boot slot or a pack of only
  the x17's `ibt-0041-0041.*` (0.7 MB) lands, an x17 that cold-boots into SexOS logs
  `[cufflinks.firmware] missing=ibt-0041-0041.sfi pack=1`; after a warm boot from
  Windows/Linux the firmware is already running and Cufflinks goes on (`loaded=0`).

## 8. W2: keyboards and mice

| Piece | Where | What |
|---|---|---|
| ATT client | `sex_bt::att` | MTU, Read By Group Type, Read By Type, Find Information, Read/Read Blob, Write/Write Command, notifications/indications; our own (empty) server answers peers' requests |
| HID over GATT | `sex_bt::gatt::Hogp` | Exchange MTU, find service 0x1812, its characteristics and their descriptors (CCCD, Report Reference), read the Report Map (Read Blob while full), read each input report's reference, enable notifications; boot protocol (Protocol Mode 0, 0x2A22/0x2A33) when the map has no keyboard/mouse; starts only on an encrypted link |
| Report maps | `sex_bt::hid` | own parser (not sexusb's GPL-3 one): modifiers, key array, buttons, relative X/Y, wheel, per report id; boot keyboard/mouse reports; rollover ignored |
| Classic HID | `sex_bt::hidp`, `l2cap::Channels` | basic-mode channels (connect, configure both ways, disconnect); host opens PSM 0x11 then 0x13 after authentication + encryption, sends SET_PROTOCOL boot; a bonded device may page us (Connection Request accepted only for bonded addresses) and open them itself; virtual cable unplug forgets the bond |
| Bonds | `sex_bt::bonds`, SMP key distribution | LE: LTK (SC: EDIV/Rand 0), IRK + identity address (we ask the responder for its identity key; RPA resolution with `crypto::ah`, spec sample checked); classic: link key; bounded to 32; stored form `CUFB` v1 + CRC-32 |
| Host | `sex_bt::host` | `connect_hid`, reconnect of bonded LE devices when they advertise (any address the IRK resolves), stored-LTK encryption (pair again if the device forgot us), notes `HidReady`, `Input`, `HidFailed`, `BondsChanged` |
| Input path | `bt_hci` HID mailbox, `servers/sexusb/src/bt.rs` | Cufflinks pushes `HidInput::{Keys, Pointer, Gone}`; SexUSB merges Bluetooth keyboards' held keys with USB ones in `send_keys` and sends motion with `send_relative`, so Input still has one producer (PD 5 task 5) and its sender check stays; SexUSB polls every tick while HID links are up |
| Bond storage | `sex_pdx::bt_bonds`, `servers/sexfiles/src/bt_bonds.rs`, `servers/cufflinks/src/store.rs` | Files keeps `/system/bluetooth.bonds` for Cufflinks only (caller checked by domain); PUT chunks then SAVE, LOAD then GET. Keys are stored in the clear, like any desktop Bluetooth store. Shibari was not used: it is RAM only. The settings store was not used: only Shell/Settings may write it and it holds numbers |

Boot-log lines added: `[cufflinks.bonds] loaded=… le=… classic=…`, `[cufflinks.bonds]
saved=1 count=… bytes=…`, `[files.bt.bonds.save] caller=52 …`, `[cufflinks.pairing]
bonds=… window_s=…`, `[cufflinks.hid.connect] addr=… name=… window=1`,
`[cufflinks.hid.skip] addr=… reason=no_prompt`, `[cufflinks.hid] addr=… device=… le=…
keyboard=… pointer=… boot=…`, `[cufflinks.hid.failed] …`, `[cufflinks.hid.dropped]`.

## 9. Pairing prompt: not done (TODO stays)

The desktop prompt is not built: it needs a `sex_pdx` prompt protocol, a Shell module with
a ~10-line hook (DAILY_DRIVER_NEXT_100_SLICES.md section 2) and two more BOOT routes
(Cufflinks -> Shell, Shell -> Cufflinks), which did not fit this slice. Auto-accept is
now bounded instead (`servers/cufflinks/src/pairing.rs`, TODO W2b):

- bonded devices always reconnect;
- a new keyboard/mouse (class of device major 0x05, or appearance 0x03C0..0x03C2) is
  connected and its numeric comparison accepted only inside a 120 s window after the
  controller is ready, and only when nothing is bonded yet (first setup);
- otherwise new devices are skipped (`[cufflinks.hid.skip] reason=no_prompt`) and a
  comparison is refused (`accepted=0 reason=no_prompt`).

## 10. Tests and receipts (W2)

- `bash scripts/bt_host_gate.sh` now also runs `att_gatt` (ATT request/response goldens,
  report-map decoding, `ah()` spec sample, bond store round trip, damage, bound) and
  `hid_peers`: a simulated BLE keyboard (Just Works pairing, identity distribution, GATT
  database with a 67-byte report map read by blobs over a 23-byte MTU, refusals before
  encryption, notifications to key events, bonds through the stored form, reconnect from
  a new resolvable private address with the stored LTK and no pairing); a simulated
  classic mouse (page, SSP, encryption, channels, boot protocol, motion; then it pages us
  and reconnects with the stored link key); unknown LE devices and strangers paging us are
  not connected. Plus the `bt_hci` HID mailbox and `bt_bonds` store tests.
- `python3 scripts/cufflinks_idle_gate.py --log …` (boot) and its unit test.
- x17 receipt (after section 5): with nothing bonded, put a Bluetooth keyboard or mouse
  in pairing mode before boot; expect `[cufflinks.hid.connect]`, `[cufflinks.pair] …
  paired=1`, `[cufflinks.bonds] saved=1`, `[cufflinks.hid] … keyboard=1`, and typing in a
  window. Reboot: `[cufflinks.bonds] loaded=1`, then the device reconnects on its own.

Next: the desktop prompt (W2b), passkey entry (most BLE keyboards ask for it: KeyboardOnly
IO capability; today `choose()` refuses it), SDP + report protocol for classic devices with
extra keys, a firmware pack slot for `ibt-*`, the Tatami Bluetooth page, then W3 (A2DP).

