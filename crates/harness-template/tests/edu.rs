// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The template driver against a model of the device: what works, and every
//! way the device can let it down.
mod model;

use edu::*;
use harness::mock::{Lines, MockClock, MockDma, MockIrq, ModelRegs};
use harness::{Error, NoIrq, Regs};
use model::{EduModel, Faults};

fn started(model: EduModel) -> (Edu<ModelRegs<EduModel>>, MockClock, Lines) {
    let mut clock = MockClock::new();
    let mut out = Lines::new();
    let edu = Edu::start(ModelRegs::new(model), &mut clock, &mut out).expect("start");
    (edu, clock, out)
}

fn poll<'a>() -> Wait<'a, NoIrq> { Wait::Poll }

#[test]
fn starts_and_reports_version() {
    let (edu, _, out) = started(EduModel::new());
    assert_eq!((edu.major, edu.minor), (1, 0));
    assert_eq!(out.find("edu.ready").as_deref(), Some("[edu.ready] version=1.0 id=0x10000ed"));
    // Liveness was checked with two patterns.
    let regs = edu.into_regs();
    assert_eq!(regs.writes_to(LIVENESS), vec![0x5a5a_a5a5, 0x0f0f_f0f0]);
}

#[test]
fn refuses_wrong_dead_and_unresponsive_devices() {
    for (faults, why) in [
        (Faults { bad_id: true, ..Default::default() }, Error::NotThisDevice),
        (Faults { dead: true, ..Default::default() }, Error::Gone),
        (Faults { no_liveness: true, ..Default::default() }, Error::BadValue),
    ] {
        let mut m = EduModel::new();
        m.faults = faults;
        let mut out = Lines::new();
        let r = Edu::start(ModelRegs::new(m), &mut MockClock::new(), &mut out);
        assert_eq!(r.err(), Some(why));
        assert_eq!(out.field("edu.refused", "reason").as_deref(), Some(why.name()));
    }
}

#[test]
fn factorial_by_polling_and_by_interrupt() {
    let (mut edu, mut clock, mut out) = started(EduModel::new());
    assert_eq!(edu.factorial(5, &mut clock, poll(), &mut out), Ok(120));
    assert_eq!(edu.factorial(12, &mut clock, poll(), &mut out), Ok(479_001_600));
    assert_eq!(edu.factorial(13, &mut clock, poll(), &mut out), Err(Error::BadRequest));

    let line = harness::mock::IrqLine::new();
    let mut m = EduModel::new();
    m.irq = line.clone();
    let regs = ModelRegs::new(m);
    // The device works while the driver sleeps waiting for its interrupt.
    let mut clock = MockClock::new().drive(regs.handle());
    let mut out = Lines::new();
    let mut edu = Edu::start(regs, &mut clock, &mut out).unwrap();
    let mut irq = MockIrq::new(line, 1);
    assert_eq!(edu.factorial(6, &mut clock, Wait::Interrupt(&mut irq), &mut out), Ok(720));
    assert_eq!(out.field("edu.factorial", "irq").as_deref(), Some("1"));
    assert_eq!(out.field("edu.irq", "status").as_deref(), Some("0x1"));
    assert_eq!(irq.unmasked, 1);
}

#[test]
fn dma_by_interrupt() {
    let line = harness::mock::IrqLine::new();
    let mut dma = MockDma::new(4096, 0x2000);
    let mut m = EduModel::new();
    m.irq = line.clone();
    m.bus = Some(dma.port());
    let regs = ModelRegs::new(m);
    let mut clock = MockClock::new().drive(regs.handle());
    let mut out = Lines::new();
    let mut edu = Edu::start(regs, &mut clock, &mut out).unwrap();
    let mut irq = MockIrq::new(line, 1);
    let mut pool = dma.pool();
    let a = pool.take(128, 8).unwrap();
    a.cpu.fill(0x77);
    edu.dma(Direction::ToDevice, &a, 0, 128, &mut clock, Wait::Interrupt(&mut irq), &mut out).unwrap();
    assert_eq!(out.field("edu.irq", "status").as_deref(), Some("0x100"));
    let regs = edu.into_regs();
    assert!(regs.model().buffer[..128].iter().all(|&b| b == 0x77));
}

#[test]
fn interrupt_status_is_acknowledged() {
    let (mut edu, _, mut out) = started(EduModel::new());
    edu.raise(0x42);
    assert_eq!(edu.acknowledge(&mut out), 0x42);
    assert_eq!(edu.acknowledge(&mut out), 0);
    assert_eq!(out.field("edu.irq", "status").as_deref(), Some("0x42"));
}

#[test]
fn stuck_device_times_out_quickly() {
    let mut m = EduModel::new();
    m.faults.stuck = true;
    let (mut edu, mut clock, mut out) = started(m);
    assert_eq!(edu.factorial(4, &mut clock, poll(), &mut out), Err(Error::Timeout));
    assert!(clock.now >= TIMEOUT_MS, "the deadline, not a spin, ended the wait");
    assert_eq!(out.field("edu.factorial", "error").as_deref(), Some("timeout"));
}

#[test]
fn dma_round_trip() {
    let mut dma = MockDma::new(64 * 1024, 0x10_0000);
    let mut m = EduModel::new();
    m.bus = Some(dma.port());
    let (mut edu, mut clock, mut out) = started(m);
    let mut pool = dma.pool();
    let a = pool.take(256, 64).unwrap();
    let b = pool.take(256, 64).unwrap();
    for (i, x) in a.cpu.iter_mut().enumerate() { *x = i as u8 ^ 0x5a; }
    edu.dma(Direction::ToDevice, &a, 16, 256, &mut clock, poll(), &mut out).unwrap();
    edu.dma(Direction::ToRam, &b, 16, 256, &mut clock, poll(), &mut out).unwrap();
    assert_eq!(a.cpu, b.cpu);
    let regs = edu.into_regs();
    assert!(!regs.model().aborted);
    assert_eq!(dma.faults(), 0);
}

#[test]
fn dma_requests_outside_the_device_limits_never_reach_it() {
    let mut dma = MockDma::new(8192, 0x10_0000);
    let (mut edu, mut clock, mut out) = started(EduModel::new());
    let mut pool = dma.pool();
    let buf = pool.take(4096, 64).unwrap();
    for (off, len) in [(0, 0), (0, 4097), (4000, 200), (u32::MAX, 2)] {
        assert_eq!(edu.dma(Direction::ToDevice, &buf, off, len, &mut clock, poll(), &mut out), Err(Error::BadRequest));
    }
    // Past the device's 28-bit DMA reach.
    let mut far = MockDma::new(4096, (1 << 28) - 16);
    let mut far_pool = far.pool();
    let fb = far_pool.take(64, 1).unwrap();
    assert_eq!(edu.dma(Direction::ToDevice, &fb, 0, 64, &mut clock, poll(), &mut out), Err(Error::BadRequest));
    assert!(edu.into_regs().writes_to(DMA_CMD).is_empty());
}

#[test]
fn info_is_consistent() {
    assert!(INFO.tags_ok());
    let id = harness::manifest::Id { vendor: VENDOR, device: DEVICE, class: 0x00, subclass: 0xff, prog_if: 0 };
    assert!(INFO.claims(id));
    assert!(!INFO.claims(harness::manifest::Id { device: 0x1111, ..id }));
}

#[test]
fn regs_trait_object_works() {
    // A driver can also take `&mut dyn Regs` when monomorphising is unwanted.
    let mut r = ModelRegs::new(EduModel::new());
    let d: &mut dyn Regs = &mut r;
    assert_eq!(d.read32(ID) & 0xffff, 0xed);
}
