// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Harness's own guarantees: waits end, bad device values are refused, records
//! parse, DMA stays in bounds, and the fuzz runner catches panics and hangs.
use harness::dma::{DmaPool, RingIndex};
use harness::fuzz::{self, FuzzRegs, Src};
use harness::irq::wait_irq;
use harness::mock::{IrqLine, Lines, MockClock, MockDma, MockIrq, ModelRegs, DeviceModel};
use harness::record::{valid_tag, Record, MAX_LINE};
use harness::regs::{field, Mmio};
use harness::wait::{poll_until, wait_for, MAX_POLLS};
use harness::{Clock, Error, Irq, Regs};

struct Fixed(u32);
impl DeviceModel for Fixed {
    fn read32(&mut self, _: u32) -> u32 { self.0 }
    fn write32(&mut self, _: u32, v: u32) { self.0 = v }
}

#[test]
fn poll_until_ends_on_deadline_on_a_frozen_clock_and_on_gone() {
    let mut regs = ModelRegs::new(Fixed(0));
    regs.logging = false;
    let mut clock = MockClock::new();
    assert_eq!(poll_until(&mut regs, &mut clock, 0, 1, 1, 50), Err(Error::Timeout));
    assert!(clock.now >= 50);
    // A clock that never moves (sleep does nothing): the poll count ends it.
    struct Frozen;
    impl Clock for Frozen { fn now_ms(&mut self) -> u64 { 0 } fn sleep_ms(&mut self, _: u64) {} }
    let mut n = 0u32;
    assert_eq!(wait_for(&mut Frozen, 10, || { n += 1; None::<()> }), Err(Error::Timeout));
    assert_eq!(n, MAX_POLLS);
    // All ones is a dead device, even when waiting for a bit to set.
    let mut gone = ModelRegs::new(Fixed(u32::MAX));
    assert_eq!(poll_until(&mut gone, &mut clock, 0, 1, 1, 50), Err(Error::Gone));
    assert_eq!(poll_until(&mut gone, &mut clock, 0, 1, 0, 50), Err(Error::Gone));
    let mut ok = ModelRegs::new(Fixed(0x10));
    assert_eq!(poll_until(&mut ok, &mut clock, 0, 0x10, 0x10, 50), Ok(0x10));
}

#[test]
fn mmio_refuses_stray_offsets() {
    let mut backing = [0u32; 4];
    let mut m = unsafe { Mmio::new(backing.as_mut_ptr() as *mut u8, 16) };
    m.write32(4, 0xabcd);
    assert_eq!(m.read32(4), 0xabcd);
    assert_eq!(m.read32(16), u32::MAX);
    assert_eq!(m.read32(2), u32::MAX);
    m.write32(u32::MAX - 1, 1);
    assert_eq!(m.stray_accesses(), 3);
    assert_eq!(m.read64(0), 0xabcd_u64 << 32);
    assert_eq!(backing[1], 0xabcd);
}

#[test]
fn fields() {
    assert_eq!(field(0xabcd_1234, 8, 8), 0x12);
    assert_eq!(field(0xabcd_1234, 28, 8), 0xa);
    assert_eq!(field(0xabcd_1234, 0, 32), 0xabcd_1234);
    assert_eq!(field(1, 40, 4), 0);
}

#[test]
fn ring_indices_from_the_device_are_checked() {
    let r = RingIndex::new(8).unwrap();
    assert!(RingIndex::new(0).is_none());
    assert_eq!(r.checked(7), Ok(7));
    assert_eq!(r.checked(8), Err(Error::BadValue));
    assert_eq!(r.checked(u32::MAX), Err(Error::BadValue));
    assert_eq!(r.next(7), 0);
    assert_eq!(r.distance(6, 2), 4);
    assert_eq!(r.distance(2, 6), 4);
    assert_eq!(r.distance(3, 3), 0);
}

#[test]
fn dma_pool_aligns_and_never_overlaps() {
    let mut mem = vec![0xffu8; 1000];
    let mut pool = DmaPool::new(&mut mem, 0x1003);
    let a = pool.take(10, 16).unwrap();
    assert_eq!(a.bus % 16, 0);
    assert!(a.cpu.iter().all(|&b| b == 0), "zeroed");
    let b = pool.take(100, 64).unwrap();
    assert!(b.bus >= a.bus + 10 && b.bus % 64 == 0);
    assert!(pool.take(10_000, 1).is_none());
    assert!(pool.take(8, 3).is_none(), "alignment must be a power of two");
    assert_eq!(a.read_u32(8), None);
    assert_eq!(a.read_u16(8), Some(0));
    assert!(a.contains_bus(a.bus + 2, 8));
    assert!(!a.contains_bus(a.bus + 2, 9));
    assert!(!a.contains_bus(u64::MAX, 2));
}

#[test]
fn records_always_parse() {
    let mut out = Lines::new();
    Record::new("drv.ready").hex("id", 0x8086).num("n", 3).text("model", "Wi-Fi \"6E\" AX210").bytes("edid", &[0, 0xff]).emit(&mut out);
    assert_eq!(out.find("drv.ready").unwrap(), "[drv.ready] id=0x8086 n=3 model=\"Wi-Fi '6E' AX210\" edid=00ff");
    assert_eq!(out.field("drv.ready", "model").unwrap(), "Wi-Fi '6E' AX210");
    Record::new("Bad Tag").num("a b", 1).text("e", "").text("nl", "x\ny").emit(&mut out);
    assert_eq!(out.all()[1], "[harness.bad_tag] a_b=1 e=\"\" nl=\"x?y\"");
    let long = "x".repeat(1000);
    let r = Record::new("drv.long").text("v", &long);
    assert_eq!(r.as_str().len(), MAX_LINE);
    assert!(valid_tag("a.b_c.d1") && !valid_tag("a..b") && !valid_tag(".a") && !valid_tag("A"));
}

#[test]
fn irq_bits_only_when_unmasked() {
    let line = IrqLine::new();
    let mut irq = MockIrq::new(line.clone(), 2);
    line.raise(1);
    assert_eq!(irq.take(), 0, "masked: held at the device");
    assert_eq!(irq.unmask(5), Err(Error::Denied));
    irq.unmask(1).unwrap();
    assert_eq!(irq.take(), 0b10);
    assert_eq!(irq.take(), 0);
    let mut clock = MockClock::new();
    assert_eq!(wait_irq(&mut irq, &mut clock, 1, 20), Err(Error::Timeout));
    line.raise(1);
    irq.unmask(0).unwrap();
    line.raise(0);
    assert_eq!(wait_irq(&mut irq, &mut clock, 1, 20), Ok(0b11), "other bits come back too");
}

#[test]
fn mock_dma_faults_outside_the_region() {
    let mut dma = MockDma::new(64, 0x1000);
    let port = dma.port();
    let mut pool = dma.pool();
    let b = pool.take(64, 1).unwrap();
    assert!(port.write(0x1000 + 60, &[1, 2, 3, 4]));
    assert!(!port.write(0x1000 + 61, &[1, 2, 3, 4]));
    assert!(!port.read(0xfff, &mut [0]));
    assert_eq!(b.read_u32(60), Some(0x0403_0201));
    assert_eq!(port.faults(), 2);
}

#[test]
fn fuzz_regs_tail_and_pins() {
    let mut r = FuzzRegs::new(&[0x01, 0, 0, 0]).pin(8, 7);
    assert_eq!(r.read32(0), 1);
    assert_eq!(r.read32(0), u32::MAX, "past the input: fell off the bus");
    assert_eq!(r.read32(8), 7);
    let mut h = FuzzRegs::new(&[0x80]);
    h.read32(0);
    assert_eq!(h.read32(0), 0, "first byte's top bit: hung, reads zeros");
    let mut s = Src::new(&[3, 0, 9, 9, 9, 1]);
    assert_eq!(s.chunk(), &[9, 9, 9]);
    assert_eq!(s.rest(), &[1]);
}

fn panics_on_magic(d: &[u8]) { if d.windows(2).any(|w| w == [0xde, 0xad]) { panic!("magic") } }
fn spins_forever(_: &[u8]) { let mut r = FuzzRegs::new(&[]); loop { if r.read32(0) == 1 { break } } }
fn behaves(d: &[u8]) { let mut r = FuzzRegs::new(d); let _ = poll_until(&mut r, &mut MockClock::new(), 0, 1, 1, 5); }

#[test]
fn fuzz_runner_finds_panics_and_hangs() {
    std::panic::set_hook(Box::new(|_| {}));
    let found = fuzz::run(panics_on_magic, &[vec![0xde, 0xac]], 7, 200_000, 0, 64);
    let (input, why) = found.expect_err("the mutator finds 0xdead");
    assert!(input.windows(2).any(|w| w == [0xde, 0xad]));
    assert_eq!(why, "magic");
    let (_, why) = fuzz::run(spins_forever, &[], 1, 1, 0, 16).expect_err("hang");
    assert!(why.starts_with("DRIVER_HANG"), "{why}");
    let ok = fuzz::run(behaves, &[vec![1, 2, 3]], 1, 300, 0, 64).expect("bounded driver passes");
    assert_eq!(ok.inputs, 300);
    let _ = std::panic::take_hook();
}
