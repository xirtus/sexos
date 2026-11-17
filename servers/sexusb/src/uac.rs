// SPDX-License-Identifier: GPL-3.0-only
//! USB Audio Class 1 and 2 descriptors: finding the playback setting of a USB sound
//! card (stereo, 16-bit PCM, 48 kHz) and the requests that set its rate. Pure, tested
//! on the host (tests/usb_audio.rs).

/// Interface class Audio, its subclasses, and the UAC2 protocol number.
pub const CLASS_AUDIO: u8 = 1;
pub const SUB_CONTROL: u8 = 1;
pub const SUB_STREAMING: u8 = 2;
pub const PROTOCOL_UAC2: u8 = 0x20;
/// Class-specific interface descriptor and its subtypes.
const CS_INTERFACE: u8 = 0x24;
const AS_GENERAL: u8 = 1;
const FORMAT_TYPE: u8 = 2;
const CLOCK_SOURCE: u8 = 0x0A;

/// A playback setting this driver can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Playback {
    pub uac2: bool,
    pub config: u8,
    /// The AudioControl interface (UAC2 rate requests go to it).
    pub control: u8,
    pub iface: u8,
    pub alt: u8,
    /// Isochronous OUT endpoint address, packet size and bInterval.
    pub ep: u8,
    pub mps: u16,
    pub interval: u8,
    /// Synchronisation type from the endpoint attributes: 1 async, 2 adaptive, 3 sync.
    pub sync: u8,
    /// UAC2: the clock source the rate is set on (the first one listed).
    pub clock: u8,
}

/// Whether a UAC1 sample-rate list (bSamFreqType, then 3-byte rates) offers 48 kHz.
fn offers_48k(kind: u8, rates: &[u8]) -> bool {
    let rate = |k: usize| rates.get(3 * k..3 * k + 3).map(|r| r[0] as u32 | (r[1] as u32) << 8 | (r[2] as u32) << 16);
    if kind == 0 {
        // Continuous: lower and upper bound.
        matches!((rate(0), rate(1)), (Some(lo), Some(hi)) if lo <= 48_000 && 48_000 <= hi)
    } else {
        (0..kind as usize).any(|k| rate(k) == Some(48_000))
    }
}

/// The playback setting in one configuration descriptor, if it has one.
pub fn playback(cfg: &[u8]) -> Option<Playback> {
    if cfg.len() < 9 { return None; }
    let config = cfg[5];
    let (mut control, mut clock) = (None::<u8>, 0u8);
    // The interface setting being read, and what it has said so far.
    let (mut iface, mut alt, mut class, mut sub, mut proto) = (0u8, 0u8, 0u8, 0u8, 0u8);
    let (mut pcm, mut channels, mut bytes, mut rate_ok) = (false, 0u8, 0u8, false);
    let mut at = 0;
    while at + 2 <= cfg.len() {
        let len = cfg[at] as usize;
        if len < 2 || at + len > cfg.len() { break; }
        let d = &cfg[at..at + len];
        match d[1] {
            4 if len >= 9 => {
                (iface, alt, class, sub, proto) = (d[2], d[3], d[5], d[6], d[7]);
                (pcm, channels, bytes, rate_ok) = (false, 0, 0, false);
                if class == CLASS_AUDIO && sub == SUB_CONTROL && control.is_none() { control = Some(iface); }
            }
            CS_INTERFACE if class == CLASS_AUDIO && sub == SUB_CONTROL && len >= 4 && d[2] == CLOCK_SOURCE => {
                if clock == 0 { clock = d[3]; }
            }
            CS_INTERFACE if class == CLASS_AUDIO && sub == SUB_STREAMING && len >= 3 => {
                let uac2 = proto == PROTOCOL_UAC2;
                match (d[2], uac2) {
                    // UAC1 AS_GENERAL: wFormatTag 1 is PCM.
                    (AS_GENERAL, false) if len >= 7 => pcm = u16::from_le_bytes([d[5], d[6]]) == 1,
                    // UAC2 AS_GENERAL: bmFormats bit 0 is PCM; then the channel count.
                    (AS_GENERAL, true) if len >= 11 => { pcm = d[6] & 1 != 0; channels = d[10]; }
                    // UAC1 Type I format: channels, subframe size, bits, then the rates.
                    (FORMAT_TYPE, false) if len >= 8 && d[3] == 1 => {
                        channels = d[4];
                        bytes = d[5];
                        rate_ok = offers_48k(d[7], &d[8..]);
                    }
                    // UAC2 Type I format: subslot size; rates come from the clock.
                    (FORMAT_TYPE, true) if len >= 6 && d[3] == 1 => { bytes = d[4]; rate_ok = true; }
                    _ => {}
                }
            }
            5 if len >= 7 && class == CLASS_AUDIO && sub == SUB_STREAMING && alt != 0 => {
                let (addr, attr) = (d[2], d[3]);
                if addr & 0x80 == 0 && attr & 3 == 1 && pcm && channels == 2 && bytes == 2 && rate_ok {
                    return Some(Playback {
                        uac2: proto == PROTOCOL_UAC2, config, control: control.unwrap_or(0), iface, alt,
                        ep: addr, mps: u16::from_le_bytes([d[4], d[5]]) & 0x7ff, interval: d[6].max(1),
                        sync: (attr >> 2) & 3, clock,
                    });
                }
            }
            _ => {}
        }
        at += len;
    }
    None
}

/// Microseconds between packets: full speed counts bInterval in 1 ms frames, high speed
/// and faster as 2^(bInterval-1) microframes of 125 µs.
pub fn service_us(high_speed: bool, interval: u8) -> u32 {
    if high_speed { 125 << (interval.clamp(1, 16) - 1).min(6) } else { 1000 << (interval.clamp(1, 16) - 1).min(4) }
}

/// xHCI endpoint context Interval for an isochronous endpoint (125 µs units, log2).
pub fn xhci_interval(high_speed: bool, interval: u8) -> u32 {
    let i = interval.clamp(1, 16) as u32 - 1;
    if high_speed { i } else { i + 3 }
}

/// The SET_CUR request that sets 48 kHz: (bmRequestType, wValue, wIndex, data, length).
/// UAC1 sets it on the endpoint (3 bytes); UAC2 on the clock source (4 bytes).
pub fn set_rate(p: &Playback) -> (u8, u16, u16, [u8; 4], u16) {
    let r = 48_000u32.to_le_bytes();
    if p.uac2 { (0x21, 0x0100, (p.clock as u16) << 8 | p.control as u16, r, 4) }
    else { (0x22, 0x0100, p.ep as u16, r, 3) }
}
