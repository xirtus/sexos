// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Host test kit: a device model behind [`Regs`], a clock that only moves when
//! the driver looks at it, interrupt bits the model raises, DMA memory the model
//! reads and writes by bus address, and a sink that keeps record lines.
//!
//! ```
//! use harness::mock::{DeviceModel, ModelRegs};
//! use harness::Regs;
//! struct Echo(u32);
//! impl DeviceModel for Echo {
//!     fn read32(&mut self, offset: u32) -> u32 { if offset == 0 { 0xed } else { self.0 } }
//!     fn write32(&mut self, _offset: u32, value: u32) { self.0 = !value; }
//! }
//! let mut regs = ModelRegs::new(Echo(0));
//! regs.write32(4, 0x1234);
//! assert_eq!(regs.read32(4), !0x1234);
//! assert_eq!(regs.writes_to(4), vec![0x1234]);
//! assert_eq!(regs.model().0, !0x1234);
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::dma::DmaPool;
use crate::error::Error;
use crate::irq::Irq;
use crate::record::Sink;
use crate::regs::Regs;
use crate::wait::Clock;

/// A simulated device: what each register read returns and what each write does.
/// `step` runs once per register read, so work the device does "in the
/// background" (a computation, a DMA transfer) can finish after some reads.
pub trait DeviceModel {
    fn read32(&mut self, offset: u32) -> u32;
    fn write32(&mut self, offset: u32, value: u32);
    fn step(&mut self) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access { Read(u32, u32), Write(u32, u32) }

/// A [`Regs`] over a [`DeviceModel`], keeping every access in order. The model
/// is shared (`Rc<RefCell<_>>`) so the test, and a [`MockClock`] that lets the
/// device work while the driver sleeps, can reach it too.
pub struct ModelRegs<M: DeviceModel> {
    model: Rc<RefCell<M>>,
    pub log: Vec<Access>,
    /// Keep the log (on by default; turn off for long runs).
    pub logging: bool,
}

impl<M: DeviceModel> ModelRegs<M> {
    pub fn new(model: M) -> Self { ModelRegs { model: Rc::new(RefCell::new(model)), log: Vec::new(), logging: true } }
    /// The model, to look at or change between driver calls.
    pub fn model(&self) -> std::cell::RefMut<'_, M> { self.model.borrow_mut() }
    /// Another handle on the same model (for [`MockClock::drive`]).
    pub fn handle(&self) -> Rc<RefCell<M>> { self.model.clone() }
    pub fn reads(&self) -> usize { self.log.iter().filter(|a| matches!(a, Access::Read(..))).count() }
    /// Values written to `offset`, in order.
    pub fn writes_to(&self, offset: u32) -> Vec<u32> {
        self.log.iter().filter_map(|a| match *a { Access::Write(o, v) if o == offset => Some(v), _ => None }).collect()
    }
}

impl<M: DeviceModel> Regs for ModelRegs<M> {
    fn read32(&mut self, offset: u32) -> u32 {
        crate::fuzz::tick();
        let v = { let mut m = self.model.borrow_mut(); m.step(); m.read32(offset) };
        if self.logging { self.log.push(Access::Read(offset, v)); }
        v
    }
    fn write32(&mut self, offset: u32, value: u32) {
        if self.logging { self.log.push(Access::Write(offset, value)); }
        self.model.borrow_mut().write32(offset, value);
    }
}

/// Time that moves only when the driver sleeps (or by `step` per look), so
/// tests are deterministic and a timeout path runs in microseconds. Models
/// given to [`MockClock::drive`] take one step per millisecond slept, so a
/// device finishes its work while an interrupt-driven driver waits.
#[derive(Default)]
pub struct MockClock { pub now: u64, pub step: u64, pub sleeps: u64, devices: Vec<Rc<RefCell<dyn DeviceModel>>> }
impl MockClock {
    pub fn new() -> Self { MockClock::default() }
    /// A clock that also advances `step` ms every time it is read.
    pub fn stepping(step: u64) -> Self { MockClock { step, ..Default::default() } }
    /// Let `model` work while the driver sleeps (see [`ModelRegs::handle`]).
    pub fn drive<M: DeviceModel + 'static>(mut self, model: Rc<RefCell<M>>) -> Self { self.devices.push(model); self }
}
impl Clock for MockClock {
    fn now_ms(&mut self) -> u64 {
        crate::fuzz::tick();
        let t = self.now;
        self.now = self.now.saturating_add(self.step);
        t
    }
    fn sleep_ms(&mut self, ms: u64) {
        self.sleeps += 1;
        self.now = self.now.saturating_add(ms);
        for d in &self.devices { for _ in 0..ms.clamp(1, 1000) { d.borrow_mut().step(); } }
    }
}

/// The interrupt wire between a model and [`MockIrq`]: the model raises vector
/// bits, the driver takes them (only those it unmasked).
#[derive(Clone, Default)]
pub struct IrqLine(Rc<Cell<u64>>);
impl IrqLine {
    pub fn new() -> Self { IrqLine::default() }
    pub fn raise(&self, vector: u8) { if vector < 64 { self.0.set(self.0.get() | 1 << vector); } }
    pub fn pending(&self) -> u64 { self.0.get() }
}

pub struct MockIrq { pub line: IrqLine, pub unmasked: u64, pub vectors: u8 }
impl MockIrq {
    /// A device with `vectors` vectors, all masked.
    pub fn new(line: IrqLine, vectors: u8) -> Self { MockIrq { line, unmasked: 0, vectors } }
}
impl Irq for MockIrq {
    fn unmask(&mut self, vector: u8) -> Result<(), Error> {
        if vector >= self.vectors { return Err(Error::Denied); }
        self.unmasked |= 1 << vector;
        Ok(())
    }
    fn mask(&mut self, vector: u8) -> Result<(), Error> {
        if vector >= self.vectors { return Err(Error::Denied); }
        self.unmasked &= !(1 << vector);
        Ok(())
    }
    fn take(&mut self) -> u64 {
        let p = self.line.0.get();
        let got = p & self.unmasked;
        self.line.0.set(p & !got);
        got
    }
}

struct Memory { ptr: *mut u8, len: usize }
impl Drop for Memory {
    fn drop(&mut self) {
        // SAFETY: ptr/len came from Box::<[u8]>::into_raw in MockDma::new.
        unsafe { drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.ptr, self.len))) }
    }
}

/// DMA memory shared by the driver (through [`MockDma::pool`]) and the model
/// (through a [`BusPort`]), as host RAM is shared with a real device. Like a
/// real device, the model writes these bytes behind the driver's back; tests
/// only look at them between driver calls. A model access outside the region
/// is an IOMMU fault: refused and counted.
pub struct MockDma { mem: Rc<Memory>, bus: u64, faults: Rc<Cell<u32>> }

impl MockDma {
    /// `len` bytes the device sees at bus addresses `bus..bus + len`.
    pub fn new(len: usize, bus: u64) -> Self {
        let b = vec![0u8; len].into_boxed_slice();
        let len = b.len();
        let ptr = Box::into_raw(b) as *mut u8;
        MockDma { mem: Rc::new(Memory { ptr, len }), bus, faults: Rc::new(Cell::new(0)) }
    }
    pub fn bus(&self) -> u64 { self.bus }
    pub fn len(&self) -> usize { self.mem.len }
    pub fn is_empty(&self) -> bool { self.mem.len == 0 }
    /// The device's side.
    pub fn port(&self) -> BusPort { BusPort { mem: self.mem.clone(), bus: self.bus, faults: self.faults.clone() } }
    /// The driver's side: the whole region as a pool.
    pub fn pool(&mut self) -> DmaPool<'_> {
        // SAFETY: the region lives while `self` (and every BusPort) does; the
        // `&mut self` borrow keeps two pools from existing at once.
        let s = unsafe { std::slice::from_raw_parts_mut(self.mem.ptr, self.mem.len) };
        DmaPool::new(s, self.bus)
    }
    /// Model accesses refused because they fell outside the region.
    pub fn faults(&self) -> u32 { self.faults.get() }
}

/// A device's view of [`MockDma`]: reads and writes by bus address.
#[derive(Clone)]
pub struct BusPort { mem: Rc<Memory>, bus: u64, faults: Rc<Cell<u32>> }
impl BusPort {
    fn at(&self, bus: u64, len: usize) -> Option<usize> {
        let off = bus.checked_sub(self.bus)? as usize;
        (off.checked_add(len)? <= self.mem.len).then_some(off)
    }
    /// Device reads host memory; false (and a fault) outside the region.
    pub fn read(&self, bus: u64, out: &mut [u8]) -> bool {
        let Some(off) = self.at(bus, out.len()) else { self.faults.set(self.faults.get() + 1); return false };
        // SAFETY: in bounds (checked); the memory is alive while the Rc is.
        unsafe { std::ptr::copy(self.mem.ptr.add(off), out.as_mut_ptr(), out.len()) }
        true
    }
    /// Device writes host memory; false (and a fault) outside the region.
    pub fn write(&self, bus: u64, data: &[u8]) -> bool {
        let Some(off) = self.at(bus, data.len()) else { self.faults.set(self.faults.get() + 1); return false };
        // SAFETY: as above.
        unsafe { std::ptr::copy(data.as_ptr(), self.mem.ptr.add(off), data.len()) }
        true
    }
    pub fn faults(&self) -> u32 { self.faults.get() }
}

/// Keeps record lines; `field("edu.ready", "id")` finds a value in them.
#[derive(Clone, Default)]
pub struct Lines(pub Rc<RefCell<Vec<String>>>);
impl Lines {
    pub fn new() -> Self { Lines::default() }
    pub fn all(&self) -> Vec<String> { self.0.borrow().clone() }
    /// The first line with exactly this tag.
    pub fn find(&self, tag: &str) -> Option<String> {
        let open = format!("[{tag}]");
        self.0.borrow().iter().find(|l| l.starts_with(&open)).cloned()
    }
    /// A field of the first line with this tag (quotes removed).
    pub fn field(&self, tag: &str, key: &str) -> Option<String> {
        let line = self.find(tag)?;
        let rest = &line[tag.len() + 2..];
        let pat = format!(" {key}=");
        let at = rest.find(&pat)? + pat.len();
        let v = &rest[at..];
        Some(if let Some(q) = v.strip_prefix('"') { q.split('"').next().unwrap_or("").to_string() } else { v.split(' ').next().unwrap_or("").to_string() })
    }
}
impl Sink for Lines {
    fn line(&mut self, line: &str) { self.0.borrow_mut().push(line.to_string()) }
}
