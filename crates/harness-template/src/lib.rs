// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A complete SexOS driver, for copying: QEMU's `edu` teaching device
//! (PCI 1234:11e8, `qemu-system-x86_64 -device edu`).
//!
//! The device is small but has everything a real one has: an identity
//! register, a liveness check, a slow operation that finishes later (a
//! factorial), an interrupt, and DMA in both directions. The driver below shows
//! the Harness way to do each:
//!
//! * it is generic over [`Regs`], [`Clock`], [`Irq`] and [`Sink`], and owns no
//!   syscalls, so the same code runs in a SexOS service and in host tests;
//! * every wait is [`wait::poll_until`] or [`irq::wait_irq`] with a timeout;
//! * every device value is checked (identity, liveness, all-ones) and every
//!   request is checked against the device's limits before it reaches the device;
//! * it reports in `[edu.*]` record lines.
//!
//! Register map (QEMU `docs/specs/edu.rst`): 0x00 identification `0xRRrr00ed`,
//! 0x04 liveness (reads back the inverse of what was written), 0x08 factorial,
//! 0x20 status (bit 0 computing, bit 7 raise an interrupt when done), 0x24
//! interrupt status, 0x60 raise, 0x64 acknowledge, 0x80/0x88/0x90/0x98 DMA source,
//! destination, count and command. The DMA buffer is 4 KiB at device address
//! 0x40000, and the device masks bus addresses to 28 bits.
#![no_std]

use harness::dma::DmaBuf;
use harness::irq::{wait_irq, Irq};
use harness::manifest::Service;
use harness::record::{Record, Sink};
use harness::regs::{is_gone, Regs};
use harness::wait::{poll_until, Clock};
use harness::{DeviceMatch, DriverInfo, Error};

pub const VENDOR: u16 = 0x1234;
pub const DEVICE: u16 = 0x11e8;

pub const ID: u32 = 0x00;
pub const LIVENESS: u32 = 0x04;
pub const FACTORIAL: u32 = 0x08;
pub const STATUS: u32 = 0x20;
pub const IRQ_STATUS: u32 = 0x24;
pub const IRQ_RAISE: u32 = 0x60;
pub const IRQ_ACK: u32 = 0x64;
pub const DMA_SRC: u32 = 0x80;
pub const DMA_DST: u32 = 0x88;
pub const DMA_COUNT: u32 = 0x90;
pub const DMA_CMD: u32 = 0x98;

pub const STATUS_COMPUTING: u32 = 0x01;
pub const STATUS_IRQ_ON_FACTORIAL: u32 = 0x80;
pub const DMA_START: u32 = 0x01;
/// Direction bit: set = device buffer to RAM, clear = RAM to device buffer.
pub const DMA_TO_RAM: u32 = 0x02;
pub const DMA_IRQ: u32 = 0x04;
/// Interrupt status values the device raises.
pub const IRQ_FACTORIAL: u32 = 0x001;
pub const IRQ_DMA: u32 = 0x100;

pub const BUFFER_ADDR: u32 = 0x40000;
pub const BUFFER_LEN: u32 = 4096;
/// The device masks DMA addresses to 28 bits.
pub const DMA_LIMIT: u64 = 1 << 28;

/// The largest n whose factorial fits the 32-bit register.
pub const MAX_FACTORIAL: u32 = 12;

pub const TIMEOUT_MS: u64 = 100;

pub const INFO: DriverInfo = DriverInfo {
    name: "edu",
    // A teaching device with no system service of its own yet; a real driver
    // names the service it runs in (Drive, Net, Bell, Usb, Display, Input).
    service: Service::Nothing,
    matches: &[DeviceMatch::id(VENDOR, DEVICE)],
    tags: &["edu.ready", "edu.refused", "edu.factorial", "edu.dma", "edu.irq"],
};

/// Which direction a DMA transfer goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { ToDevice, ToRam }

/// How the driver learns an operation finished.
pub enum Wait<'a, I: Irq> {
    /// Poll the status register.
    Poll,
    /// Sleep until the interrupt bit (vector 0) arrives, then confirm by polling.
    Interrupt(&'a mut I),
}

pub struct Edu<R: Regs> {
    regs: R,
    pub major: u8,
    pub minor: u8,
}

impl<R: Regs> Edu<R> {
    /// Checks this is an edu device and that it answers, and reports it.
    pub fn start<C: Clock, S: Sink>(mut regs: R, clock: &mut C, out: &mut S) -> Result<Self, Error> {
        let _ = clock;
        let refuse = |out: &mut S, e: Error, id: u32| {
            Record::new("edu.refused").text("reason", e.name()).hex("id", id as u64).emit(out);
            Err(e)
        };
        let id = regs.read32(ID);
        if is_gone(id) { return refuse(out, Error::Gone, id); }
        if id & 0xffff != 0x00ed { return refuse(out, Error::NotThisDevice, id); }
        // Liveness: the device reads back the inverse of what was written. Two
        // patterns, so a register stuck at one value cannot pass.
        for probe in [0x5a5a_a5a5u32, 0x0f0f_f0f0] {
            regs.write32(LIVENESS, probe);
            let back = regs.read32(LIVENESS);
            if back != !probe { return refuse(out, if is_gone(back) { Error::Gone } else { Error::BadValue }, id); }
        }
        let (major, minor) = ((id >> 24) as u8, (id >> 16) as u8);
        Record::new("edu.ready").text("version", &version(major, minor)).hex("id", id as u64).emit(out);
        Ok(Edu { regs, major, minor })
    }

    /// n! computed by the device (n <= [`MAX_FACTORIAL`]).
    pub fn factorial<C: Clock, I: Irq, S: Sink>(&mut self, n: u32, clock: &mut C, mut wait: Wait<'_, I>, out: &mut S) -> Result<u32, Error> {
        if n > MAX_FACTORIAL { return Err(Error::BadRequest); }
        let by_irq = matches!(wait, Wait::Interrupt(_));
        self.regs.write32(STATUS, if by_irq { STATUS_IRQ_ON_FACTORIAL } else { 0 });
        self.regs.write32(FACTORIAL, n);
        if let Wait::Interrupt(irq) = &mut wait {
            irq.unmask(0)?;
            let got = wait_irq(&mut **irq, clock, 1, TIMEOUT_MS);
            self.acknowledge(out);
            got?;
        }
        poll_until(&mut self.regs, clock, STATUS, STATUS_COMPUTING, 0, TIMEOUT_MS).inspect_err(|e| {
            Record::new("edu.factorial").num("n", n as u64).text("error", e.name()).emit(out);
        })?;
        let v = self.regs.read32(FACTORIAL);
        Record::new("edu.factorial").num("n", n as u64).num("value", v as u64).flag("irq", by_irq).emit(out);
        Ok(v)
    }

    /// Copies `len` bytes between `buf` (from its start) and the device buffer
    /// at `device_offset`.
    pub fn dma<C: Clock, I: Irq, S: Sink>(&mut self, dir: Direction, buf: &DmaBuf<'_>, device_offset: u32, len: u32, clock: &mut C, mut wait: Wait<'_, I>, out: &mut S) -> Result<(), Error> {
        // Everything the device will use is checked before it sees it.
        if len == 0 || len as usize > buf.len() { return Err(Error::BadRequest); }
        let dev_end = device_offset.checked_add(len).ok_or(Error::BadRequest)?;
        if dev_end > BUFFER_LEN { return Err(Error::BadRequest); }
        let ram = buf.bus;
        if ram.checked_add(len as u64).map_or(true, |e| e > DMA_LIMIT) { return Err(Error::BadRequest); }
        let dev = (BUFFER_ADDR + device_offset) as u64;
        let (src, dst) = match dir { Direction::ToDevice => (ram, dev), Direction::ToRam => (dev, ram) };
        // edu takes its 64-bit DMA registers in one access or as the low half
        // only; both addresses are under 2^28, so the low half is all of it.
        self.regs.write32(DMA_SRC, src as u32);
        self.regs.write32(DMA_DST, dst as u32);
        self.regs.write32(DMA_COUNT, len);
        let by_irq = matches!(wait, Wait::Interrupt(_));
        let cmd = DMA_START | if dir == Direction::ToRam { DMA_TO_RAM } else { 0 } | if by_irq { DMA_IRQ } else { 0 };
        self.regs.write32(DMA_CMD, cmd);
        if let Wait::Interrupt(irq) = &mut wait {
            irq.unmask(0)?;
            let got = wait_irq(&mut **irq, clock, 1, TIMEOUT_MS);
            self.acknowledge(out);
            got?;
        }
        let r = poll_until(&mut self.regs, clock, DMA_CMD, DMA_START, 0, TIMEOUT_MS);
        let rec = Record::new("edu.dma").text("dir", if dir == Direction::ToRam { "to_ram" } else { "to_device" }).num("len", len as u64).flag("irq", by_irq);
        match r {
            Ok(_) => { rec.emit(out); Ok(()) }
            Err(e) => { rec.text("error", e.name()).emit(out); Err(e) }
        }
    }

    /// Reads and acknowledges the interrupt status; returns what was pending.
    pub fn acknowledge<S: Sink>(&mut self, out: &mut S) -> u32 {
        let v = self.regs.read32(IRQ_STATUS);
        if is_gone(v) || v == 0 { return 0; }
        self.regs.write32(IRQ_ACK, v);
        Record::new("edu.irq").hex("status", v as u64).emit(out);
        v
    }

    /// Asks the device to raise an interrupt with `value` (a self test).
    pub fn raise(&mut self, value: u32) { self.regs.write32(IRQ_RAISE, value) }

    pub fn into_regs(self) -> R { self.regs }
}

fn version(major: u8, minor: u8) -> heapless_version::V { heapless_version::V::new(major, minor) }

/// "1.0" without allocation.
mod heapless_version {
    pub struct V { b: [u8; 7], n: usize }
    impl V {
        pub fn new(major: u8, minor: u8) -> V {
            let mut v = V { b: [0; 7], n: 0 };
            v.push(major); v.b[v.n] = b'.'; v.n += 1; v.push(minor);
            v
        }
        fn push(&mut self, x: u8) {
            if x >= 100 { self.b[self.n] = b'0' + x / 100; self.n += 1; }
            if x >= 10 { self.b[self.n] = b'0' + x / 10 % 10; self.n += 1; }
            self.b[self.n] = b'0' + x % 10; self.n += 1;
        }
    }
    impl core::ops::Deref for V {
        type Target = str;
        fn deref(&self) -> &str { core::str::from_utf8(&self.b[..self.n]).unwrap_or("?") }
    }
}
