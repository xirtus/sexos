// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Classic Bluetooth HID (HID Profile 1.1): two L2CAP channels, control (PSM 0x11) and
//! interrupt (PSM 0x13). The host opens control first, then interrupt (a device that
//! reconnects opens them to us in the same order), then asks for the boot protocol, so
//! every keyboard and mouse sends the fixed boot reports: `A1 01` + keyboard report,
//! `A1 02` + mouse report, on the interrupt channel.
//!
//! Report protocol (reports laid out by the descriptor in the device's SDP record) is not
//! used: SDP is not implemented, and the boot protocol is mandatory for keyboards and
//! mice that claim it, which is what a desktop needs.

use crate::hid::{self, Input};

pub const PSM_CONTROL: u16 = 0x0011;
pub const PSM_INTERRUPT: u16 = 0x0013;

/// Transaction headers (type << 4 | parameter).
pub const HANDSHAKE: u8 = 0x00;
pub const HID_CONTROL: u8 = 0x10;
pub const SET_PROTOCOL_BOOT: u8 = 0x70;
pub const SET_PROTOCOL_REPORT: u8 = 0x71;
pub const DATA_INPUT: u8 = 0xA1;
pub const DATA_OUTPUT: u8 = 0xA2;
/// HID_CONTROL parameter: the device is unplugged (bond removed on the device).
pub const VIRTUAL_CABLE_UNPLUG: u8 = 0x05;

/// Boot report ids.
pub const BOOT_KEYBOARD: u8 = 1;
pub const BOOT_MOUSE: u8 = 2;

/// Decode one interrupt-channel message.
pub fn input(msg: &[u8]) -> Option<Input> {
    let (&header, rest) = msg.split_first()?;
    if header != DATA_INPUT { return None; }
    let (&id, report) = rest.split_first()?;
    match id {
        BOOT_KEYBOARD => hid::boot_keyboard(report),
        BOOT_MOUSE => hid::boot_mouse(report),
        _ => None,
    }
}

/// The keyboard LEDs (num, caps, scroll lock), sent on the interrupt channel.
pub fn leds(bits: u8) -> [u8; 3] { [DATA_OUTPUT, BOOT_KEYBOARD, bits] }

/// What a control-channel message means to the host.
#[derive(Debug, PartialEq, Eq)]
pub enum Control { Handshake(u8), Unplug, Other }

pub fn control(msg: &[u8]) -> Control {
    match msg.first() {
        Some(&h) if h >> 4 == HANDSHAKE >> 4 => Control::Handshake(h & 0x0F),
        Some(&h) if h == HID_CONTROL | VIRTUAL_CABLE_UNPLUG => Control::Unplug,
        _ => Control::Other,
    }
}
