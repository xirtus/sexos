# 2026-11-23 Every USB controller driven; x17 input hardening (PR #517)

- **SexUSB drives every leased xHCI**, not only the first. Tiger Lake laptops (the
  Alienware x17 R1) have two: the Type-C/Thunderbolt xHCI at 00:0d.0, which PCI order
  leased first, and the chipset xHCI at 00:14.0 behind every USB-A port. The chipset one
  was leased to an extra slot nobody drove, so USB keyboards, mice, receivers and the
  Ventoy stick were dead on the first x17 boot. `Usb` keeps one controller selected
  (`select`), parks the others, and tags each device with its controller; events, root
  port changes, hubs, transfers, sticks, network, Bluetooth, sound and sleep all select
  the device's controller first. A controller that fails to start no longer stops the
  rest. Each extra controller gets 1 MiB of the DMA region (`DmaRegionPool::split`); the
  USB DMA grant grew from 2 to 4 MiB. New records: `[sexusb.controller.extra]`,
  `[sexusb.controller]`, `[sexusb.controllers]`, `[sexusb.controller.scan]`.
- **xHCI lease wakes the controller**: D3hot to D0 and memory/bus master on
  (`[kernel.pci.xhci.power]`), as NVMe already did; firmware may leave the controller
  it does not boot from off, and its registers then read all ones.
- **IRQ1 routing**: an ISA IRQ follows its MADT source override to the right GSI, and
  the keyboard interrupt goes to the boot core's real LAPIC id, not a hard-coded 0.
- **PS/2 stuck-byte poller** takes a byte only after 30 ms with no IRQ1, so it never
  races the interrupt handler (that race doubled or lost keys in QEMU gates).
