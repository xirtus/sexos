# Harness: writing a SexOS driver

Harness is the SexOS driver SDK (`crates/harness`). It is for anyone, inside or
outside the project, who wants to make a device work on SexOS with a native
driver: a Rust crate that runs in a ring-3 driver service, on a device BOOT
leased to that service, behind the IOMMU. Its sibling for apps is Leather
(`docs/sdk/LEATHER_WEFT_TUTORIAL.md`).

What you get:

| Piece | Where | What it is |
|---|---|---|
| The SDK crate | `crates/harness` | `Regs`, `Clock`, `Irq`, `DmaPool`/`DmaBuf`/`RingIndex`, bounded waits, `[tag] key=value` records, `Error` |
| A device model kit | `harness::mock` (`mock` feature) | `DeviceModel` + `ModelRegs`, `MockClock`, `MockIrq`/`IrqLine`, `MockDma`/`BusPort`, `Lines` |
| Fuzz hooks | `harness::fuzz` (`mock` feature) | `FuzzRegs`, `FuzzIrq`, `Src`, the built-in mutator and `fuzz_main!`, a hang budget |
| System bindings | `harness::sexos` (`sexos` feature) | the lease's BAR view, the service's DMA pool, syscall 86 interrupts, the clock, the serial record stream |
| A template driver | `crates/harness-template` | a complete driver for QEMU's `edu` device, with model tests and a fuzz binary |
| The host service | `servers/saddle` | runs Harness drivers inside SexOS on generic leases |
| The hardware list | `docs/HARDWARE.md`, `scripts/hardware_list.py` | what is supported and what real machines showed |

Licence: Harness and the template are `GPL-2.0-or-later OR MIT`, like every
crate a driver service links (`docs/LICENSING.md`), so a driver may also carry
GPL-2.0 code from another system's driver.

## 1. The shape of a SexOS driver

SexOS is not Linux and its drivers are not Linux drivers (`docs/DESIGN_GUARDRAILS.md`).
There is no `probe()` callback, no interrupt handler, no `ioremap`, no
`dma_alloc_coherent`, no sleeping lock, no `/dev` node. Instead:

- **BOOT leases the device.** At boot the kernel walks PCI once and leases each
  device to the service whose manifest (`sex_hwdb::manifest`) matches it: a view of
  the right BAR at a slot, PCI command bits as the lease kind allows, a DMA pool
  behind the IOMMU, and MSI/MSI-X vectors. The kernel only builds leases; which
  service gets which device is data.
- **The service owns the authority; the driver crate owns the logic.** The
  service's `main` opens the lease (`harness::sexos`), then hands the driver crate
  plain values: a `Regs`, a `Clock`, an `Irq`, DMA buffers and a record `Sink`. The
  driver crate itself holds no syscalls, so the same code runs against a model on
  your laptop.
- **Interrupts are bits, not handlers.** A raised vector sets a pending bit and
  wakes the service. The driver sleeps (`Clock::sleep_ms` returns early on a bit),
  takes the bits (`Irq::take`) and handles them in ordinary code.
- **Everything the device says is untrusted.** The device, its firmware and
  whatever is plugged into it can be buggy, hostile or simply gone. All-ones reads
  mean gone. Ring indices, lengths, descriptor fields and firmware tables are checked
  before use. A driver never panics and never waits without a bound.
- **The driver reports in records.** `[edu.ready] version=1.0 id=0x10000ed`. QEMU gates
  read them, Spindle `hw save` collects them on real machines, and the hardware
  list is built from them.

## 2. Start from the template

`crates/harness-template` drives QEMU's `edu` teaching device (PCI `1234:11e8`,
`qemu-system-x86_64 -device edu`): identity, a liveness check, a slow operation
with an optional interrupt, and DMA both ways. Copy it:

```sh
cp -r crates/harness-template crates/sex-mydevice
# Cargo.toml: rename the package and lib, keep the licence; add the crate to the
# root workspace members.
```

Then replace the register map and the operations. The parts to keep are the
patterns:

```rust
pub fn start<C: Clock, S: Sink>(mut regs: R, clock: &mut C, out: &mut S) -> Result<Self, Error> {
    let id = regs.read32(ID);
    if is_gone(id) { return refuse(out, Error::Gone, id); }        // fell off the bus
    if id & 0xffff != 0x00ed { return refuse(out, Error::NotThisDevice, id); }
    // ... a liveness check with two patterns, so a stuck register cannot pass
    Record::new("edu.ready").text("version", &version(major, minor)).emit(out);
    Ok(Edu { regs, major, minor })
}
```

```rust
// Every wait is bounded by a deadline AND a poll count; all ones ends it as Gone.
poll_until(&mut self.regs, clock, STATUS, STATUS_COMPUTING, 0, TIMEOUT_MS)?;
```

```rust
// Every request is checked against the device's limits before the device sees it.
if len == 0 || len as usize > buf.len() { return Err(Error::BadRequest); }
if ram.checked_add(len as u64).map_or(true, |e| e > DMA_LIMIT) { return Err(Error::BadRequest); }
```

```rust
// Interrupt mode: unmask, sleep until the bit, acknowledge, then confirm by polling.
irq.unmask(0)?;
let got = wait_irq(irq, clock, 1, TIMEOUT_MS);
self.acknowledge(out);
got?;
```

## 3. Registers, time, interrupts, DMA

| Need | Use | Rule |
|---|---|---|
| Read or write a register | `Regs::read32/write32` (`read64/write64` are two 32-bit accesses, low first; `modify32`) | Never a raw pointer in the driver. `Mmio` refuses (and counts) unaligned and out-of-window offsets. |
| Know the device is gone | `regs::is_gone(v)` | All ones = removed, powered down or a bus error. |
| Pull a field | `regs::field(v, shift, width)` | |
| Wait for a bit | `wait::poll_until(regs, clock, off, mask, want, ms)` | Returns `Timeout` or `Gone`; never spins forever. |
| Wait for anything | `wait::wait_for(clock, ms, || ...)` | At most `wait::MAX_POLLS` looks, even on a clock that never moves. |
| Wait for an interrupt | `irq::wait_irq(irq, clock, bits, ms)` | Returns every bit it took, so none is lost. |
| A device without interrupts | `irq::NoIrq` | |
| DMA memory | `DmaPool::take(len, align)` → `DmaBuf { cpu, bus }` | Buffers borrow the pool: none outlives it, none overlaps. Accessors are bounds checked. |
| An index the device wrote | `RingIndex::checked(v)` | The only way a device value becomes an index. |
| Report | `Record::new("name.event").num(..).hex(..).text(..).bytes(..).emit(out)` | Tags `[a-z0-9_.]`, starting with your driver's name. Hex blobs (`bytes`) become fuzz seeds when a real machine reports them. |

## 4. Test against a model

A model is a `DeviceModel`: what each read returns, what each write does, and a
`step` that lets background work (a computation, a DMA transfer) finish. Wrap it
in `ModelRegs` and the driver cannot tell it from hardware. `ModelRegs` keeps every
access (`writes_to(offset)`), and `MockClock::new().drive(regs.handle())` lets the
model work while the driver sleeps waiting for an interrupt.

```rust
let line = IrqLine::new();
let mut dma = MockDma::new(4096, 0x2000);          // bus addresses 0x2000..0x3000
let mut model = EduModel::new();
model.irq = line.clone();
model.bus = Some(dma.port());                       // the model's side of DMA memory
let regs = ModelRegs::new(model);
let mut clock = MockClock::new().drive(regs.handle());
let mut out = Lines::new();
let mut edu = Edu::start(regs, &mut clock, &mut out)?;
let mut irq = MockIrq::new(line, 1);
assert_eq!(edu.factorial(6, &mut clock, Wait::Interrupt(&mut irq), &mut out), Ok(720));
assert_eq!(out.field("edu.irq", "status").as_deref(), Some("0x1"));
```

Give the model knobs that break it (dead, stuck, wrong identity, a register that
does not answer) and test every one; see `crates/harness-template/tests/`. A model
DMA outside the memory you gave it is counted as an IOMMU fault (`MockDma::faults`).

```sh
cargo test -p harness-template
```

## 5. Fuzz it

Models test the device you expect. The fuzzer tests the device you get: every
register read is the next word of the input (`FuzzRegs`), interrupt bits arrive
whenever the input says (`FuzzIrq`), and past the end of the input the device
either fell off the bus or hung. A target passes when the driver neither panics
nor reads the device more than `fuzz::READ_BUDGET` times for one input.

```rust
fn run(d: &[u8]) {
    let regs = FuzzRegs::new(d).pin(ID, 0x0100_00ed);  // pin the identity so bring-up gets through
    let mut clock = MockClock::new();
    let Ok(mut dev) = Edu::start(regs, &mut clock, &mut Quiet) else { return };
    // ... drive every operation
}
fn seeds() -> Vec<Vec<u8>> { /* inputs that look like a healthy device */ }
harness::fuzz_main!(run, seeds);
```

```sh
cargo run --release -p harness-template --features fuzz --bin fuzz_edu          # 3000 inputs
HARNESS_FUZZ_SECONDS=300 cargo run --release -p harness-template --features fuzz --bin fuzz_edu
./target/release/fuzz_edu crashes/fuzz_edu-*.bin                                 # replay a finding
```

For coverage-guided fuzzing, the same `run` goes in a cargo-fuzz target:
`fuzz_target!(|d: &[u8]| harness::fuzz::one(run, d));`. In-tree drivers also get a
target in `fuzz/drivers` (see its README), which runs on every driver pull request
and for 20 minutes per target every night.

## 6. Run it in a service

The service's `main` is the only place with authority. With the `sexos` feature:

```rust
use harness::sexos::{lease, dma_pool, LeasedRegs, LeaseIrq, Serial, SystemClock};
let l = lease(SLOT)?;                  // what BOOT leased at SLOT
let regs = LeasedRegs::open(SLOT)?;    // the BAR view of that lease
regs.enable(true)?;                    // memory decode + bus master, if the lease kind allows
let mut pool = dma_pool()?;            // this service's DMA pool, behind the IOMMU
let mut irq = LeaseIrq::new(l);
let mut dev = my_driver::Device::start(regs, &mut SystemClock, &mut Serial)?;
```

### Saddle: where outside drivers run

A device no system service drives gets a **generic lease** (`Lease::Generic`) to
**Saddle** (`servers/saddle`, BOOT domain 54): a view of BAR0 (at least 256 bytes,
at most 64 KiB of it), memory and bus-master command bits, one MSI vector, and a
256 KiB DMA pool behind the IOMMU. The kernel knows nothing else about the device.
Saddle holds that authority and runs each driver in its table with only `Regs`,
`Clock`, `Irq`, DMA buffers and a record sink. A driver that faults takes Saddle
down; the kernel resets the device and starts Saddle again, as for every driver
service.

To run your driver there:

1. Add the device's exact id to the `SADDLE` manifest in
   `crates/sex-hwdb/src/manifest.rs` (`Match::id(VENDOR, DEVICE, Lease::Generic, SADDLE_SLOT)`).
   Saddle is consulted last, so it never takes a device a system service drives.
2. Add your crate to `servers/saddle/Cargo.toml` and a row to `DRIVERS` in
   `servers/saddle/src/main.rs`: your `DriverInfo` and a function that starts your
   driver on the lease.
3. Add a row to `scripts/device_matrix_gate.py` that boots QEMU with your device (if
   QEMU models it) and names the record lines that prove it came up.

The template shows all three: QEMU's edu device boots in the `edu_saddle` row,
and the gate requires `[saddle.selftest] driver=edu ok=1`.

Not yet: drivers loaded at run time (today they are linked into Saddle), one PKU
key per driver (Saddle shares the Senses organ's key, so the IOMMU, not PKU,
fences an outside driver's DMA from other services), and devices whose registers
are not in BAR0. A system service's device (a disk, a NIC, a GPU) uses its own
lease kind; a new kind is kernel growth, named as such in its pull request
(`docs/DESIGN_GUARDRAILS.md`, section 1).

## 7. Get it on the hardware list

`docs/HARDWARE.md` is generated; never edit it.

1. Add the device's row to the census table in `crates/sex-hwdb/src/lib.rs`
   (`exact()`): model name, service, support level. `Driven` only when a driver on
   master drives it; `InProgress` while it is in a branch.
2. Run `python3 scripts/hardware_list.py` and commit `docs/HARDWARE.md` and
   `docs/hardware.json` with it.
3. When a real machine runs it, its owner saves a report with Spindle `hw save`
   and files it (`hw-reports/README.md`). The next regeneration shows the machine in
   the device's "Seen on" column, as `works` or `failed`. If your driver answers a
   question no check asks yet, add a row to `CHECKS` in `scripts/hw_report.py`.

## 8. Before you send it

- `cargo test -p YOUR_CRATE` passes, with a model test for each way the device can fail.
- The fuzz binary runs a few minutes clean (`HARNESS_FUZZ_SECONDS=300`).
- `cargo build -p YOUR_CRATE --target x86_64-unknown-none` builds (the crate is `no_std`).
- No Linux API shapes: no file descriptors, ioctls, errno, `/dev` or `/sys` paths, no
  kernel code. Code ported from another system's driver is listed in
  `docs/BRIDGE_LEDGER.md` table 2, and a GPL-2.0-only port keeps the service GPL-2.0
  (`docs/LICENSING.md`).
- The device's row in sex-hwdb and the regenerated hardware list.
