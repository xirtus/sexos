# Real-PC drivers: wired network, HD Audio, SATA (2026-11-09)

Workstream D of `LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md`. Goal: a desktop or laptop
from the last ten years gets network, sound and its SATA disk without virtio. Each
controller is a BOOT lease like the existing NIC and virtio leases: one BAR view, the
memory and bus-master command bits, and a DMA pool. Drivers never name a physical
address; they poll with bounded waits.

## What BOOT leases now

| Controller | PCI match | Service and slot | BAR | DMA pool |
|---|---|---|---|---|
| Intel e1000 (82540), e1000e (82574, I217/I218/I219) | product ids in `sex_pdx::pci_command::Identity::model` | SexNet, `SLOT_NIC` 19 | BAR0 | region 2, 256 KiB |
| Intel igb (82576, I210/I211, I350), igc (I225/I226) | product ids | SexNet, 19 | BAR0 (64 KiB prefix) | region 2 |
| Realtek r8169 (RTL8111/8168/8101), r8125 (RTL8125/8126) | product ids | SexNet, 19 | BAR2, else BAR1 | region 2 |
| Intel HD Audio (any vendor: Intel, AMD, NVIDIA, VIA) | class 04:03 | Bell, `SLOT_SOUND` 20 | BAR0 | region 0x4300, 256 KiB |
| AHCI SATA | class 01:06:01 | drive service, `SLOT_AHCI_HOST` 21 | BAR5 (ABAR) | region 3, 512 KiB (the drive pool) |

Kernel: `kernel/src/nic_boot.rs` (models, BAR choice, register prefix),
`kernel/src/device_lease.rs` (class leases), `kernel/src/devmgr.rs::class_lease`,
`kernel/src/boot_memory.rs::{class_bar,class_command,class_dma}`.

### ABI changes (named; `abi_version_hash` recomputed)

- `sex_pdx::pci_command::Model` gains `I219, I82576, I210, I350, I225, Rtl8168, Rtl8125`,
  plus `Family`, `Model::family/advanced/registers_end`.
- `sex_pdx::pci_command::{HDA_DMA_REGION_ID, HDA_DMA_BYTES, AHCI_DMA_REGION_ID, AHCI_DMA_BYTES, SLOT_AHCI_HOST}`.
- Kernel `PciCapData` gains `class: u32`; the command syscall checks a class lease's
  register prefix and DMA pool by its class.
- Bell's `[bell.sound.ready]` record now starts with `card=virtio|hda`.

## Sound: Intel HD Audio in Bell

`servers/sexbell/src/hda.rs`. Controller reset, CORB/RIRB command rings (256 entries),
codec discovery from STATESTS, then for each audio function group the output pins:
speaker first, then headphones, then line out. HDMI and DisplayPort pins are skipped
(that sound belongs to the monitor). A depth-first walk through mixers and selectors finds
a converter; every node on the path is powered, unmuted at 0 dB and selected, and the pin
gets output enable (plus headphone amp, plus EAPD where offered). Every other analog jack
that reaches the same converter plays too.

One output stream runs from boot to shutdown: 16 periods of 8 KiB (48 kHz, 16-bit
stereo, 683 ms around the ring). Bell writes each mixed period one period ahead of the
play position (LPIB) and zeroes a period once the controller has read it, so an empty
ring plays silence. No interrupts are used. Bell's mixer, notes and program sound rings
are unchanged: `sound.rs` puts the virtio card and HD Audio behind one `Card`.

Gate: `python3 scripts/sound_gate.py --iso ISO --source-disk BLANK --out DIR --card hda`
(QEMU `intel-hda` + `hda-duplex`, recorded to WAV). Passed 2026-11-09: 440 Hz for 400 ms
and 1000 Hz for 310 ms, the same as the virtio card.

## Network: Intel and Realtek wired Ethernet in SexNet

`servers/sexnet/src/nic.rs` (reset and MAC), `ethernet.rs` (queues), `realtek.rs` (new).
The driver picks its path by `Model::family()`:
- e1000 (82540): unchanged.
- e1000e (82574, I217/I218/I219): legacy descriptors. I219 also gets the ich8lan bits Linux
  sets (TARC0, RFCTL NFS filter off, CTRL_EXT bit 22, TXDCTL bit 22, TIPG 0x602008).
  CTRL_EXT.DRV_LOAD tells firmware a driver owns the port (e1000e, igb, igc).
- igb (82576, I210/I211, I350) and igc (I225/I226): advanced descriptors, queue 0 at
  0xC000/0xE000, SRRCTL 2 KiB one-buffer, queues enabled after RCTL/TCTL and polled until
  they read back enabled. igc resets with CTRL.DEV_RST and has no TIPG. After reset the
  driver waits for RAH.AV and restarts PHY autonegotiation through MDIC (QEMU's igb
  keeps link down without it, as real parts can).
- Realtek r8169/r8125: own register map and 16-byte descriptors (OWN/EOR/FS/LS), CRC
  stripped from received length, same DMA pool layout and phases.

Gates (2026-11-09, all passed):
- `python3 scripts/program_net_gate.py --nic igb` and `--nic e1000e`: DHCP lease
  10.0.2.15/24, 84,360-byte HTTP download, `run +net peek` gets 200, plain `run peek` refused.
- `python3 scripts/ethernet_peer_gate.py --model igb|e1000e|e1000`: 25 exact ARP exchanges
  with RX/TX ring wraps.
- `bash scripts/sexnet_service_host_gate.sh`, including `tests/sexnet_realtek.rs` (12 tests,
  both Realtek chip types against a width-checked register mock). QEMU has no 8169 or I225.

## Bare-metal checklist (next week)

Capture the serial or on-screen log lines named here.

**Sound**
- `[kernel.cap.class.grant] ... kind=Hda`, `[bell.hda.codec]`, `[bell.hda.route]` (which
  pins and converter), `[bell.hda.ready]`. Then `beep` in Spindle.
- Laptops with digital microphones on Intel 10th gen and later often expose the audio DSP
  (class 04:01 or 04:03 prog-if 0x80) and need Sound Open Firmware; the legacy HDA path may
  find no codec there. Desktops and most AMD machines use plain HDA.
- AMD and NVIDIA controllers can need snoop enabled in PCI config space (Linux
  `azx_init_pci`); if sound is silent or garbled, that is the first suspect.
- Laptop speaker amplifiers switched by codec GPIO (vendor quirks) are not handled yet.
- If the DMA pool lands above 4 GiB on a controller without 64-bit support (GCAP bit 0),
  the stream will not start; BOOT currently allocates the pool low.

**Network**
- `[kernel.cap.nic.grant] ... model=...` names the chip; then DHCP and `run +net peek`.
- igb/igc: a part that never sets RAH.AV hits the 2000-tick deadline; SerDes or fibre igb
  boards have no copper PHY at address 1 for the autonegotiation restart; igc DEV_RST is
  treated as self-clearing; real autonegotiation takes 2-4 s (inside the 5 s deadline).
  Frames flagged with checksum errors (bits 30-31) are dropped.
- I219: ULP left on by firmware is not undone, so the PHY may never show link; no ME/AMT
  SMBus handover beyond DRV_LOAD; the SPT+ IOSFPC/FEXTNVM workarounds are not written.
  This is the most likely NIC to need work on a modern Intel desktop or laptop.
- Realtek: nothing powers up a PHY firmware left off (Linux uses PHYAR on r8169, OCP on
  r8125); Linux's per-revision ePHY/ERI/MCU writes are skipped (most 8111E-H and 8125B
  parts work without them); RTL8126 may need RX_PAUSE_SLOT_ON; check Cfg9346 locking.
