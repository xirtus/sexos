# IOMMU: a DMA domain for every driver (stage D4, 2026-11-09; AMD-Vi 2026-11-10)

Stage D4 of `DRIVERS_AND_X17_R1_PLAN_2026_11_09.md`. The sex-manual promised "zero-copy
DMA via lent memory". Until now that was not safe to build: without an IOMMU, a
driver that can start DMA can write any physical address, so we gave each driver a
fixed DMA pool and copied data through it ("bounce buffers"). This stage turns on
Intel VT-d, gives each driver service its own DMA domain, and adds the first real
loan: sexdrive reads straight into Files' block buffer.

## What a device can reach now

With VT-d or AMD-Vi present (every Intel and AMD CPU with PKU we target has one;
QEMU has them as `-device intel-iommu` and `-device amd-iommu`):

| Device | Reaches |
|---|---|
| A leased device (NVMe, xHCI, NIC, virtio sound, and AHCI/HDA once #79 lands) | its service's BOOT DMA pool at the pool's physical address, its own firmware-reserved memory (RMRR), and memory the service lends it |
| A device nobody leased, on a translated unit (dGPU, Thunderbolt, Wi-Fi, edu, ...) | nothing. Its DMA faults |
| An unleased device with firmware-reserved memory (RMRR) | only its own reserved memory (one small domain per such device) |
| A device on a unit with no leased devices (the Intel graphics unit) | unchanged: that unit stays off, so the firmware framebuffer keeps scanning out |

Drivers did not change to get this: pools are mapped at their physical address, so
the addresses drivers already program are valid device addresses. Every stray DMA is
blocked and logged as `[iommu.fault] device=bb:dd.f owner_pd=N address=... access=write
reason=...`, checked from the timer every 16 ticks.

## How it is built

- **`crates/sex-iommu`** (no_std, no hardware access, host-tested): DMAR parsing (units,
  device scopes through bridges, RMRR), IVRS parsing (units, device id ranges, aliases,
  IVMD), VT-d and AMD-Vi register fields, entries, commands and events, 3/4-level I/O
  page tables in both formats, and the fixed lending window. `scripts/iommu_host_gate.sh`.
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
of reaching the memory. If the IOMMU does not confirm that flush, RETURN answers
`UNCERTAIN`, the device addresses are never reused, and the service must treat the
memory as still lent. LEND is refused (`NO_IOMMU`) unless every unit translating the
service's devices really came on. At most 64 loans per service. Without an IOMMU every LEND
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

## AMD-Vi (2026-11-10)

AMD machines now get the same domains, lending and zero-copy reads as Intel ones.
Everything above the hardware is shared: one domain per driver service, BOOT DMA pools
identity-mapped, the 128 MiB lending window, syscall 90, loan revocation on restart.
Only the unit differs:

| | Intel VT-d | AMD-Vi |
|---|---|---|
| Firmware table | DMAR: units, device scopes, RMRR | IVRS: units (newest IVHD type only), device id ranges and aliases, IVMD |
| Who translates a device | scope match, else the INCLUDE_PCI_ALL unit | the unit whose device entries list its requester id |
| Per device | context entry (root table, context table per bus) | 32-byte entry in one 2 MiB device table per segment |
| Unleased device | context entry not present | entry points at an empty page table (domain 0): every DMA faults |
| Flushes | register-based context and IOTLB invalidation | command ring: INVALIDATE_DEVTAB_ENTRY, INVALIDATE_IOMMU_PAGES, COMPLETION_WAIT |
| Faults | fault recording registers | event log ring (IO_PAGE_FAULT etc.) |

Details:
- **Devices behind a PCIe-to-PCI bridge** DMA with the bridge's requester id. IVRS
  alias entries say so, and the alias gets the same domain (`[iommu.device] ... alias=1`).
- **IVMD** unity and exclusion ranges are mapped like RMRR: into the owning service's
  domain, or a small domain of its own for a device nobody leased. Exclusion ranges
  are mapped rather than programmed into the exclusion registers, so they stay per device.
- **Firmware that left the IOMMU on** (AMD pre-boot DMA protection) is turned off just
  long enough to load our device table, command ring and event log, then turned on and
  flushed with INVALIDATE_IOMMU_ALL, or per device id and domain when that is missing.
- **Interrupts** stay unremapped (device table IV = 0), so MSI/MSI-X work unchanged.
- **RETURN** of a loan waits for COMPLETION_WAIT after the domain flush; if it never
  arrives the answer is `UNCERTAIN`, exactly as on VT-d.

QEMU's AMD-Vi model blocks DMA through empty or read-only entries without writing an
event, so on AMD the self-test proves isolation by the data (sentinel pages unchanged)
and does not require logged faults; on VT-d it requires both. Real AMD hardware logs
IO_PAGE_FAULT events, which `[iommu.fault]` shows with reason `amd_page_not_present` or
`amd_access_not_permitted`.

## What comes next

- **Intel VMD / RST** ("RAID On", likely on the x17 R1): drives behind VMD do DMA with
  the VMD endpoint's requester id. Handled already, without waiting for the VMD driver
  (PR #84) to share a table: a leased device that is not on the real PCI bus (legacy
  config scan does not see it) is behind VMD, so the VMD endpoint (`sex_hwdb::is_vmd`)
  joins that service's domain (`[iommu.device] ... vmd_endpoint=1`). One requester id
  can sit in one domain only, so every drive behind one VMD must belong to one service
  (sexdrive); a second claimant is logged as `[iommu.device.shared]`. Not testable in
  QEMU (no VMD model); check the boot log on the x17 R1.
- **More loans**: sexnet receive/transmit buffers straight into connection buffers, Bell
  playing from programs' sound rings, USB mass storage into Files. Each is a driver
  change only; the kernel side is done.
- **Interrupt remapping** (needed for x2APIC with >255 CPUs and to stop devices forging
  MSIs): a later step, now that MSI-X has landed. Both vendors.
- **Queued invalidation** on VT-d and page-selective flushes on both, for faster
  loan/return on busy drivers (today a return flushes the whole domain).
- **More than one PCI segment** (large servers): units on segment 1+ are skipped today.

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
  `DEVICE_WRITE`, statuses `DENIED`, `NO_IOMMU`, `EXHAUSTED`, `UNCERTAIN`, `MAX_LOANS` 64,
  `WINDOW_BYTES` 128 MiB: `crates/sex-pdx/src/dma_lend.rs`, added to `ABI_INPUTS`.
- No slot, message or shared struct changed.

## Gates

- `scripts/iommu_host_gate.sh` (CI): DMAR/IVRS (device ranges, aliases, IVMD, the
  newest-IVHD-type rule), VT-d fields, AMD-Vi entries, commands and events, page tables
  in both formats, window. 12 tests. Writing the AMD-Vi table test found and fixed a
  wrong next-level field in AMD page-table entries before any AMD machine ran it.
- `scripts/sexdrive_dma_host_gate.sh`: two new zero-copy tests (PRPs name the loan, no
  copy; a timeout ends the loan).
- `scripts/iommu_gate.py` (QEMU, VT-d; `--vendor amd` for AMD-Vi): the BOOT self-test with QEMU's `edu` DMA device
  (unleased device blocked, translated read and write land, a read-only page holds, an
  unmapped address faults), then the whole `filesystem_gate` (directories, files,
  reboot, power cut) and `sound_gate` (two notes at the right pitch) with every driver
  behind VT-d and no driver fault allowed. Any gate built on `quil_autosave_gate.boot`
  can run behind VT-d with `SEXOS_GATE_IOMMU='-device intel-iommu,aw-bits=48'`.
