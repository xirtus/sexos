# IOMMU: a DMA domain for every driver (stage D4, 2026-11-09)

Stage D4 of `DRIVERS_AND_X17_R1_PLAN_2026_11_09.md`. The sex-manual promised "zero-copy
DMA via lent memory". Until now that was not safe to build: without an IOMMU, a
driver that can start DMA can write any physical address, so we gave each driver a
fixed DMA pool and copied data through it ("bounce buffers"). This stage turns on
Intel VT-d, gives each driver service its own DMA domain, and adds the first real
loan: sexdrive reads straight into Files' block buffer.

## What a device can reach now

With VT-d present (every Intel CPU with PKU we target has it; QEMU has it with
`-device intel-iommu`):

| Device | Reaches |
|---|---|
| A leased device (NVMe, xHCI, NIC, virtio sound, and AHCI/HDA once #79 lands) | its service's BOOT DMA pool at the pool's physical address, its own firmware-reserved memory (RMRR), and memory the service lends it |
| A device nobody leased, on a translated unit (dGPU, Thunderbolt, Wi-Fi, edu, ...) | nothing. Its DMA faults |
| An unleased device with firmware-reserved memory (RMRR) | only that reserved memory |
| A device on a unit with no leased devices (the Intel graphics unit) | unchanged: that unit stays off, so the firmware framebuffer keeps scanning out |

Drivers did not change to get this: pools are mapped at their physical address, so
the addresses drivers already program are valid device addresses. Every stray DMA is
blocked and logged as `[iommu.fault] device=bb:dd.f owner_pd=N address=... access=write
reason=...`, checked from the timer every 16 ticks.

## How it is built

- **`crates/sex-iommu`** (no_std, no hardware access, host-tested): DMAR parsing (units,
  device scopes through bridges, RMRR), IVRS parsing (AMD units), VT-d register fields
  and entry encodings, 3/4-level I/O page tables (VT-d and AMD-Vi entry formats), and
  the fixed lending window. `scripts/iommu_host_gate.sh`, 9 tests.
- **`kernel/src/iommu.rs`**: runs once at BOOT, after every device lease and DMA pool
  exists and before the capability graph is sealed. It does not touch `devmgr`: it
  walks every protection domain's capabilities, so every `CapabilityData::Pci` lease
  with registers (bar0_view) gets its owner's domain, and every Region with a DMA base
  is identity-mapped in that owner's domain. New leasing code (driver manifests, more
  devices per service) is covered automatically.
- One domain per service (domain id = order found), one root table per unit, context
  tables per bus. Table frames come from one BOOT reservation sized by the number of
  services; each service also gets a 128 MiB lending window whose page tables are
  built at BOOT, so lending never allocates. The window sits at the first free 1 GiB
  slot under 4 GiB, so devices limited to 32-bit DMA can use loans too.
- Units: legacy (not scalable) mode, register-based invalidation, write-buffer flush
  when the unit asks for it, clflush of table writes when the unit does not snoop.
  Firmware that left translation or protected-memory regions on (Windows "Kernel DMA
  Protection" laptops) is handled: our root table replaces theirs and PMR is turned off
  after translation is on.
- Interrupt remapping stays off. MSI/MSI-X writes to 0xFEEx_xxxx are not translated
  by DMA remapping, so the MSI-X work (syscall 86) works unchanged.

## Lending: syscall 90 (`sex_pdx::dma_lend`)

| Op | Arguments | Reply |
|---|---|---|
| `OP_INFO` (0) | none | rsi = 1 if this service's devices are behind an active IOMMU, rdx = domain id, r10 = faults, r8 = pages on loan |
| `OP_LEND` (1) | rsi = region slot, rdx = offset, r10 = bytes, r8 = `DEVICE_READ` and/or `DEVICE_WRITE` | rsi = device address, rdx = pages |
| `OP_RETURN` (2) | rsi = device address | rsi = pages |

Rules: a service can lend only memory it holds as a Region capability, and a device
gets no more than the service has (device writes need the region's WRITE right and
every page mapped writable for that service; every page must be mapped for it now,
under its own PKU key). Pages need not
be physically contiguous; the device sees them as one run. Returning unmaps the pages
and flushes the IOTLB before the call returns, after which the device faults instead
of reaching the memory. At most 64 loans per service. Without an IOMMU every LEND
answers `NO_IOMMU` and drivers keep their pools. When the kernel restarts a service,
its devices are quiesced and then every loan of the old run is returned.

## First user: sexdrive reads with no copy

At start sexdrive lends Files' 128 KiB block buffer (region 4) to its NVMe controller
for device writes (`[sexdrive.dma.loan] slot=17 active=1 ... reads=zero_copy`). Typed
reads then point the PRPs at the loan, so the SSD writes Files' buffer directly and
the 128 KiB `memcpy` per read is gone. The PRP list itself stays in sexdrive's pool.
Writes still copy into the pool on purpose: the copy is what stops Files from changing
bytes after sexdrive checked and submitted them.

A read that times out is the dangerous case for any zero-copy design: the controller
may still finish it later, into memory Files has reused. sexdrive now ends the loan
first (`[sexdrive.dma.loan.ended] reason=uncertain_io`), so a late completion hits the
IOMMU and faults instead of corrupting Files' memory. Host tests cover both paths.

## AMD-Vi, VMD, and what comes next

- **AMD machines** (`IVRS` table): units are read and reported (`[iommu.amd.unit]`,
  `[iommu.absent] vendor=amd ... reason=amd_vi_not_yet`) and everything runs as before
  with pools. The page-table code already has the AMD-Vi entry format; what remains is
  the device table, command buffer and event log (QEMU has `-device amd-iommu` to gate
  it). That is the next IOMMU step.
- **Intel VMD / RST** ("RAID On", likely on the x17 R1): drives behind VMD do DMA with
  the VMD endpoint's requester id. Follow-up agreed with the VMD thread (PR #84): a small
  table in `kernel/src/vmd.rs` (child -> VMD endpoint) that `iommu.rs` asks at BOOT, so
  the endpoint's context entry lands in the domain of the service driving those drives.
- **More loans**: sexnet receive/transmit buffers straight into connection buffers, Bell
  playing from programs' sound rings, USB mass storage into Files. Each is a driver
  change only; the kernel side is done.
- **Interrupt remapping** (needed for x2APIC with >255 CPUs and to stop devices forging
  MSIs): a later step, after MSI-X lands.
- **Queued invalidation** for faster loan/return on busy drivers (today's register
  invalidation flushes a whole domain).

## Bare metal (x17 R1) notes

Tiger Lake has two VT-d units: one only for the iGPU (00:02.0) and one INCLUDE_PCI_ALL
unit. We lease nothing on the graphics unit, so it stays off and the boot framebuffer
keeps working. The catch-all unit translates NVMe, xHCI (with its RMRR for legacy USB),
the Realtek NIC and HDA. The NVIDIA GPU, Wi-Fi and Thunderbolt are unleased and
blocked, which is also the protection against Thunderbolt DMA attacks. Look for
`[iommu.enabled]`, `[iommu.device]` and `[iommu.fault]` in the boot log; a fault from a
real driver is a driver bug worth a report. If VT-d is disabled in the BIOS there is no
DMAR table and SexOS runs exactly as before (`[iommu.absent]`).

## ABI changes (hash recomputed)

- Syscall **90** `SYS_DMA` with `OP_INFO`/`OP_LEND`/`OP_RETURN`, `DEVICE_READ`,
  `DEVICE_WRITE`, statuses `DENIED`, `NO_IOMMU`, `EXHAUSTED`, `MAX_LOANS` 64,
  `WINDOW_BYTES` 128 MiB: `crates/sex-pdx/src/dma_lend.rs`, added to `ABI_INPUTS`.
- No slot, message or shared struct changed.

## Gates

- `scripts/iommu_host_gate.sh` (CI): DMAR/IVRS, VT-d fields, page tables, window.
- `scripts/sexdrive_dma_host_gate.sh`: two new zero-copy tests (PRPs name the loan, no
  copy; a timeout ends the loan).
- `scripts/iommu_gate.py` (QEMU, VT-d): the BOOT self-test with QEMU's `edu` DMA device
  (unleased device blocked, translated read and write land, a read-only page holds, an
  unmapped address faults), then the whole `filesystem_gate` (directories, files,
  reboot, power cut) and `sound_gate` (two notes at the right pitch) with every driver
  behind VT-d and no driver fault allowed. Any gate built on `quil_autosave_gate.boot`
  can run behind VT-d with `SEXOS_GATE_IOMMU='-device intel-iommu,aw-bits=48'`.
