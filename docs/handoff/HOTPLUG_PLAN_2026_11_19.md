# Hot-plug everywhere: docks, monitors and eGPUs (plan, 2026-11-19)

Goal: on the Alienware x17 R1 (Tiger Lake, Thunderbolt 4 on the CPU, RTX 3070) and
on ordinary PCs, a USB-C or Thunderbolt dock, an external monitor or an eGPU
enclosure can be plugged in and pulled out while SexOS runs, and the desktop and
programs keep going.

## What exists before this work

- USB: SexUSB already enumerates hubs and devices plugged in after boot and lets
  go when they are pulled (sticks, USB Ethernet CDC-ECM, HID, Bluetooth). Gates:
  `usb_storage_gate.py`, `usb_net_gate.py`.
- Displays: `sex-kms` models several outputs (`layout.rs`: arrange, output_at,
  move_to); `sex-gpu-intel::hotplug` reads Type-C/combo port hot-plug status.
  Display takes over the Intel panel (PR #453) but has no full modeset yet, so an
  external monitor on the x17 cannot be lit natively.
- PCIe: every device lease is made once at BOOT (`kernel/src/devmgr.rs`), and the
  kernel's BOOT authority is sealed after that (`runtime_mint=0`). Nothing noticed
  a device that arrived later.

## Stages

| Stage | What | Where | State |
|---|---|---|---|
| H1 | PCIe native hot-plug: find ports, slot life (settle, power, link, button, surprise removal), number buses, assign BARs and bridge windows below a port, quiet on removal | `crates/sex-hotplug`, `kernel/src/hotplug.rs` | this PR |
| H2 | Runtime driver leases for hot-plugged devices: BOOT prepares, per hot-plug port, the capability slots, page-table leaves over the port's windows and an IOMMU domain; arrival only fills prepared leaves (D1 pattern, like IOMMU lending). Services learn of new or lost leases from a reserved pending bit on `SYS_DEVICE` (86) and re-read `OP_LIST`. First users: sexdrive (NVMe hot-add/remove), sexnet (dock NICs) | kernel `device_irq.rs`, `devmgr.rs`, `boot_memory.rs`; sexdrive, sexnet, sexusb | next |
| H3 | Monitors: hot-plug events from virtio-gpu (QEMU) and Intel Type-C/DP-alt ports; Display keeps one scanout per output; Shell re-lays out the desktop (`sex-kms::layout`) and moves windows from a monitor that left onto the ones that remain | sexdisplay, silk-shell, sex-kms | after #453 (Intel modeset is the "Add native Intel display driver" thread's) |
| H4 | eGPU: an NVIDIA GPU arriving below a Thunderbolt port is leased to sexgpu-nv at runtime (large prefetchable BAR window from the plan); removal stops it cleanly | sexgpu-nv with the "Bring up the NVIDIA RTX 3070" thread | after H2 |
| H5 | USB-C docks: USB3 hubs and multi-TT hubs, Realtek RTL8153/8156 dock Ethernet (vendor or CDC-NCM config), dock audio, and the dock's DP alt-mode screens through H3 | sexusb, sexnet | parallel to H2 |

## Who owns what

- Thunderbolt/USB4 controller (NHI rings, ICM connection manager, security levels,
  device approval, tunnels): "Add Wi-Fi 6, Bluetooth audio and Thunderbolt" thread.
  Its server tells hot-plug "tunnel up/down at root port bb:dd.f"; native PCIe
  hot-plug on the root port fires by itself too, so that message is a hint.
- `_OSC` (firmware hands native PCIe hot-plug to the OS): sexacpi, owned by the x17
  thread. Until it runs, firmware may also act on Thunderbolt root ports. H1 only
  touches slot control on ports whose slot says hot-plug capable.
- Intel display modeset and Type-C port bring-up: "Add native Intel display driver".

## Design notes

- Polling, not interrupts, for H1: three config reads per port every 20 ms on core 0
  is cheaper than wiring each port's MSI, and works on ports whose MSI is broken.
  Moving to the port's MSI later is a contained change in `hotplug.rs`.
- Arrival work runs in the tick, so it never allocates and keeps its working tree
  in a static buffer (`WORK`), and the planner bounds its stack (24 items a level).
- Bus numbers come from the range firmware reserved below the port (Thunderbolt
  firmware reserves plenty; QEMU needs `bus-reserve`). A tree that does not fit is
  logged as `[hotplug.tree.partial]` and planned as far as it goes.
- I/O BARs are never assigned (no driver here uses I/O space).
