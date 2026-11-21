// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Hot-plug logic shared by the kernel and the services: no registers are
//! touched here, so every rule is checked on the host.
//!
//! - [`pcie`]: what the PCI Express capability says about a port's slot.
//! - [`slot`]: one slot's life (settle, power, link, attention button, surprise
//!   removal) as a state machine fed with what the registers say now.
//! - [`plan`]: memory windows and BAR addresses for a tree of devices that just
//!   arrived below a port (an NVMe drive, a dock's switch, an eGPU enclosure).
#![no_std]

pub mod pcie;
pub mod plan;
pub mod slot;
