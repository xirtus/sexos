// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Laptop touchpads over I2C HID, without the bus.
//!
//! - `i2c_hid`: the HID-over-I2C protocol (Microsoft's 1.0 specification): the HID
//!   descriptor, command and register encodings.
//! - `descriptor`: a HID report descriptor parser that finds a Windows Precision
//!   Touchpad's contact fields (tip, contact id, X, Y), the contact count and buttons.
//! - `touch`: turns input reports into a frame of contacts.
//! - `gesture`: one finger moves the pointer, two scroll, taps click, three-finger
//!   swipes change workspace, pinches zoom.
//!
//! The bus driver (the DesignWare I2C controller) feeds bytes in; pointer events come out.
#![cfg_attr(not(test), no_std)]

pub mod descriptor;
pub mod gesture;
pub mod i2c_hid;
pub mod session;
pub mod touch;
