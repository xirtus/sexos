// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Interrupts as SexOS delivers them: not handlers but pending bits. When a
//! leased device raises one of its vectors, the kernel sets that vector's bit
//! and wakes the service; the driver takes the bits when it next looks. A driver
//! that never unmasks a vector simply polls.

use crate::error::Error;
use crate::wait::{wait_for, Clock};

pub trait Irq {
    /// Let vector `vector` (0-based, of this device) interrupt.
    fn unmask(&mut self, vector: u8) -> Result<(), Error>;
    /// Hold vector `vector` at the device.
    fn mask(&mut self, vector: u8) -> Result<(), Error>;
    /// The pending vector bits of this device since the last take (bit `i` is
    /// vector `i`), cleared.
    fn take(&mut self) -> u64;
}

impl<I: Irq + ?Sized> Irq for &mut I {
    fn unmask(&mut self, vector: u8) -> Result<(), Error> { (**self).unmask(vector) }
    fn mask(&mut self, vector: u8) -> Result<(), Error> { (**self).mask(vector) }
    fn take(&mut self) -> u64 { (**self).take() }
}

/// A device without interrupts (or a driver that polls): unmask is refused and
/// nothing is ever pending.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoIrq;
impl Irq for NoIrq {
    fn unmask(&mut self, _vector: u8) -> Result<(), Error> { Err(Error::Unsupported) }
    fn mask(&mut self, _vector: u8) -> Result<(), Error> { Ok(()) }
    fn take(&mut self) -> u64 { 0 }
}

/// Waits until one of `bits` is pending (and returns all bits taken), or the
/// deadline passes. Bits outside `bits` that were taken are returned too, so
/// the caller can handle them instead of losing them.
pub fn wait_irq<I: Irq, C: Clock>(irq: &mut I, clock: &mut C, bits: u64, timeout_ms: u64) -> Result<u64, Error> {
    let mut seen = 0u64;
    wait_for(clock, timeout_ms, || {
        seen |= irq.take();
        (seen & bits != 0).then_some(seen)
    })
}
