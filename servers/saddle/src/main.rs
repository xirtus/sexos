// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Saddle: the service that runs drivers built on the Harness SDK
//! (`crates/harness`), for devices no system service drives.
//!
//! BOOT leases Saddle each device its manifest names (`sex_hwdb::manifest::SADDLE`)
//! as a generic lease: a view of BAR0, memory and bus-master command bits, MSI
//! vectors and a DMA pool behind the IOMMU. Saddle holds that authority; each driver
//! in [`DRIVERS`] is a plain Harness crate that gets only a `Regs`, a `Clock`, an
//! `Irq`, DMA buffers and a record sink. A driver that faults takes Saddle down,
//! and the kernel restarts Saddle on a reset device, as it does every driver
//! service (`kernel/src/service_restart.rs`).
//!
//! Drivers are linked in today. A driver joins by adding its crate, a row in
//! [`DRIVERS`] and its device ids in the SADDLE manifest.
#![no_std]
#![no_main]

use harness::dma::DmaPool;
use harness::manifest::Id;
use harness::record::Record;
use harness::sexos::{dma_pool, lease, LeaseIrq, LeasedRegs, Serial};
use harness::Clock;
use harness::{DriverInfo, Error};
use sex_hwdb::manifest::SADDLE_SLOT;

/// One hosted driver: what it claims, and how Saddle runs it on a lease.
struct Hosted {
    info: &'static DriverInfo,
    run: fn(LeasedRegs, sex_pdx::device::Lease, &mut DmaPool<'static>) -> Result<(), Error>,
}

static DRIVERS: &[Hosted] = &[Hosted { info: &edu::INFO, run: run_edu }];

/// The template driver's bring-up and a self test of each of its operations:
/// polled and interrupt-driven factorials, and a DMA round trip.
fn run_edu(regs: LeasedRegs, l: sex_pdx::device::Lease, pool: &mut DmaPool<'static>) -> Result<(), Error> {
    use edu::{Direction, Edu, Wait};
    let (mut clock, mut out) = (YieldClock, Serial);
    let mut dev = Edu::start(regs, &mut clock, &mut out)?;
    // Saddle's gate starts edu with a 32-bit DMA mask; BOOT's pools sit below 4 GiB.
    dev.set_dma_limit(1 << 32);
    let polled = dev.factorial(5, &mut clock, Wait::<harness::NoIrq>::Poll, &mut out) == Ok(120);
    let mut irq = LeaseIrq::new(l);
    let by_irq = l.vectors > 0 && dev.factorial(10, &mut clock, Wait::Interrupt(&mut irq), &mut out) == Ok(3_628_800);
    let dma = (|| {
        let a = pool.take(512, 64).ok_or(Error::BadRequest)?;
        let b = pool.take(512, 64).ok_or(Error::BadRequest)?;
        for (i, x) in a.cpu.iter_mut().enumerate() { *x = (i as u8).wrapping_mul(7) ^ 0xa5; }
        dev.dma(Direction::ToDevice, &a, 64, 512, &mut clock, Wait::<harness::NoIrq>::Poll, &mut out)?;
        dev.dma(Direction::ToRam, &b, 64, 512, &mut clock, Wait::<harness::NoIrq>::Poll, &mut out)?;
        if a.cpu == b.cpu { Ok(()) } else { Err(Error::BadValue) }
    })();
    let ok = polled && dma.is_ok();
    Record::new("saddle.selftest").text("driver", "edu").flag("ok", ok).flag("factorial", polled)
        .flag("irq", by_irq).num("vectors", l.vectors as u64).text("dma", dma.err().map_or("ok", Error::name)).emit(&mut Serial);
    if ok { Ok(()) } else { Err(Error::BadValue) }
}

fn id_of(l: &sex_pdx::device::Lease) -> Id {
    Id { vendor: l.vendor, device: l.device, class: (l.class >> 16) as u8, subclass: (l.class >> 8) as u8, prog_if: l.class as u8 }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    Record::new("saddle.start").num("drivers", DRIVERS.len() as u64).emit(&mut Serial);
    let mut pool = dma_pool().ok();
    let mut started = 0u64;
    // Every lease Saddle holds: its first device at SADDLE_SLOT, further ones in
    // the extra slots, in BOOT's order.
    for index in 0..sex_pdx::device::MAX_LEASES as u64 {
        let Some(l) = sex_pdx::device::list(index) else { break };
        if l.slot != SADDLE_SLOT && !(sex_pdx::device::EXTRA_SLOT_BASE..sex_pdx::device::EXTRA_SLOT_BASE + sex_pdx::device::EXTRA_SLOTS).contains(&l.slot) { continue; }
        let id = id_of(&l);
        let Some(h) = DRIVERS.iter().find(|h| h.info.claims(id)) else {
            Record::new("saddle.unclaimed").hex("vendor", l.vendor as u64).hex("device", l.device as u64).num("slot", l.slot).emit(&mut Serial);
            continue;
        };
        let r = match (lease(l.slot), LeasedRegs::open(l.slot), pool.as_mut()) {
            (Ok(_), Ok(regs), Some(pool)) => regs.enable(true).and_then(|_| (h.run)(regs, l, pool)),
            (_, Err(e), _) | (Err(e), _, _) => Err(e),
            (_, _, None) => Err(Error::Denied),
        };
        let rec = Record::new("saddle.driver").text("name", h.info.name).hex("vendor", l.vendor as u64).hex("device", l.device as u64).num("slot", l.slot);
        match r {
            Ok(()) => { started += 1; rec.text("state", "running").emit(&mut Serial) }
            Err(e) => rec.text("state", "failed").text("reason", e.name()).emit(&mut Serial),
        }
    }
    Record::new("saddle.ready").num("drivers", started).emit(&mut Serial);
    // Untimed park: hosted drivers are interrupt-free here, and a timed poll
    // would be an unreviewed self-wait (check_self_wait.py admits no task 53).
    loop {
        sex_pdx::sys_wait_self();
    }
}

/// Saddle's clock yields instead of parking on a timer: a timed self-wait from a
/// new boot task is refused by the self-wait gate until it is reviewed, and the
/// hosted self tests only wait a few milliseconds.
struct YieldClock;
impl Clock for YieldClock {
    fn now_ms(&mut self) -> u64 { sex_pdx::get_ticks() }
    fn sleep_ms(&mut self, _ms: u64) { sex_pdx::sys_yield() }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[saddle.panic]");
    // Untimed park: hosted drivers are interrupt-free here, and a timed poll
    // would be an unreviewed self-wait (check_self_wait.py admits no task 53).
    loop {
        sex_pdx::sys_wait_self();
    }
}
