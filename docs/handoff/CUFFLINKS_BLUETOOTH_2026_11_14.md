# Cufflinks: native Bluetooth, slices W1, W1b, W2, W3 and W2b (2026-11-14/15)

Cufflinks is SexOS's own Bluetooth stack: no BlueZ, no Linux bridge, no Tuxedo. W1
delivers the transport, the Intel firmware load, controller bring-up, discovery and
pairing. W1b starts it at boot. W2 adds keyboards and mice (classic HID and HID over
GATT), bonds kept on disk, and a bounded pairing policy in place of the desktop prompt.
W3 adds headphones: A2DP Source with our own SBC encoder, Bell's mix routed to the
headset, and ACL flow control. W2b adds the desktop: the Shell's Bluetooth panel
(Super+Shift+B) with scan, pair, forget and the numeric-comparison prompt, which
replaces the pairing window as the normal way to pair.
Plan row: `APP_PARITY_PLAN_2026_11_14.md` section W. W2 sections: 7 to 10 below; W3:
sections 11 to 14; W2b: sections 15 to 18.

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
| `sex_pdx::bt_audio::SLOT` (W3: Bell's mix for a headset, Bell + Cufflinks) | 1006, region id 0x4404, 256 KiB (4 KiB header, 64,512 stereo frames at 48 kHz) |
| `sex_pdx::bt_ui::SLOT` (W2b: the Bluetooth panel, Shell + Cufflinks) | 1007, region id 0x4405, 4 KiB |
| Cufflinks -> Shell wake (W2b) | slot 1008 (`bt_ui::NOTIFY_SLOT`), op 0x2CA (`bt_ui::OP_EVENT`), no reply |
| Shell -> Display panel lines (W2b) | op 0x2CB (`bt_ui::OP_PANEL`), Shell (domain 3) only |
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

## 9. Pairing prompt (W2): bounded window, now the fallback

W2 shipped without the desktop prompt; W2b (section 15) built it. The bounded rule W2
used instead stays as the fallback for a boot where no Shell is attached to the panel
region or Cufflinks's wake does not reach it (`servers/cufflinks/src/pairing.rs`):

- bonded devices always reconnect;
- a new keyboard/mouse (class of device major 0x05, or appearance 0x03C0..0x03C2) is
  connected and its numeric comparison accepted only inside a 120 s window after the
  controller is ready, and only when nothing is bonded yet (first setup);
- otherwise new devices are skipped (`[cufflinks.hid.skip] reason=no_prompt`) and a
  comparison is refused (`accepted=0 reason=no_prompt`).

With the prompt available (the normal desktop) nothing new is connected on its own:
scan results are only listed (`[cufflinks.hid.skip] reason=panel`).

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

Next: ~~the desktop prompt (W2b)~~ (done, section 15), passkey entry (most BLE keyboards ask for it: KeyboardOnly
IO capability; today `choose()` refuses it), SDP + report protocol for classic devices with
extra keys, a firmware pack slot for `ibt-*`, the Tatami Bluetooth page, then W3 (A2DP, sections 11 to 14).


## 11. W3: headphones (A2DP into Bell)

| Piece | Where | What |
|---|---|---|
| SBC encoder | `sex_bt::sbc` | Our own, integer only (the SexOS target is soft-float): the A2DP spec's analysis filterbank (prototype windows in Q24, cosine matrix in Q15, subband samples in Q8), scale factors, joint-stereo decisions, the spec's bit allocation (loudness and SNR), quantization, frame packing and CRC-8. Every mode, 4/8/12/16 blocks, 4/8 subbands. Cufflinks streams 48 kHz joint stereo, 16 blocks, 8 subbands, loudness, bitpool 53 (119-byte frames) |
| SDP | `sex_bt::sdp` | Data elements (bounded depth, element count, response size, continuation rounds), a Service Search Attribute client with continuation, `a2dp_sink()` (AVDTP PSM and versions from the record), and a small server answering peers with our A2DP Source record |
| AVDTP | `sex_bt::avdtp` | Signaling codec with start/continue/end fragments (bounded reassembly), SEP and capability parsing, SBC capability choice (48 kHz; joint stereo, 16 blocks, 8 subbands, loudness preferred; bitpool at most 53 within the sink's range); `Source` as initiator (DISCOVER, GET_CAPABILITIES, SET_CONFIGURATION, OPEN, START, SUSPEND, CLOSE) and as acceptor (a headset that configures us; DISCOVER, GET_(ALL_)CAPABILITIES, GET_CONFIGURATION, OPEN, START, SUSPEND, CLOSE, ABORT, DELAY_REPORT; RECONFIGURE and SECURITY_CONTROL refused, unknown signals general-rejected) |
| Media | `sex_bt::a2dp` | RTP (version 2, payload type 96, sequence, timestamp in samples) + SBC payload header, as many whole frames as the transport channel's MTU takes (5 at the default 672, 7 at 895) |
| L2CAP | `sex_bt::l2cap` | The peer's MTU option is kept per channel (raised to the 48-byte minimum), channels can be closed (`disconnect`) |
| Host | `sex_bt::host` | `connect_audio` (page, SSP, encrypt, SDP on PSM 1, AVDTP signaling then transport channel on the sink's PSM), `play` (PCM in, media packets out), `suspend_audio`, `resume_audio`, `close_audio`, `start_audio`, `audio_state`, `is_audio` (class of device Audio/Video with the Audio or Rendering service); notes `AudioSink`, `AudioReady`, `AudioStreaming`, `AudioSuspended`, `AudioClosed`, `AudioFailed`. A bonded headset may page us and set the stream up itself |
| ACL flow control | `sex_bt::host` | ACL packets leave only while the controller has buffers (Read Buffer Size; LE pool or one shared pool when it reports no LE buffers); Number Of Completed Packets gives them back per link, never more than the link has out; a link's buffers come back when it disconnects. Media is further held to `MEDIA_WINDOW` (6) packets queued or in the controller |
| Bell side | `servers/sexbell/src/sound.rs` | While Cufflinks marks the `bt_audio` sink on, each mixed period (all programs' streams and Bell's notes, after master and row volume) goes into the region and the card plays silence; when the sink goes off the speakers come back |
| Cufflinks side | `servers/cufflinks/src/audio.rs` | Turns the sink on at `AudioReady`, off when the stream or link goes; paces by its own 1 ms tick (100 ms lead, no burst after an underrun), drops the oldest frames when Bell is more than 0.75 s ahead, suspends after 3 s of silence and resumes when sound comes, plays a short two-note chime when a headset first streams |
| Region | `sex_pdx::bt_audio` | Bell writes frames (never over what Cufflinks has not taken; overflow counted), Cufflinks peeks and consumes; no routes, no new ops, both sides poll |

Security: audio is sent only on an encrypted link to a bonded device (`play` refuses
otherwise, and an Encryption Change to off stops it until encryption is back). AVDTP
channels to us need a bonded or encrypted link (like HID); data on them is ignored
until the link is encrypted. SDP to us is open to any link we let in (only bonded
devices may page us). Every length and count from the peer is checked; malformed SDP
fails the search, malformed AVDTP gets the spec's rejects, and random input never panics
(host test).

Pairing: a new headset found by scanning is connected only in the pairing window
(`pairing.rs`, same rule as keyboards: first setup, nothing bonded yet, 120 s);
otherwise `[cufflinks.audio.skip] reason=no_prompt` until the desktop prompt (W2b).

## 12. W3 numbers, kernel and boot log

- Kernel: 3 lines (BOOT wiring only, no audio code in ring 0): `init_bluetooth` takes
  Bell's domain and pairs the `bt_audio` region between Bell and Cufflinks; init passes
  Bell's id. Log: `[boot.bt.audio] bell=… cufflinks=52 slot=1006 bytes=262144 runtime_mint=0`.
- ABI: `sex_pdx::bt_audio` (slot 1006 in both, region 0x4404); `spec/abi_inputs.sha256`
  re-pinned for `kernel/src/boot_memory.rs` and `crates/sex-pdx/src/lib.rs`. No new
  syscalls, no new PDX operations.
- Boot log: `[bell.bt.route] region=1`, `[cufflinks.audio.route] ok=1 slot=1006` (both at
  start, checked by `scripts/cufflinks_idle_gate.py`); with a headset:
  `[cufflinks.audio.connect] addr=… name=… window=1`, `[cufflinks.audio.sink] handle=…
  psm=0x0019 avdtp=0x0103 a2dp=0x0103 features=…`, `[cufflinks.audio] addr=… rate=48000
  mode=JointStereo blocks=16 subbands=8 bitpool=53 frame_bytes=119 frames_per_packet=…`,
  `[cufflinks.audio.streaming]`, `[bell.bt.sink] on=1 sinks=1 dropped=0`,
  `[cufflinks.audio.idle] … suspend=1`, `[cufflinks.audio.suspended]`,
  `[cufflinks.audio.closed]`, `[cufflinks.audio.stopped] handle=… frames=… dropped=…`,
  `[cufflinks.audio.failed] code=…` (AVDTP/SDP error, or 0xF0 no sink, 0xF1 no usable
  configuration), `[cufflinks.audio.start] … initiator=1` (a bonded headset paged us,
  opened AVDTP and configured nothing for 2 s, so we set the stream up).

## 13. W3 tests (`bash scripts/bt_host_gate.sh`)

- `sbc`: frame header and length for the usual streams (`9C FD 35`, `9C BD 35`, 119
  bytes) and other modes, refused configurations, CRC-8 against an independent bitwise
  implementation and a damaged frame, and round trips through `tests/common/sbc_ref.rs`,
  a floating-point decoder written separately from the spec (it shares only the
  prototype tables): a sine at 48 kHz joint stereo comes back above 45 dB SNR, every
  mode/subbands/allocation/blocks combination above 40 dB, a three-tone-plus-noise signal
  above 25 dB, silence stays silent, full-scale square waves clip without bad frames.
- `a2dp_peers`: a simulated headset behind a scripted controller with four ACL buffers:
  page, SSP Just Works, encryption, SDP answered in two parts (continuation), DISCOVER
  (a video sink, a busy audio sink, a free one: the free one is picked), capabilities,
  SET_CONFIGURATION (checked byte for byte), OPEN, transport channel, START; a tone
  played into the host is decoded by the headset (above 30 dB SNR both channels; RTP
  sequence and timestamps checked); the controller holds its buffers and the host never
  sends more than four, then flow resumes; bogus completions give no credit; suspend,
  resume, encryption off and on, close; the headset reconnects by paging us with the
  stored key and sets the stream up itself (we answer as acceptor, send no commands of
  our own) and audio flows; strangers are refused; malformed AVDTP gets the right
  rejects and the stream keeps going; acceptor configuration checks; fragment round
  trip; L2CAP MTU option; SDP elements against a hand-written record, our server's
  continuation against our client; malformed SDP (deep nesting, overruns, wrong
  transaction, endless continuation, missing protocol list) fails cleanly; 20,000
  random packets through every parser never panic.
- The `sex_pdx::bt_audio` ring test, and the existing HID tests now with controllers
  that return ACL buffers (they would stall otherwise).

## 14. W3: not done, and the x17 receipt

Not done (next slices):

- AAC (needs FFmpeg as a library or our own encoder), mSBC/HFP (headset microphone),
  AVRCP (play/pause and volume buttons from the headset), delay reporting, content
  protection (SCMS-T), the tray item and the Tatami Bluetooth page.
- 44.1 kHz-only sinks: we stream 48 kHz only (A2DP requires sinks to support both, so
  this should not bite); a resampler would lift it.
- With no sound card Bell renders nothing, so there is no Bluetooth audio either: Bell
  is clocked by its card. A card-less Bell clock (render by the tick while the sink is
  on) is the fix.
- One headset streams at a time (the first ready one).
- ~~The desktop prompt (W2b) still gates new devices~~: done in W2b (section 15); a new
  headset is paired from the Bluetooth panel.
- No QEMU gate for audio: QEMU has no Bluetooth controller model.

x17 receipt (after section 10), with nothing bonded and Bluetooth headphones in
pairing mode before boot:

1. `[cufflinks.audio.connect] addr=… name="…" window=1`, `[cufflinks.pair] … paired=1`,
   `[cufflinks.bonds] saved=1`.
2. `[cufflinks.audio.sink] … psm=0x0019`, `[cufflinks.audio] … rate=48000 …`,
   `[cufflinks.audio.streaming]`, and the two-note chime in the headphones.
3. Play sound (Qupid or `beep`): it comes from the headphones, not the speakers;
   `[bell.bt.sink] on=1`. After 3 s of silence `[cufflinks.audio.suspended]`; sound
   again resumes it.
4. Turn the headphones off: `[cufflinks.disconnected]`, `[cufflinks.audio.stopped]`,
   `[bell.bt.sink] on=0`, sound back on the speakers. Turn them on again: they page us,
   `[cufflinks.audio] …` again without pairing. Send any `[cufflinks.audio.failed]` line.


## 15. W2b: the Bluetooth panel and pairing prompt

| Piece | Where | What |
|---|---|---|
| Protocol | `crates/sex-pdx/src/bt_ui.rs` | One 4 KiB region shared by the Shell and Cufflinks: Cufflinks writes a `Snapshot` (controller phase, scanning, up to 12 devices with name, kind and flags, the comparison waiting for the user, the last pairing outcome) under a sequence lock; the Shell writes `Command`s (scan on/off, pair, forget, confirm with a ticket) into an 8-entry ring; the Shell marks itself attached and says whether its panel is open. Cufflinks wakes the Shell with `OP_EVENT` (no reply). Also the panel's text model (`Ui`: lines, keys, clicks, dialog), its wire to Display (`OP_PANEL`, `Lines` in 16-byte chunks, `Assembler`) and the layout both sides place it with (`Panel`) |
| Shell | `servers/silk-shell/src/bt_panel.rs` + 6 hook lines in `main.rs`, Super+Shift+B in `wm.rs` | Maps the region at start and attaches; reads the snapshot after a wake or while the panel shows; sends the lines to Display only when they change; keys (Up/Down, Enter, S, Delete, Y/N, Esc) and clicks go to `bt_ui::Ui`; a comparison opens the panel by itself. While the panel shows it takes every key press that is not a Super chord (releases pass on) |
| Display | `servers/sexdisplay/src/bt_panel.rs` + 3 hook lines in `main.rs` | Assembles `OP_PANEL` from the Shell only and draws the panel over the desktop under the bar at the right, after the tray menu and before the lock surface (which covers it) |
| Cufflinks | `servers/cufflinks/src/ui.rs`, `main.rs`, `pairing.rs` | Publishes phase (no controller, starting, on, failed), devices from scans and bonds (a bonded LE device is listed under its bond address), connected/in-use/pairing/paired marks; takes commands only while a controller is up; Pair connects as a keyboard/mouse (`connect_hid`) or headset (`connect_audio`) by kind, unsupported kinds and devices not seen are refused with a reason; Forget is `Host::forget` |
| Host | `sex_bt::host` | `Note::ConnectFailed { addr, status }` when a connection we asked for does not come up (page timeout), so a Pair fails at once instead of after 40 s; the device can be asked again |
| Kernel | `boot_memory::init_bluetooth` (4 lines), `init.rs` (call takes the Shell's id) | Pairs the `bt_ui` region between the Shell and Cufflinks and grants Cufflinks slot 1008 -> Shell. Log: `[boot.bt.ui] shell=3 cufflinks=52 slot=1007 notify=1008 runtime_mint=0` |

Why a region and one wake route, not two routes: Cufflinks polls the region every tick
anyway while a controller is up, and with no controller there is nothing to command, so
there is no Shell -> Cufflinks route and an idle Cufflinks is never woken by the
desktop (`cufflinks_idle_gate.py` keeps checking that). The Shell sleeps in its listen,
so it needs the wake; Cufflinks sends list changes only while the panel is open, and
comparisons and outcomes always.

The panel, top to bottom: "Bluetooth" with On / On, scanning / Off / Starting / Failed /
Unavailable; a status line ("No Bluetooth controller", "Paired with ...", "Pairing with
... failed", "... is not in range"); "Scan for devices" / "Stop scanning"; devices,
paired and connected first ("Silk Keyboard    keyboard, in use"); a key hint. The
dialog: "Pair with <name>?", "Does it show 123456?", "Yes, the numbers match", "No".

## 16. W2b pairing policy

`pairing.rs` (pure, host-tested as compiled):

- Prompt available = a Shell attached to the region and Cufflinks's last wake reached it.
- With the prompt: a scanned new device is never connected on its own; the user picks
  Pair. Every numeric comparison is asked in the panel, whether the user started the
  pairing or a device asked us; only Yes accepts. No answer within 30 s refuses
  (`reason=timeout`); one question at a time (a second device asking meanwhile is
  refused, `reason=busy`). Just Works pairing (a device with no screen) needs no
  question: the user's Pair is the consent, and Cufflinks only starts it on a Pair.
- Without the prompt: the W2 first-setup window (120 s, nothing bonded) as before.
  Once anything is bonded, strangers are never auto-accepted, prompt or not.
- A Pair that does not connect within 40 s (or fails to page) is reported failed.
- Bonded devices come back as before.

Boot-log lines added: `[cufflinks.ui.route] ok=1 slot=1007`, `[cufflinks.ui.wake]
event=… ok=0 status=…`, `[cufflinks.ui.scan] on=1|0`, `[cufflinks.ui.pair] addr=…
name=… audio=… user=1` (or `found=0`, `supported=0`, `timeout=1`), `[cufflinks.ui.forget]
addr=…`, `[cufflinks.pair] addr=… compare=… asked=1 ticket=… requested=…`, `accepted=user`,
`accepted=0 reason=user|timeout|busy`, `[cufflinks.pair] ticket=… stale=1`,
`[cufflinks.connect.failed] addr=… status=…`, `[cufflinks.pairing] bonds=… window_s=…
prompt=…`; Shell `[shell.bt.route] ok=1 slot=1007`, `[shell.bt.panel] open=… phase=…
lines=… selected=… dialog=… status="…"`, `[shell.bt.command] Pair([..]) sent=1`;
Display `[display.bt.panel] lines=… selected=…`, `[display.bt.panel.draw] lines=…
panel=WxH+X+Y`.

## 17. W2b tests

- `bash scripts/bt_host_gate.sh` adds the `sex_pdx::bt_ui` tests (device kinds from class
  and appearance; names made printable and bounded; table updates, eviction that never
  drops a paired device, snapshot round trip and refusal of damaged snapshots; commands;
  the region: commands written before Cufflinks starts are dropped, full ring, torn
  writes never read, a runaway writer dropped; the panel through the Display wire:
  "No Bluetooth controller", "not running", list order, Enter/Delete/Down/S/Esc, clicks
  on the rows drawn, comparison dialog opening by itself, answered once, closed when
  Cufflinks gives up, outcomes) and `tests/cufflinks_pairing.rs` (prompt means the user
  decides; without a prompt only first setup pairs; bonded means no stranger; answer
  timeout and stale tickets; requests bounded and expiring).
- `cargo test -p sex-bt`: `hid_peers` `a_page_that_fails_is_noted_and_can_be_retried`.
- `tests/wm_layouts.rs`: Super+Shift+B is Bluetooth, Super+B still the wallpaper.
- `scripts/cufflinks_idle_gate.py` (boot log): the panel region is paired and both sides
  find it; Cufflinks still sleeps.
- `scripts/bt_panel_gate.py` (QEMU, in CI after the tray gate; passed locally: a
  420x102 panel at 852,34 on 1280x800, 42,840 pixels changed): Super+Shift+B opens
  the panel, Display draws it, it says "No Bluetooth controller" (QEMU has none), Escape
  closes it. The comparison dialog and pairing need a controller: host tests plus the
  x17 receipt below.

## 18. W2b: not done, and the x17 receipt

Not done: a tray icon or quick-settings row for Bluetooth (the panel opens from
Super+Shift+B only; a bar icon needs Display's tray drawing), a Tatami page, a
palette/launcher entry, device names for bonds not seen this boot (bonds do not store
names: such a device shows its address until it is seen), passkey entry (still refused),
renaming and per-device "connect/disconnect" separate from Pair, and battery levels.

x17 receipt (after section 14):

1. Boot: `[boot.bt.ui] shell=3 cufflinks=52 …`, `[cufflinks.ui.route] ok=1`,
   `[shell.bt.route] ok=1`, then `[cufflinks.pairing] bonds=… prompt=1`.
2. Super+Shift+B: the panel shows "Bluetooth  On" (or "On, scanning") and the devices
   around. Put a keyboard in pairing mode, press S (scan), pick it, Enter:
   `[shell.bt.command] Pair(…) sent=1`, `[cufflinks.ui.pair] … user=1`.
3. If the keyboard shows a number: the panel asks "Does it show 123456?"; Y:
   `[cufflinks.pair] … asked=1 …`, `… accepted=user`, `… paired=1`, "Paired with …".
   Type on it. (A keyboard that asks for a passkey still fails: not done.)
4. Headphones the same way (no number: Just Works), then Delete on the row forgets them:
   `[cufflinks.ui.forget]`, `[cufflinks.bonds] saved=1 count=…`.
5. A device that tries to pair by itself while the panel is closed: the panel opens with
   the question; leave it 30 s: `accepted=0 reason=timeout`.

