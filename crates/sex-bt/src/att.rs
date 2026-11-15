// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The Attribute Protocol (Core 5.4, Vol 3 Part F), client side: the requests GATT
//! discovery and HID over GATT need, and their responses, byte-exact.

use alloc::vec::Vec;

pub mod op {
    pub const ERROR: u8 = 0x01;
    pub const EXCHANGE_MTU_REQ: u8 = 0x02;
    pub const EXCHANGE_MTU_RSP: u8 = 0x03;
    pub const FIND_INFORMATION_REQ: u8 = 0x04;
    pub const FIND_INFORMATION_RSP: u8 = 0x05;
    pub const READ_BY_TYPE_REQ: u8 = 0x08;
    pub const READ_BY_TYPE_RSP: u8 = 0x09;
    pub const READ_REQ: u8 = 0x0A;
    pub const READ_RSP: u8 = 0x0B;
    pub const READ_BLOB_REQ: u8 = 0x0C;
    pub const READ_BLOB_RSP: u8 = 0x0D;
    pub const READ_BY_GROUP_TYPE_REQ: u8 = 0x10;
    pub const READ_BY_GROUP_TYPE_RSP: u8 = 0x11;
    pub const WRITE_REQ: u8 = 0x12;
    pub const WRITE_RSP: u8 = 0x13;
    pub const WRITE_CMD: u8 = 0x52;
    pub const NOTIFICATION: u8 = 0x1B;
    pub const INDICATION: u8 = 0x1D;
    pub const CONFIRMATION: u8 = 0x1E;
}

pub mod error {
    pub const INVALID_HANDLE: u8 = 0x01;
    pub const READ_NOT_PERMITTED: u8 = 0x02;
    pub const REQUEST_NOT_SUPPORTED: u8 = 0x06;
    pub const INSUFFICIENT_AUTHENTICATION: u8 = 0x05;
    pub const ATTRIBUTE_NOT_LONG: u8 = 0x0B;
    pub const INSUFFICIENT_ENCRYPTION: u8 = 0x0F;
    pub const ATTRIBUTE_NOT_FOUND: u8 = 0x0A;
}

/// GATT declaration and descriptor types (16-bit UUIDs).
pub mod uuid {
    pub const PRIMARY_SERVICE: u16 = 0x2800;
    pub const CHARACTERISTIC: u16 = 0x2803;
    pub const CCCD: u16 = 0x2902;
    pub const REPORT_REFERENCE: u16 = 0x2908;
    pub const HID_SERVICE: u16 = 0x1812;
    pub const BOOT_KEYBOARD_INPUT: u16 = 0x2A22;
    pub const BOOT_MOUSE_INPUT: u16 = 0x2A33;
    pub const HID_INFORMATION: u16 = 0x2A4A;
    pub const REPORT_MAP: u16 = 0x2A4B;
    pub const HID_CONTROL_POINT: u16 = 0x2A4C;
    pub const REPORT: u16 = 0x2A4D;
    pub const PROTOCOL_MODE: u16 = 0x2A4E;
}

/// The default and our preferred ATT MTU on LE.
pub const DEFAULT_MTU: u16 = 23;
pub const OUR_MTU: u16 = 185;

fn le16(b: &[u8], at: usize) -> u16 { u16::from_le_bytes([b[at], b[at + 1]]) }

pub fn exchange_mtu(mtu: u16) -> Vec<u8> { let mut p = alloc::vec![op::EXCHANGE_MTU_REQ]; p.extend_from_slice(&mtu.to_le_bytes()); p }
fn range(code: u8, start: u16, end: u16, kind: Option<u16>) -> Vec<u8> {
    let mut p = alloc::vec![code];
    p.extend_from_slice(&start.to_le_bytes());
    p.extend_from_slice(&end.to_le_bytes());
    if let Some(k) = kind { p.extend_from_slice(&k.to_le_bytes()); }
    p
}
pub fn read_by_group_type(start: u16, end: u16, kind: u16) -> Vec<u8> { range(op::READ_BY_GROUP_TYPE_REQ, start, end, Some(kind)) }
pub fn read_by_type(start: u16, end: u16, kind: u16) -> Vec<u8> { range(op::READ_BY_TYPE_REQ, start, end, Some(kind)) }
pub fn find_information(start: u16, end: u16) -> Vec<u8> { range(op::FIND_INFORMATION_REQ, start, end, None) }
pub fn read(handle: u16) -> Vec<u8> { let mut p = alloc::vec![op::READ_REQ]; p.extend_from_slice(&handle.to_le_bytes()); p }
pub fn read_blob(handle: u16, offset: u16) -> Vec<u8> {
    let mut p = read(handle);
    p[0] = op::READ_BLOB_REQ;
    p.extend_from_slice(&offset.to_le_bytes());
    p
}
pub fn write(handle: u16, value: &[u8]) -> Vec<u8> {
    let mut p = alloc::vec![op::WRITE_REQ];
    p.extend_from_slice(&handle.to_le_bytes());
    p.extend_from_slice(value);
    p
}
pub fn write_command(handle: u16, value: &[u8]) -> Vec<u8> { let mut p = write(handle, value); p[0] = op::WRITE_CMD; p }

/// A 16-bit UUID, or the 16-bit alias inside a Bluetooth base 128-bit UUID; None for
/// other 128-bit UUIDs (vendor services).
pub fn uuid16(b: &[u8]) -> Option<u16> {
    const BASE: [u8; 12] = [0xFB, 0x34, 0x9B, 0x5F, 0x80, 0x00, 0x00, 0x80, 0x00, 0x10, 0x00, 0x00];
    match b.len() {
        2 => Some(le16(b, 0)),
        16 if b[..12] == BASE && b[14..] == [0, 0] => Some(le16(b, 12)),
        _ => None,
    }
}

/// One answer from the server.
#[derive(Debug, PartialEq, Eq)]
pub enum Pdu<'a> {
    Error { request: u8, handle: u16, code: u8 },
    Mtu(u16),
    /// (handle, end group handle, value) per entry.
    Groups(Vec<(u16, u16, &'a [u8])>),
    /// (handle, value) per entry.
    Types(Vec<(u16, &'a [u8])>),
    /// (handle, uuid bytes) per entry.
    Information(Vec<(u16, &'a [u8])>),
    Read(&'a [u8]),
    Written,
    Notification { handle: u16, value: &'a [u8] },
    Indication { handle: u16, value: &'a [u8] },
    /// A request from the peer's client (we answer it ourselves), or something else.
    Other(u8),
}

/// Parse one ATT PDU from the server.
pub fn parse(p: &[u8]) -> Option<Pdu<'_>> {
    let (&code, b) = p.split_first()?;
    let list = |each: usize| -> Option<Vec<&[u8]>> {
        let n = *b.first()? as usize;
        if n < each || (b.len() - 1) % n != 0 { return None; }
        Some(b[1..].chunks(n).collect())
    };
    Some(match code {
        op::ERROR if b.len() >= 4 => Pdu::Error { request: b[0], handle: le16(b, 1), code: b[3] },
        op::EXCHANGE_MTU_RSP if b.len() >= 2 => Pdu::Mtu(le16(b, 0)),
        op::READ_BY_GROUP_TYPE_RSP => Pdu::Groups(list(6)?.into_iter().map(|e| (le16(e, 0), le16(e, 2), &e[4..])).collect()),
        op::READ_BY_TYPE_RSP => Pdu::Types(list(3)?.into_iter().map(|e| (le16(e, 0), &e[2..])).collect()),
        op::FIND_INFORMATION_RSP if !b.is_empty() => {
            let n = match b[0] { 1 => 4, 2 => 18, _ => return None };
            if (b.len() - 1) % n != 0 { return None; }
            Pdu::Information(b[1..].chunks(n).map(|e| (le16(e, 0), &e[2..])).collect())
        }
        op::READ_RSP | op::READ_BLOB_RSP => Pdu::Read(b),
        op::WRITE_RSP => Pdu::Written,
        op::NOTIFICATION if b.len() >= 2 => Pdu::Notification { handle: le16(b, 0), value: &b[2..] },
        op::INDICATION if b.len() >= 2 => Pdu::Indication { handle: le16(b, 0), value: &b[2..] },
        c => Pdu::Other(c),
    })
}

/// Our answer to a request from the peer's GATT client: we serve no attributes yet, so
/// MTU exchange is answered and every other request gets "request not supported" (or
/// "attribute not found" for discovery, which is what an empty server says). None:
/// nothing to send (a command, or not a request).
pub fn serve(p: &[u8]) -> Option<Vec<u8>> {
    let (&code, b) = p.split_first()?;
    if code == op::EXCHANGE_MTU_REQ {
        let mut r = alloc::vec![op::EXCHANGE_MTU_RSP];
        r.extend_from_slice(&OUR_MTU.to_le_bytes());
        return Some(r);
    }
    let request = code & 0x40 == 0 && code & 1 == 0 && code != op::CONFIRMATION && code != op::NOTIFICATION && code != op::INDICATION;
    if !request { return None; }
    let handle = if b.len() >= 2 { le16(b, 0) } else { 0 };
    let reason = match code {
        op::FIND_INFORMATION_REQ | op::READ_BY_TYPE_REQ | op::READ_BY_GROUP_TYPE_REQ | 0x06 => error::ATTRIBUTE_NOT_FOUND,
        op::READ_REQ | op::READ_BLOB_REQ | op::WRITE_REQ => error::INVALID_HANDLE,
        _ => error::REQUEST_NOT_SUPPORTED,
    };
    let mut r = alloc::vec![op::ERROR, code];
    r.extend_from_slice(&handle.to_le_bytes());
    r.push(reason);
    Some(r)
}
