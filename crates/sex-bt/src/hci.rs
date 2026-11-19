// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! HCI packets (Bluetooth Core 5.4, Vol 4 Part E): commands the host sends, events the
//! controller sends back, and ACL data both ways. Packets travel with a one-byte type in
//! front (the UART "H4" types) through SexOS's shared ring (`sex_pdx::bt_hci`), so the
//! USB transport never needs to look inside them.

use alloc::vec::Vec;

/// Packet types, as the first byte of a packet on the ring.
pub const COMMAND: u8 = 0x01;
pub const ACL: u8 = 0x02;
pub const EVENT: u8 = 0x04;

/// A Bluetooth device address, least significant byte first (as on the wire).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Addr(pub [u8; 6]);

impl core::fmt::Display for Addr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let a = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", a[5], a[4], a[3], a[2], a[1], a[0])
    }
}

/// Opcode = OGF << 10 | OCF.
pub mod op {
    pub const INQUIRY: u16 = 0x0401;
    pub const INQUIRY_CANCEL: u16 = 0x0402;
    pub const CREATE_CONNECTION: u16 = 0x0405;
    pub const LINK_KEY_REQUEST_REPLY: u16 = 0x040B;
    pub const LINK_KEY_REQUEST_NEGATIVE_REPLY: u16 = 0x040C;
    pub const AUTHENTICATION_REQUESTED: u16 = 0x0411;
    pub const REMOTE_NAME_REQUEST: u16 = 0x0419;
    pub const IO_CAPABILITY_REQUEST_REPLY: u16 = 0x042B;
    pub const USER_CONFIRMATION_REQUEST_REPLY: u16 = 0x042C;
    pub const USER_CONFIRMATION_REQUEST_NEGATIVE_REPLY: u16 = 0x042D;
    pub const IO_CAPABILITY_REQUEST_NEGATIVE_REPLY: u16 = 0x0434;
    pub const SET_EVENT_MASK: u16 = 0x0C01;
    pub const RESET: u16 = 0x0C03;
    pub const WRITE_LOCAL_NAME: u16 = 0x0C13;
    pub const WRITE_SCAN_ENABLE: u16 = 0x0C1A;
    pub const WRITE_CLASS_OF_DEVICE: u16 = 0x0C24;
    pub const WRITE_INQUIRY_MODE: u16 = 0x0C45;
    pub const WRITE_SIMPLE_PAIRING_MODE: u16 = 0x0C56;
    pub const WRITE_LE_HOST_SUPPORTED: u16 = 0x0C6D;
    pub const READ_LOCAL_VERSION: u16 = 0x1001;
    pub const READ_LOCAL_FEATURES: u16 = 0x1003;
    pub const READ_BUFFER_SIZE: u16 = 0x1005;
    pub const READ_BD_ADDR: u16 = 0x1009;
    pub const LE_SET_EVENT_MASK: u16 = 0x2001;
    pub const LE_READ_BUFFER_SIZE: u16 = 0x2002;
    pub const LE_READ_LOCAL_FEATURES: u16 = 0x2003;
    pub const LE_SET_SCAN_PARAMETERS: u16 = 0x200B;
    pub const LE_SET_SCAN_ENABLE: u16 = 0x200C;
    pub const LE_CREATE_CONNECTION: u16 = 0x200D;
    pub const LE_START_ENCRYPTION: u16 = 0x2019;
    pub const LE_LTK_REQUEST_REPLY: u16 = 0x201A;
    pub const LE_LTK_REQUEST_NEGATIVE_REPLY: u16 = 0x201B;
    pub const LE_SET_EXTENDED_SCAN_PARAMETERS: u16 = 0x2041;
    pub const LE_SET_EXTENDED_SCAN_ENABLE: u16 = 0x2042;
}

/// Event codes.
pub mod ev {
    pub const INQUIRY_COMPLETE: u8 = 0x01;
    pub const INQUIRY_RESULT: u8 = 0x02;
    pub const CONNECTION_COMPLETE: u8 = 0x03;
    pub const DISCONNECTION_COMPLETE: u8 = 0x05;
    pub const REMOTE_NAME_REQUEST_COMPLETE: u8 = 0x07;
    pub const ENCRYPTION_CHANGE: u8 = 0x08;
    pub const COMMAND_COMPLETE: u8 = 0x0E;
    pub const COMMAND_STATUS: u8 = 0x0F;
    pub const NUMBER_OF_COMPLETED_PACKETS: u8 = 0x13;
    pub const LINK_KEY_REQUEST: u8 = 0x17;
    pub const LINK_KEY_NOTIFICATION: u8 = 0x18;
    pub const INQUIRY_RESULT_WITH_RSSI: u8 = 0x22;
    pub const EXTENDED_INQUIRY_RESULT: u8 = 0x2F;
    pub const IO_CAPABILITY_REQUEST: u8 = 0x31;
    pub const IO_CAPABILITY_RESPONSE: u8 = 0x32;
    pub const USER_CONFIRMATION_REQUEST: u8 = 0x33;
    pub const SIMPLE_PAIRING_COMPLETE: u8 = 0x36;
    pub const LE_META: u8 = 0x3E;
    pub const VENDOR: u8 = 0xFF;
    // LE meta sub-events.
    pub const LE_CONNECTION_COMPLETE: u8 = 0x01;
    pub const LE_ADVERTISING_REPORT: u8 = 0x02;
    pub const LE_LTK_REQUEST: u8 = 0x05;
    pub const LE_ENHANCED_CONNECTION_COMPLETE: u8 = 0x0A;
    pub const LE_EXTENDED_ADVERTISING_REPORT: u8 = 0x0D;
}

/// A command packet with its type byte: `01 opcode(le16) plen params`.
pub fn command(opcode: u16, params: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + params.len());
    p.push(COMMAND);
    p.extend_from_slice(&opcode.to_le_bytes());
    p.push(params.len().min(255) as u8);
    p.extend_from_slice(&params[..params.len().min(255)]);
    p
}

/// The opcode and parameters of a command packet (with its type byte).
pub fn parse_command(p: &[u8]) -> Option<(u16, &[u8])> {
    if p.len() < 4 || p[0] != COMMAND { return None; }
    let plen = p[3] as usize;
    if p.len() != 4 + plen { return None; }
    Some((u16::from_le_bytes([p[1], p[2]]), &p[4..]))
}

/// An event packet with its type byte: `04 code plen params`.
pub fn event(code: u8, params: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(3 + params.len());
    p.extend_from_slice(&[EVENT, code, params.len() as u8]);
    p.extend_from_slice(params);
    p
}

/// A Command Complete event packet.
pub fn command_complete(opcode: u16, ret: &[u8]) -> Vec<u8> {
    let mut params = Vec::with_capacity(3 + ret.len());
    params.push(1);
    params.extend_from_slice(&opcode.to_le_bytes());
    params.extend_from_slice(ret);
    event(ev::COMMAND_COMPLETE, &params)
}

/// A Command Status event packet.
pub fn command_status(opcode: u16, status: u8) -> Vec<u8> {
    let o = opcode.to_le_bytes();
    event(ev::COMMAND_STATUS, &[status, 1, o[0], o[1]])
}

/// An event, decoded as far as the host stack needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    CommandComplete { ncmd: u8, opcode: u16, ret: &'a [u8] },
    CommandStatus { status: u8, ncmd: u8, opcode: u16 },
    InquiryComplete { status: u8 },
    ConnectionComplete { status: u8, handle: u16, addr: Addr, link_type: u8 },
    DisconnectionComplete { status: u8, handle: u16, reason: u8 },
    RemoteNameComplete { status: u8, addr: Addr, name: &'a [u8] },
    EncryptionChange { status: u8, handle: u16, enabled: u8 },
    CompletedPackets { handles: &'a [u8] },
    LinkKeyRequest { addr: Addr },
    LinkKeyNotification { addr: Addr, key: [u8; 16], key_type: u8 },
    IoCapabilityRequest { addr: Addr },
    IoCapabilityResponse { addr: Addr, io: u8, oob: u8, auth: u8 },
    UserConfirmationRequest { addr: Addr, value: u32 },
    SimplePairingComplete { status: u8, addr: Addr },
    LeConnectionComplete { status: u8, handle: u16, role: u8, peer_type: u8, peer: Addr },
    LeLtkRequest { handle: u16, rand: [u8; 8], ediv: u16 },
    /// Inquiry results and advertising reports: parse with `report::reports`.
    Report { code: u8, sub: u8, params: &'a [u8] },
    /// 0xFF: the first byte of `params` is the vendor's own sub-event.
    Vendor { params: &'a [u8] },
    Other { code: u8, params: &'a [u8] },
}

fn addr(b: &[u8]) -> Addr { let mut a = [0; 6]; a.copy_from_slice(&b[..6]); Addr(a) }
fn le16(b: &[u8]) -> u16 { u16::from_le_bytes([b[0], b[1]]) }

/// Decode an event packet (with its type byte). None: not a whole event, or too short
/// for its kind.
pub fn parse_event(p: &[u8]) -> Option<Event<'_>> {
    if p.len() < 3 || p[0] != EVENT || p.len() != 3 + p[2] as usize { return None; }
    let (code, b) = (p[1], &p[3..]);
    let need = |n: usize| if b.len() >= n { Some(()) } else { None };
    Some(match code {
        ev::COMMAND_COMPLETE => { need(3)?; Event::CommandComplete { ncmd: b[0], opcode: le16(&b[1..]), ret: &b[3..] } }
        ev::COMMAND_STATUS => { need(4)?; Event::CommandStatus { status: b[0], ncmd: b[1], opcode: le16(&b[2..]) } }
        ev::INQUIRY_COMPLETE => { need(1)?; Event::InquiryComplete { status: b[0] } }
        ev::CONNECTION_COMPLETE => { need(11)?; Event::ConnectionComplete { status: b[0], handle: le16(&b[1..]) & 0xfff, addr: addr(&b[3..]), link_type: b[9] } }
        ev::DISCONNECTION_COMPLETE => { need(4)?; Event::DisconnectionComplete { status: b[0], handle: le16(&b[1..]) & 0xfff, reason: b[3] } }
        ev::REMOTE_NAME_REQUEST_COMPLETE => {
            need(7)?;
            let name = &b[7..];
            let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            Event::RemoteNameComplete { status: b[0], addr: addr(&b[1..]), name: &name[..end] }
        }
        ev::ENCRYPTION_CHANGE => { need(4)?; Event::EncryptionChange { status: b[0], handle: le16(&b[1..]) & 0xfff, enabled: b[3] } }
        ev::NUMBER_OF_COMPLETED_PACKETS => { need(1)?; Event::CompletedPackets { handles: b } }
        ev::LINK_KEY_REQUEST => { need(6)?; Event::LinkKeyRequest { addr: addr(b) } }
        ev::LINK_KEY_NOTIFICATION => {
            need(23)?;
            let mut key = [0; 16];
            key.copy_from_slice(&b[6..22]);
            Event::LinkKeyNotification { addr: addr(b), key, key_type: b[22] }
        }
        ev::IO_CAPABILITY_REQUEST => { need(6)?; Event::IoCapabilityRequest { addr: addr(b) } }
        ev::IO_CAPABILITY_RESPONSE => { need(9)?; Event::IoCapabilityResponse { addr: addr(b), io: b[6], oob: b[7], auth: b[8] } }
        ev::USER_CONFIRMATION_REQUEST => { need(10)?; Event::UserConfirmationRequest { addr: addr(b), value: u32::from_le_bytes([b[6], b[7], b[8], b[9]]) } }
        ev::SIMPLE_PAIRING_COMPLETE => { need(7)?; Event::SimplePairingComplete { status: b[0], addr: addr(&b[1..]) } }
        ev::INQUIRY_RESULT | ev::INQUIRY_RESULT_WITH_RSSI | ev::EXTENDED_INQUIRY_RESULT => Event::Report { code, sub: 0, params: b },
        ev::LE_META => {
            need(1)?;
            match b[0] {
                ev::LE_CONNECTION_COMPLETE | ev::LE_ENHANCED_CONNECTION_COMPLETE => {
                    need(12)?;
                    Event::LeConnectionComplete { status: b[1], handle: le16(&b[2..]) & 0xfff, role: b[4], peer_type: b[5], peer: addr(&b[6..]) }
                }
                ev::LE_LTK_REQUEST => {
                    need(13)?;
                    let mut rand = [0; 8];
                    rand.copy_from_slice(&b[3..11]);
                    Event::LeLtkRequest { handle: le16(&b[1..]) & 0xfff, rand, ediv: le16(&b[11..]) }
                }
                ev::LE_ADVERTISING_REPORT | ev::LE_EXTENDED_ADVERTISING_REPORT => Event::Report { code, sub: b[0], params: &b[1..] },
                _ => Event::Other { code, params: b },
            }
        }
        ev::VENDOR => { need(1)?; Event::Vendor { params: b } }
        _ => Event::Other { code, params: b },
    })
}

/// ACL packet boundary flags.
pub const PB_FIRST_NON_FLUSHABLE: u8 = 0b00;
pub const PB_CONTINUING: u8 = 0b01;
pub const PB_FIRST_FLUSHABLE: u8 = 0b10;

/// An ACL data packet with its type byte: `02 handle|pb|bc (le16) len (le16) data`.
pub fn acl(handle: u16, pb: u8, data: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(5 + data.len());
    p.push(ACL);
    p.extend_from_slice(&((handle & 0xfff) | ((pb as u16 & 3) << 12)).to_le_bytes());
    p.extend_from_slice(&(data.len() as u16).to_le_bytes());
    p.extend_from_slice(data);
    p
}

/// (handle, packet boundary flag, data) of an ACL packet with its type byte.
pub fn parse_acl(p: &[u8]) -> Option<(u16, u8, &[u8])> {
    if p.len() < 5 || p[0] != ACL { return None; }
    let h = le16(&p[1..]);
    let len = le16(&p[3..]) as usize;
    if p.len() != 5 + len { return None; }
    Some((h & 0xfff, ((h >> 12) & 3) as u8, &p[5..]))
}
