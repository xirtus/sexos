# Driver manifests and MSI/MSI-X interrupts (stages D2 and D3)

Date: 2026-11-09. Plan: `DRIVERS_AND_X17_R1_PLAN_2026_11_09.md` section 6.

## What changed, in plain terms

**Before.** `kernel/src/devmgr.rs` decided which service got which device with a
hard-coded `match` on PCI class, one device per kind, and every driver polled its device.

**Now.**

1. **Driver manifests (D2).** Each driver service has a match table in
   `crates/sex-hwdb/src/manifest.rs`: which classes/vendors/ids it drives, which slot its
   driver reads the first device of each kind from, how many devices it may hold, and how
   many interrupt vectors it wants per device. BOOT walks PCI once and leases each device
   to the first manifest that claims it. Adding a controller is a data change there.
   The kernel keeps only the *mechanisms*: how a lease of each kind is built (which BAR,
   how long a register view, which command bits, which DMA pool).
2. **Many devices per service.** The first device of a kind keeps the slot its driver has
   always used (NVMe 16, xHCI 8, NIC 19, sound 20, AHCI 21). Further devices land at the
   next free slot from `sex_pdx::device::EXTRA_SLOT_BASE` (160) up to 191, in the same
   domain. Two NVMe drives in QEMU now lease as slots 16 and 160; neither replaces the
   other. Extra devices share the service's existing DMA pool.
3. **MSI/MSI-X (D3).** For a manifest that wants vectors, BOOT programs the device's
   MSI-X table (or its MSI capability when it has no MSI-X) with vectors from
   0x50..0x8F aimed at the boot core, every vector masked, legacy INTx off. The MSI-X
   table is mapped into supervisor-only pages; no service can reach it.
4. **Interrupts are notification bits, not messages.** When a vector fires, the kernel
   sets that vector's bit in the owning service's pending word and wakes its main task.
   Bits coalesce, so an interrupt storm cannot fill the service's message ring.
5. **NVMe stops polling.** sexdrive creates its I/O completion queue with interrupts
   enabled on vector 0, unmasks it, and while a command is outstanding sleeps in the
   timed wait until the bit arrives (the one-tick deadline only bounds a lost
   interrupt). Without a vector it polls as before.

## ABI changes

| Change | Where |
|---|---|
| **Syscall 86 `SYS_DEVICE`**: `OP_LIST` (the n-th lease this service holds: slot, ids, class, bus/dev/fn, interrupt mode, vector count, first pending bit), `OP_UNMASK`/`OP_MASK` (one vector of one of the caller's own leases), `OP_TAKE` (read and clear the caller's pending bits) | `crates/sex-pdx/src/device.rs`, `kernel/src/device_irq.rs` |
| `sex_pdx::device::{EXTRA_SLOT_BASE = 160, EXTRA_SLOTS = 32, MAX_LEASES, Lease, Mode}` | `crates/sex-pdx/src/device.rs` |
| Syscall 70 (`sys_wait_self_ticks`) also returns when an interrupt bit is pending. The untimed inbox wait (53) does not, so a bit nobody takes can never make an idle service spin | `kernel/src/scheduler.rs` |
| `sex_hwdb::manifest` (data, not syscall ABI) | `crates/sex-hwdb/src/manifest.rs` |
| `abi_version_hash` recomputed; `crates/sex-pdx/src/device.rs` added to the ABI inputs | `sexos_build_spec.toml`, `scripts/entrypoint_build.sh` |

Syscall numbers in use by open branches that this avoids: 84 (power / crash log), 85,
90 (IOMMU `SYS_DMA_LEND`), 100/101 (IPC).

## Authority

Nothing is minted after BOOT. Syscall 86 reads BOOT's lease records, flips mask bits of
vectors BOOT already gave the caller's own leases, and clears the caller's own bits. A
service can list only its own leases. Without interrupt remapping (stage D4) a driver that
can DMA could also forge an MSI write; that is the same trust a DMA-capable driver already
has before the IOMMU, and the IOMMU thread leaves remapping off for now so these vectors
keep working.

## Where this differs from the plan, and why

- **The plan said "each driver service declares its match table in its catalog entry."**
  The tables live in `sex-hwdb` instead of inside each service's ELF. BOOT has to know
  them before any service runs, and the ELF loader has no manifest section yet; a shared
  data crate gives the same property (no device lists in kernel code, one place per
  service) today. Moving each table into a `.sexos.manifest` section of the service image
  is a small follow-up once the loader reads notes.
- **The manual says "interrupt capabilities deliver MSI-X to driver rings."** Interrupts
  set a pending bit and wake the task instead of queueing a message. A message per
  interrupt lets a fast device fill the ring that also carries IPC requests; bits cannot
  overflow, and a driver reads every pending vector in one call. This is how seL4
  notifications work and fits a multiserver design better.
- **Vectors go to the boot core only.** Spreading vectors over cores (one NVMe queue per
  core) comes with stage D6.

## Gates

- `scripts/hwdb_host_gate.sh` (CI): new `crates/sex-hwdb/tests/manifest.rs` checks the
  x17 R1 parts reach the right service and lease kind, unclaimed parts (VMD, Wi-Fi, IDE
  SATA, EHCI, virtio-net, bridges) stay unleased, and first slots match what drivers read.
- `tests/nic_command_boot.rs`: extra NIC slots are accepted; slots outside 19 and
  160..191 are still refused.
- QEMU: `filesystem_gate.py` passes with MSI-X NVMe (`[device.irq.msix]`,
  `[sexdrive.nvme.irq] mode=msix polling=0`, `[sexdrive.nvme.irq.wake]`), and with a
  second NVMe drive added (`SEXOS_GATE_DEVICES`, leased at slot 160 with its own vector).
  Sound (virtio and HDA), network (e1000e, igb), USB path and master runtime gates rerun.

## Next

- Drivers for the second device of a kind (sexdrive: second NVMe for Intel RAID 0, see
  the VMD thread's PR #84; that thread keeps `SLOT_NVME_HOST_2 == EXTRA_SLOT_BASE`).
- MSI-X in more drivers: xHCI (interrupter 0), HDA, the NICs, virtio queues. Each is:
  set the manifest's `vectors`, enable the device's interrupt, unmask, take bits.
- Devices behind Intel VMD keep polling: their MSI-X is remapped through VMD's own table.
- Vectors per core, and interrupt remapping with the IOMMU (D4).
