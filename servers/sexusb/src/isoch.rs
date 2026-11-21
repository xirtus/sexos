// SPDX-License-Identifier: GPL-3.0-only
//! Isochronous transfers on xHCI (xHCI 1.2 sections 4.11.2.5, 4.14.2, 6.2.3, 6.4.1.3): the
//! endpoint parameters, Isoch TRBs and how many packets fit a transfer.
//!
//! Generic on purpose: webcams (UVC), USB audio and Bluetooth SCO all stream this way.
//! Pure arithmetic with no MMIO, so `tests/sexusb_isoch.rs` runs this exact file.

/// TRB type 5: Isoch.
pub const TRB_ISOCH: u32 = 5;
/// Endpoint context types.
pub const EP_ISOCH_OUT: u32 = 1;
pub const EP_ISOCH_IN: u32 = 5;

// Speeds as the port status reports them.
pub const FULL: u32 = 1;
pub const LOW: u32 = 2;
pub const HIGH: u32 = 3;
pub const SUPER: u32 = 4;

/// What an isochronous endpoint can move per service interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Endpoint {
    /// Bytes per packet (wMaxPacketSize bits 10:0).
    pub mps: u16,
    /// Max Burst Size field: packets per burst minus one (high speed: the extra
    /// transactions in wMaxPacketSize bits 12:11; SuperSpeed: the companion's bMaxBurst).
    pub burst: u8,
    /// Mult field: bursts per interval minus one (SuperSpeed companion bmAttributes 1:0).
    pub mult: u8,
    /// Interval field: the service interval is 2^interval x 125 us.
    pub interval: u32,
}

impl Endpoint {
    /// From the endpoint descriptor (and the SuperSpeed companion, when there is one).
    pub fn new(speed: u32, w_max_packet: u16, b_interval: u8, companion: Option<(u8, u8)>) -> Self {
        let mps = w_max_packet & 0x7FF;
        let (burst, mult) = match (speed, companion) {
            (SUPER | 5, Some((max_burst, attributes))) => (max_burst.min(15), (attributes & 3).min(2)),
            (HIGH, _) => (((w_max_packet >> 11) & 3).min(2) as u8, 0),
            _ => (0, 0),
        };
        Self { mps, burst, mult, interval: interval(speed, b_interval) }
    }
    /// Max ESIT Payload: bytes per service interval.
    pub fn esit_payload(&self) -> u32 { self.mps as u32 * (self.burst as u32 + 1) * (self.mult as u32 + 1) }
    /// Microframes (125 us) per service interval.
    pub fn microframes(&self) -> u32 { 1 << self.interval.min(15) }
}

/// The Interval field for an isochronous endpoint: bInterval is already an exponent
/// (2^(b-1) frames at full speed, microframes at high and SuperSpeed).
pub fn interval(speed: u32, b_interval: u8) -> u32 {
    let b = b_interval.clamp(1, 16) as u32 - 1;
    match speed { FULL | LOW => (b + 3).min(15), _ => b }
}

/// Packets a transfer of `len` bytes takes (at least one: a zero-length packet).
pub fn packets(len: u32, mps: u16) -> u32 { if mps == 0 { 1 } else { len.div_ceil(mps as u32).max(1) } }

/// Transfer Burst Count and Transfer Last Burst Packet Count (xHCI 4.11.2.3).
pub fn burst_counts(packets: u32, burst: u8) -> (u32, u32) {
    let per = burst as u32 + 1;
    let tbc = packets.div_ceil(per).max(1) - 1;
    let rem = packets % per;
    let tlbpc = if rem == 0 { burst as u32 } else { rem - 1 };
    (tbc, tlbpc)
}

/// One Isoch TRB for a whole TD in one buffer: `frame` is the 11-bit frame id to start in,
/// or `None` to start As Soon As Possible (SIA).
pub fn trb(buffer: u64, len: u32, ep: &Endpoint, frame: Option<u16>, ioc: bool, cycle: u32) -> [u32; 4] {
    let n = packets(len, ep.mps);
    let (tbc, tlbpc) = burst_counts(n, ep.burst);
    let d2 = (len & 0x1_FFFF) | (0 << 17) /* TD Size: last TRB of the TD */ | (0 << 22) /* interrupter 0 */;
    let mut d3 = (cycle & 1) | (1 << 2) /* ISP */ | ((tbc & 3) << 7) | (TRB_ISOCH << 10) | ((tlbpc & 15) << 16);
    if ioc { d3 |= 1 << 5; }
    match frame {
        Some(f) => d3 |= ((f as u32) & 0x7FF) << 20,
        None => d3 |= 1 << 31,
    }
    [buffer as u32, (buffer >> 32) as u32, d2, d3]
}

/// The bytes one TD carries for this endpoint: one full service interval.
pub fn td_bytes(ep: &Endpoint) -> u32 { ep.esit_payload() }

/// The smallest alternate setting that carries `need` bytes per interval, else the largest.
/// `alts` holds (alternate setting, bytes per interval).
pub fn pick_alt(alts: &[(u8, u32)], need: u32) -> Option<u8> {
    alts.iter().filter(|a| a.1 >= need).min_by_key(|a| a.1)
        .or_else(|| alts.iter().max_by_key(|a| a.1)).map(|a| a.0)
}
