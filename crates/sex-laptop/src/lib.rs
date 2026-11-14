// SPDX-License-Identifier: GPL-3.0-only
//! The laptop half of power: what the firmware's numbers mean, without the firmware.
//!
//! `servers/sexacpi` runs the AML; this crate holds the parts that can be tested on the
//! host: the ACPI embedded controller protocol (over any port pair), battery arithmetic
//! (`_BST`/`_BIX` to a percentage, time left and wear), backlight steps from `_BCL`, thermal
//! trip checks, `_CRS` resource parsing, and CPU power profiles.
#![cfg_attr(not(test), no_std)]

pub mod backlight;
pub mod battery;
pub mod ec;
pub mod profile;
pub mod resources;
pub mod thermal;
