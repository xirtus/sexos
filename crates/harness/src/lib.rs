// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! # Harness: the SexOS driver SDK
//!
//! A SexOS driver is a Rust crate that runs inside a ring-3 driver service on a
//! device BOOT leased to that service. It never runs in the kernel, never sees a
//! Linux API, and never trusts what the device tells it. Harness is the small set
//! of pieces every such driver needs, so that the same driver source runs in three
//! places unchanged:
//!
//! | Where | Registers | Time | Interrupts | DMA |
//! |---|---|---|---|---|
//! | SexOS service (`sexos` feature) | [`sexos::LeasedRegs`] over the lease's BAR view | [`sexos::SystemClock`] | [`sexos::LeaseIrq`] (syscall 86) | [`sexos::dma_pool`] |
//! | Host unit tests (`mock` feature) | [`mock::ModelRegs`] over your device model | [`mock::MockClock`] | [`mock::MockIrq`] | [`mock::MockDma`] |
//! | Host fuzzing (`mock` feature) | [`fuzz::FuzzRegs`]: every read is hostile input | [`mock::MockClock`] | [`fuzz::FuzzIrq`] | [`mock::MockDma`] |
//!
//! ## The rules a driver keeps
//!
//! 1. **Every wait has a bound.** Use [`wait::poll_until`] and [`wait::wait_for`];
//!    they stop on a deadline *and* on a poll count, so a clock that never moves
//!    cannot hang the service either. A device that never answers is an `Err`.
//! 2. **Every device value is untrusted.** Ring indices go through
//!    [`dma::RingIndex::checked`], lengths through [`dma::DmaBuf`]'s checked
//!    accessors, and an all-ones read means the device fell off the bus
//!    ([`regs::is_gone`]). A driver never panics on what a device says.
//! 3. **Report in records.** One `[tag] key=value` line per fact, through
//!    [`record::Record`]. These lines are what Spindle `hw save` collects on a
//!    real machine and what `scripts/hw_report.py` and the hardware list read.
//! 4. **No ambient authority.** The driver gets only what its service hands it:
//!    a register window, DMA memory, interrupt bits, a clock. Which devices a
//!    service gets is data: its manifest in `sex_hwdb::manifest` (re-exported as
//!    [`manifest`]).
//!
//! See `docs/sdk/DRIVER_SDK.md` for the walkthrough and
//! `crates/harness-template` for a complete driver to copy.
#![cfg_attr(not(feature = "std"), no_std)]

pub mod dma;
pub mod error;
pub mod irq;
pub mod record;
pub mod regs;
pub mod wait;

#[cfg(feature = "mock")]
pub mod fuzz;
#[cfg(feature = "mock")]
pub mod mock;
#[cfg(feature = "sexos")]
pub mod sexos;

pub use dma::{DmaBuf, DmaPool, RingIndex};
pub use error::Error;
pub use irq::{Irq, NoIrq};
pub use record::{Record, Sink};
pub use regs::{Regs, GONE};
pub use wait::Clock;

/// Device identity and driver manifests: which devices a service asks BOOT to
/// lease to it (`Match`, `Manifest`, `Lease` kinds) and the census table names.
pub mod manifest {
    pub use sex_hwdb::manifest::{Lease, Manifest, Match};
    pub use sex_hwdb::{class_name, describe, vendor_name, Device, Id, Service, Support};
}

/// One kind of device a driver drives: an exact id, or a PCI class
/// (`class << 16 | subclass << 8 | prog_if`, compared under `class_mask`);
/// a `vendor` or `device` of 0 matches any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceMatch { pub vendor: u16, pub device: u16, pub class: u32, pub class_mask: u32 }

impl DeviceMatch {
    pub const fn id(vendor: u16, device: u16) -> Self { DeviceMatch { vendor, device, class: 0, class_mask: 0 } }
    pub const fn class(class: u32, class_mask: u32) -> Self { DeviceMatch { vendor: 0, device: 0, class, class_mask } }
    pub fn matches(&self, id: sex_hwdb::Id) -> bool {
        let class = (id.class as u32) << 16 | (id.subclass as u32) << 8 | id.prog_if as u32;
        class & self.class_mask == self.class & self.class_mask
            && (self.vendor == 0 || self.vendor == id.vendor)
            && (self.device == 0 || self.device == id.device)
    }
}

/// What a driver tells the SDK tools about itself. `tags` are the record tags
/// it emits, so a hardware report line can be traced back to its driver.
#[derive(Clone, Copy, Debug)]
pub struct DriverInfo {
    /// Short name, used as the record tag prefix (`[NAME.ready]`).
    pub name: &'static str,
    /// The service the driver runs in.
    pub service: sex_hwdb::Service,
    /// Devices it drives.
    pub matches: &'static [DeviceMatch],
    /// Record tags it emits.
    pub tags: &'static [&'static str],
}

impl DriverInfo {
    /// Whether this driver claims a device.
    pub fn claims(&self, id: sex_hwdb::Id) -> bool { self.matches.iter().any(|m| m.matches(id)) }
    /// Whether every tag is valid and starts with the driver's name.
    pub fn tags_ok(&self) -> bool {
        record::valid_tag(self.name)
            && self.tags.iter().all(|t| record::valid_tag(t) && t.strip_prefix(self.name).is_some_and(|r| r.starts_with('.')))
    }
}
