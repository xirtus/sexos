// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! HID over I2C, version 1.00.

/// The 30-byte HID descriptor read from the register firmware names in `_DSM`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HidDescriptor {
    pub bcd_version: u16,
    pub report_desc_length: u16,
    pub report_desc_register: u16,
    pub input_register: u16,
    pub max_input_length: u16,
    pub output_register: u16,
    pub max_output_length: u16,
    pub command_register: u16,
    pub data_register: u16,
    pub vendor_id: u16,
    pub product_id: u16,
    pub version_id: u16,
}

pub const DESCRIPTOR_LEN: usize = 30;

impl HidDescriptor {
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < DESCRIPTOR_LEN { return None; }
        let w = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        if w(0) as usize != DESCRIPTOR_LEN || w(2) != 0x0100 { return None; }
        let d = Self {
            bcd_version: w(2), report_desc_length: w(4), report_desc_register: w(6), input_register: w(8),
            max_input_length: w(10), output_register: w(12), max_output_length: w(14), command_register: w(16),
            data_register: w(18), vendor_id: w(20), product_id: w(22), version_id: w(24),
        };
        (d.max_input_length >= 2 && d.report_desc_length > 0).then_some(d)
    }
}

/// Command opcodes (high nibble of the second command byte).
pub const OP_RESET: u8 = 0x1;
pub const OP_GET_REPORT: u8 = 0x2;
pub const OP_SET_REPORT: u8 = 0x3;
pub const OP_SET_POWER: u8 = 0x8;
/// Report types.
pub const REPORT_INPUT: u8 = 1;
pub const REPORT_FEATURE: u8 = 3;

/// Bytes to write for a command: command register, then the command word. Report ids of
/// 15 and above take the escape form (0xF in the id nibble, id in a third byte).
pub fn command(register: u16, opcode: u8, report_type: u8, report_id: u8, out: &mut [u8; 5]) -> usize {
    out[0..2].copy_from_slice(&register.to_le_bytes());
    let id_nibble = if report_id >= 0xF { 0xF } else { report_id };
    out[2] = (report_type & 3) << 4 | id_nibble;
    out[3] = opcode & 0xF;
    if report_id >= 0xF { out[4] = report_id; 5 } else { 4 }
}

/// `SET_POWER`: on (0) or sleep (1).
pub fn set_power(register: u16, on: bool, out: &mut [u8; 5]) -> usize {
    let n = command(register, OP_SET_POWER, 0, if on { 0 } else { 1 }, out);
    out[2] = if on { 0 } else { 1 };
    n
}

/// `SET_REPORT` of a feature report through the data register: the command, then the data
/// register, the length (2 + payload) and the payload (report id first).
pub fn set_feature(command_register: u16, data_register: u16, report_id: u8, payload: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut c = [0u8; 5];
    let n = command(command_register, OP_SET_REPORT, REPORT_FEATURE, report_id, &mut c);
    let total = n + 2 + 2 + 1 + payload.len();
    if out.len() < total { return None; }
    out[..n].copy_from_slice(&c[..n]);
    out[n..n + 2].copy_from_slice(&data_register.to_le_bytes());
    out[n + 2..n + 4].copy_from_slice(&((3 + payload.len()) as u16).to_le_bytes());
    out[n + 4] = report_id;
    out[n + 5..total].copy_from_slice(payload);
    Some(total)
}

/// An input report as read from the input register: two length bytes (the length counts
/// itself), then the report. A length of 0 (or 2) means nothing new — after a reset, or
/// when polled without an interrupt.
pub fn input_payload(raw: &[u8]) -> Option<&[u8]> {
    if raw.len() < 2 { return None; }
    let len = u16::from_le_bytes([raw[0], raw[1]]) as usize;
    if len <= 2 { return None; }
    raw.get(2..len.min(raw.len()))
}

/// The `_DSM` that names the HID descriptor register (UUID 3cdff6f7-4267-4555-ad05-b30a3d8938de).
pub const DSM_UUID: [u8; 16] = [0xF7, 0xF6, 0xDF, 0x3C, 0x67, 0x42, 0x55, 0x45, 0xAD, 0x05, 0xB3, 0x0A, 0x3D, 0x89, 0x38, 0xDE];

#[cfg(test)]
mod tests {
    use super::*;

    fn elan_descriptor() -> [u8; 30] {
        // A typical touchpad: report descriptor 0x2B2 bytes at register 2, input at 3,
        // command 5, data 6, vendor 04F3 (ELAN).
        let mut d = [0u8; 30];
        let words: [u16; 13] = [30, 0x0100, 0x02B2, 0x0002, 0x0003, 0x0020, 0x0004, 0x0011, 0x0005, 0x0006, 0x04F3, 0x3195, 0x0001];
        for (i, w) in words.iter().enumerate() { d[i * 2..i * 2 + 2].copy_from_slice(&w.to_le_bytes()); }
        d
    }

    #[test]
    fn descriptor_parses_and_rejects_garbage() {
        let d = HidDescriptor::parse(&elan_descriptor()).unwrap();
        assert_eq!((d.report_desc_length, d.input_register, d.command_register, d.data_register, d.vendor_id), (0x2B2, 3, 5, 6, 0x04F3));
        assert_eq!(d.max_input_length, 0x20);
        let mut bad = elan_descriptor();
        bad[2] = 0x10;
        assert_eq!(HidDescriptor::parse(&bad), None);
        assert_eq!(HidDescriptor::parse(&[0xFF; 30]), None);
        assert_eq!(HidDescriptor::parse(&[30, 0]), None);
    }

    #[test]
    fn commands_encode() {
        let mut out = [0u8; 5];
        assert_eq!(command(5, OP_RESET, 0, 0, &mut out), 4);
        assert_eq!(&out[..4], &[5, 0, 0x00, 0x01]);
        let n = set_power(5, true, &mut out);
        assert_eq!(&out[..n], &[5, 0, 0x00, 0x08]);
        let n = set_power(5, false, &mut out);
        assert_eq!(&out[..n], &[5, 0, 0x01, 0x08]);
        assert_eq!(command(5, OP_GET_REPORT, REPORT_FEATURE, 0x12, &mut out), 5);
        assert_eq!(&out, &[5, 0, 0x3F, 0x02, 0x12]);
    }

    #[test]
    fn input_mode_feature_report() {
        // Precision Touchpad: Input Mode (report 3) = 3 switches from mouse to touchpad.
        let mut out = [0u8; 16];
        let n = set_feature(5, 6, 3, &[3], &mut out).unwrap();
        assert_eq!(&out[..n], &[5, 0, 0x33, 0x03, 6, 0, 4, 0, 3, 3]);
    }

    #[test]
    fn input_reports_frame() {
        assert_eq!(input_payload(&[0, 0, 9, 9]), None);
        assert_eq!(input_payload(&[2, 0]), None);
        assert_eq!(input_payload(&[5, 0, 1, 2, 3, 0xEE]), Some(&[1u8, 2, 3][..]));
        assert_eq!(input_payload(&[0x40, 0, 1, 2]), Some(&[1u8, 2][..]));
    }
}
