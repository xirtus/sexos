# Thunderbolt 4 / USB4 on SexOS (2026-11-19)

Owner: thread "Add Wi-Fi 6, Bluetooth audio and Thunderbolt" (plan:
`UNTETHERED_X17_PLAN_2026_11_19.md`, stages T1-T3). What appears behind a
tunnel (PCIe rescan below the root port, monitors, eGPUs) belongs to the hot-plug
thread; the interface between them is "tunnel up / gone at this route".

## How Intel's integrated controllers work

Ice Lake, Tiger Lake (the x17's 11800H: NHI 8086:9a1f/9a21), Alder Lake, Raptor
Lake and Meteor Lake carry the Thunderbolt controller in the CPU. The OS sees a
**Native Host Interface** (NHI, PCI class 0c0340): DMA rings, one per hop, in a
256 KiB BAR. Ring 0 carries **control frames**: 32-bit words sent big-endian with
a CRC-32C after them; the frame type travels in the descriptor's SOF/EOF fields.

On these PCs a **firmware connection manager** (ICM) inside the controller builds
every tunnel itself. The OS:

1. powers the firmware up (config word VS_CAP_22 "force power", then VS_CAP_9
   FW_READY; BAR0 FW_STS shows ICM_EN and NVM_AUTH_DONE);
2. starts ring 0 both ways and sends DRIVER_READY;
3. reads one dword of the root router's config space until it answers;
4. hears DEVICE_CONNECTED for every router (also those plugged in before boot),
   with its UUID, route (one byte per hop), link, Gen 3 speed and the security
   level the BIOS set;
5. at security level **user**, sends APPROVE_DEVICE for a device the person
   trusts; the firmware then tunnels its PCIe. Level **none** tunnels at once;
   **secure** needs a stored key (ADD_DEVICE_KEY / CHALLENGE_DEVICE, stage T2);
   **dponly** / **usbonly** never tunnel PCIe;
6. hears DEVICE_DISCONNECTED (for a dock, its children go too) and RTD3 vetoes.

DisplayPort and USB 3 tunnels need no approval: the firmware makes them as soon
as a device connects.

## Stage T1a: driver core (`crates/sex-thunderbolt`)

- `frame.rs`: CRC-32C, encode/decode (CRC checked for config/event frames, not
  for the firmware's own ICM events and responses, as Linux does), config READ
  addresses, routes.
- `ring.rs`: raw-mode rings, 10 slots, 256-byte buffers, producer word (TX, top
  half of +8) and consumer word (RX, +8), RX takes every frame type, reposting one
  slot per frame taken. Polled: no ring interrupts yet.
- `icm.rs`: messages (DRIVER_READY response, DEVICE_CONNECTED/DISCONNECTED,
  APPROVE_DEVICE, RTD3_VETO) and `Manager`: phases Firmware → DriverReady →
  RootRouter → Ready, one request in flight with Linux's timeouts (20 s driver
  ready, 10 s approve, 5 s others) and three retries, a device table, an approval
  queue, removal of a dock's subtree deepest first.
- No firmware CM (`Failure::NoFirmware`): the OS must be the connection manager
  itself (USB4 software CM, stage T3).

Written fresh from the USB4 / Thunderbolt 3 specifications; Linux's
`drivers/thunderbolt` (GPL-2.0) was read for register offsets and message layouts.

Tests: `crates/sex-thunderbolt/tests/icm.rs`, a model NHI that checks every sent
frame's CRC and descriptor, answers like the ICM and injects plug events
(`scripts/thunderbolt_host_gate.sh`, in CI).

## Stage T1b: lease and service

- BOOT's USB manifest claims Intel devices of class 0c0340 (`Lease::Thunderbolt`)
  for SexUSB at slot 29: a BAR0 view of the whole 256 KiB (the firmware status
  word sits at 0x39944, past the usual 64 KiB cap; `Kind::view_max`), the memory
  and bus-master command bits, and a 64 KiB DMA pool at slot 30 (region 0x4600).
- At lease time BOOT raises force power in VS_CAP_22 (DMA delay 0x22), since
  drivers never see config space, and logs `[kernel.pci.tb.force_power] fw_ready=`.
- SexUSB (`servers/sexusb/src/thunderbolt.rs`) runs `Manager` every loop pass and
  logs `[tb.nhi]`, `[tb.prepared]`, `[tb.phase]`, `[tb.ready] nvm= security=`,
  `[tb.device.connected] route= gbps= security= authorized=`, `[tb.device.waiting]`,
  `[tb.device.disconnected]`. USB4 is the USB family, and SexUSB is already its own
  domain; a separate Thunderbolt domain stays possible if it grows.
- PCIe tunnels at security level user are not approved yet (T2 adds the prompt);
  DisplayPort and USB tunnels work without approval.

ABI: new capability slots 29/30 in SexUSB and the region id 0x4600
(`sex_pdx::pci_command::SLOT_THUNDERBOLT*`); no new syscall.

x17 check: boot with a dock or USB-C monitor on the Thunderbolt port and read the
`[tb.*]` lines (`[tb.phase] Ready`, then one `[tb.device.connected]` per router).

## Stage T1c: sleep and wake

On the kernel's sleep notice (`sex_pdx::power::OP_DEVICE_SLEEP`), going down:

1. SexUSB stops ring 0 and posts DRV_UNLOADS (0x07) on the NHI's inbound mailbox
   (`sex_thunderbolt::mailbox`, Linux `icm_suspend`), waits up to 500 ms for the
   firmware, and forgets its devices;
2. it asks for D3hot (`sex_pdx::device::set_power`); BOOT first sends the link
   controller GO2SX through VS_CAP_19 and waits for VS_CAP_18 DONE
   (`sex_pcipm::nhi`, Linux `icl_nhi_suspend_noirq`), so a plug wakes the PC.

Coming up, D0 is asked for; BOOT raises force power again and waits for FW_READY,
and SexUSB starts a new manager (DRIVER_READY); the firmware reports what is still
plugged in. Logs: `[tb.sleep] unload= d3hot=`, `[pci.power.tb.go2sx] done= ms=`,
`[pci.power.tb.fw_ready] ready= ms=`, `[tb.wake] d0= fw_sts=`. Linux has no
GO2SX_EXIT; wake is force power alone.

## Next

- **T2**: the desktop prompt for security level user, remembered approvals by
  UUID, secure-level keys, and "tunnel up at route R" to the hot-plug code.
- **T3**: USB4 software connection manager for hosts whose firmware leaves it to us.
