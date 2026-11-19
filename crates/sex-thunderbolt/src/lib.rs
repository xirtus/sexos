// SPDX-License-Identifier: GPL-3.0-only
//! Thunderbolt 4 / USB4 host driver core (stage T1 of the untethered-x17 plan).
//!
//! Intel's integrated Thunderbolt controllers (Ice Lake, Tiger Lake and later)
//! expose a Native Host Interface (NHI): DMA rings, one per "hop", through which
//! the host talks to the Thunderbolt fabric. Ring 0 carries control frames. On
//! these PCs a firmware connection manager (ICM) inside the controller sets up
//! the tunnels; the OS tells it the driver is ready, hears which devices plugged
//! in, and approves the ones the user trusts (security level "user").
//!
//! Written fresh from the USB4 and Thunderbolt 3 specifications, with Linux's
//! `drivers/thunderbolt` (GPL-2.0) read as a reference for register offsets and
//! message layouts. Nothing here touches hardware directly: the server maps the
//! NHI's registers and its DMA pool onto [`Nhi`].
#![no_std]

pub mod frame;
pub mod icm;
pub mod regs;
pub mod ring;

/// What the driver needs from the controller: its BAR0 registers and a DMA pool
/// (offsets are from the pool's start; the pool's bus address is in [`ring::Layout`]).
pub trait Nhi {
    fn read(&mut self, register: u32) -> u32;
    fn write(&mut self, register: u32, value: u32);
    fn load(&self, offset: u64, out: &mut [u8]);
    fn store(&mut self, offset: u64, bytes: &[u8]);
}

/// Intel PCI ids of NHIs that run a firmware connection manager this crate can
/// drive (Linux `icm_probe`: Ice Lake, Tiger Lake, Alder Lake, Raptor Lake, Meteor Lake).
pub const ICM_NHI: [(u16, &str); 13] = [
    (0x8a17, "Ice Lake NHI0"), (0x8a0d, "Ice Lake NHI1"),
    (0x9a1b, "Tiger Lake-LP NHI0"), (0x9a1d, "Tiger Lake-LP NHI1"),
    (0x9a1f, "Tiger Lake-H NHI0"), (0x9a21, "Tiger Lake-H NHI1"),
    (0x463e, "Alder Lake NHI0"), (0x466d, "Alder Lake NHI1"),
    (0xa73e, "Raptor Lake NHI0"), (0xa76d, "Raptor Lake NHI1"),
    (0x7eb2, "Meteor Lake-M NHI0"), (0x7ec2, "Meteor Lake-P NHI0"), (0x7ec3, "Meteor Lake-P NHI1"),
];

/// The name of an integrated Intel NHI, if it is one.
pub fn known(vendor: u16, device: u16) -> Option<&'static str> {
    if vendor != 0x8086 { return None; }
    ICM_NHI.iter().find(|(d, _)| *d == device).map(|(_, n)| *n)
}
