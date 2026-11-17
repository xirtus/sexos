// SPDX-License-Identifier: GPL-3.0-only
//! Sound to a monitor over HDMI or DisplayPort, the codec's half: which pins are
//! digital display outputs, what the monitor says it can play (its ELD), and the
//! audio InfoFrame the codec sends it. Pure, tested on the host (tests/bell_hdmi.rs).
//!
//! The display side (the GPU's transcoder turning audio on, and its power well)
//! belongs to the display driver; until it runs, a monitor can be found and named
//! here but may stay silent.

/// Pin capability bits (HDA spec 7.3.4.9).
pub const PIN_CAP_OUT: u32 = 1 << 4;
pub const PIN_CAP_HDMI: u32 = 1 << 7;
pub const PIN_CAP_DP: u32 = 1 << 24;
/// Audio widget capability: digital converter (7.3.4.6).
pub const WIDGET_DIGITAL: u32 = 1 << 9;

/// Verbs for digital converters and display pins.
pub const SET_DIGI_CONVERT_1: u32 = 0x70d;
pub const SET_CVT_CHAN_COUNT: u32 = 0x72d;
pub const GET_HDMI_DIP_SIZE: u32 = 0xf2e;
pub const GET_HDMI_ELDD: u32 = 0xf2f;
pub const SET_HDMI_DIP_INDEX: u32 = 0x730;
pub const SET_HDMI_DIP_DATA: u32 = 0x731;
pub const SET_HDMI_DIP_XMIT: u32 = 0x732;
/// Pin sense: something is connected (bit 31), and its ELD is valid (bit 30).
pub const SENSE_PRESENT: u32 = 1 << 31;
pub const SENSE_ELD_VALID: u32 = 1 << 30;
/// DIGI_CONVERT_1: digital output on, no copyright or pre-emphasis bits.
pub const DIGITAL_ENABLE: u32 = 0x01;
/// Intel display codecs hide their display pins until bit 1 of vendor verb 0x781 on
/// the vendor node (0x08) is set ("all pins"); 0xf81 reads it.
pub const INTEL_VENDOR_NODE: u32 = 0x08;
pub const INTEL_GET_VENDOR: u32 = 0xf81;
pub const INTEL_SET_VENDOR: u32 = 0x781;
pub const INTEL_ALL_PINS: u32 = 1 << 1;
pub const fn is_intel_display_codec(vendor: u32) -> bool { vendor >> 16 == 0x8086 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display { Hdmi, DisplayPort }

/// A pin that sends sound to a monitor, when its capabilities and default
/// configuration say so and something is wired to it.
pub fn display_pin(pin_caps: u32, config: u32) -> Option<Display> {
    if pin_caps & PIN_CAP_OUT == 0 { return None; }
    if config >> 30 == 1 { return None; } // no connection
    if pin_caps & PIN_CAP_DP != 0 { Some(Display::DisplayPort) }
    else if pin_caps & PIN_CAP_HDMI != 0 { Some(Display::Hdmi) }
    else { None }
}

/// What a monitor's ELD (EDID-Like Data, HDA 7.3.3.34) says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Eld {
    pub version: u8,
    pub name: [u8; 16],
    pub name_len: usize,
    /// Most channels any PCM format plays (2 when the monitor lists none).
    pub pcm_channels: u8,
    /// PCM sample rates as the CEA short audio descriptor bits: 32, 44.1, 48, 88.2,
    /// 96, 176.4 and 192 kHz for bits 0..7.
    pub pcm_rates: u8,
    /// Whether it says it is DisplayPort (connection type 1) rather than HDMI (0).
    pub displayport: bool,
}
impl Eld {
    pub fn name(&self) -> &[u8] { &self.name[..self.name_len] }
    pub fn plays_48k(&self) -> bool { self.pcm_rates & 1 << 2 != 0 }
}

/// Reads an ELD buffer. None when it is too short or not version 2 (CEA-861).
pub fn parse_eld(b: &[u8]) -> Option<Eld> {
    // Header: byte 0 version in bits 3..7; byte 2 baseline length in 4-byte words.
    if b.len() < 20 { return None; }
    let version = b[0] >> 3;
    if version != 2 && version != 31 { return None; }
    let mnl = (b[4] & 0x1f) as usize; // monitor name length
    let sad_count = (b[5] >> 4) as usize;
    let displayport = (b[5] >> 2) & 3 == 1;
    let name_at = 20;
    if name_at + mnl + 3 * sad_count > b.len() { return None; }
    let mut name = [0u8; 16];
    let name_len = mnl.min(16);
    name[..name_len].copy_from_slice(&b[name_at..name_at + name_len]);
    let (mut pcm_channels, mut pcm_rates) = (0u8, 0u8);
    for k in 0..sad_count {
        let sad = &b[name_at + mnl + 3 * k..name_at + mnl + 3 * k + 3];
        // Format code 1 is linear PCM.
        if (sad[0] >> 3) & 0xf == 1 {
            pcm_channels = pcm_channels.max((sad[0] & 7) + 1);
            pcm_rates |= sad[1] & 0x7f;
        }
    }
    if pcm_channels == 0 { pcm_channels = 2; pcm_rates |= 0b111; }
    Some(Eld { version, name, name_len, pcm_channels, pcm_rates, displayport })
}

/// The CEA-861 audio InfoFrame for two-channel PCM, "refer to stream header" for
/// format, rate and size: type 0x84, version 1, length 10, then checksum and payload,
/// as the codec's DIP buffer takes it.
pub fn audio_infoframe(channels: u8) -> [u8; 14] {
    let mut f = [0u8; 14];
    f[0] = 0x84;
    f[1] = 0x01;
    f[2] = 10;
    // Data byte 1: coding type 0 (from stream), channel count - 1.
    f[4] = channels.saturating_sub(1) & 7;
    // Data byte 4: speaker allocation 0 (front left and right).
    let sum = f.iter().fold(0u8, |s, b| s.wrapping_add(*b));
    f[3] = 0u8.wrapping_sub(sum);
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digital_pins_are_found() {
        assert_eq!(display_pin(PIN_CAP_OUT | PIN_CAP_HDMI, 0x1856_0010), Some(Display::Hdmi));
        assert_eq!(display_pin(PIN_CAP_OUT | PIN_CAP_HDMI | PIN_CAP_DP, 0x1856_0010), Some(Display::DisplayPort));
        assert_eq!(display_pin(PIN_CAP_OUT | PIN_CAP_HDMI, 0x4056_0010), None);
        assert_eq!(display_pin(PIN_CAP_OUT, 0x0101_0010), None);
        assert_eq!(display_pin(PIN_CAP_HDMI, 0x1856_0010), None);
    }
}
