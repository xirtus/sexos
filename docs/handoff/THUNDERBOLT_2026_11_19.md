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

## Next

- **T1b**: BOOT leases the NHI by class 0c0340 (BAR0 view to 0x3a000, a small DMA
  pool), raising force power at lease time since drivers never see config space;
  a Thunderbolt server (its own PD) runs `Manager` and logs `[tb.*]`.
- **T2**: the desktop prompt for security level user, remembered approvals by
  UUID, secure-level keys, and "tunnel up at route R" to the hot-plug code.
- **T3**: USB4 software connection manager for hosts whose firmware leaves it to us.
