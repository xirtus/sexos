// SPDX-License-Identifier: GPL-3.0-only
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod g711;
pub mod vp8;
mod vp8_tables;
pub mod host;
pub mod peer;

pub use host::Rtc;
pub use peer::{Peer, Signaling, SdpType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Audio, Video }

/// The codecs Kaleidoscope negotiates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec { Opus, Pcmu, Pcma, Vp8, Vp9, H264 }

impl Codec {
    pub fn name(self) -> &'static str {
        match self { Codec::Opus => "opus", Codec::Pcmu => "PCMU", Codec::Pcma => "PCMA", Codec::Vp8 => "VP8", Codec::Vp9 => "VP9", Codec::H264 => "H264" }
    }
    pub fn mime(self) -> alloc::string::String {
        let k = if self.is_audio() { "audio" } else { "video" };
        alloc::format!("{k}/{}", self.name()).to_ascii_lowercase()
    }
    pub fn clock(self) -> u32 { match self { Codec::Opus => 48_000, Codec::Pcmu | Codec::Pcma => 8_000, _ => 90_000 } }
    pub fn channels(self) -> Option<u16> { (self == Codec::Opus).then_some(2) }
    pub fn is_audio(self) -> bool { matches!(self, Codec::Opus | Codec::Pcmu | Codec::Pcma) }
    /// Kaleidoscope can send it (its own VP8 encoder and G.711).
    pub fn encodable(self) -> bool { matches!(self, Codec::Vp8 | Codec::Pcmu | Codec::Pcma) }
    pub fn from_rtpmap(name: &str, clock: u32) -> Option<Codec> {
        let c = match name.to_ascii_lowercase().as_str() {
            "opus" => Codec::Opus, "pcmu" => Codec::Pcmu, "pcma" => Codec::Pcma, "vp8" => Codec::Vp8, "vp9" => Codec::Vp9, "h264" => Codec::H264,
            _ => return None,
        };
        (c.clock() == clock).then_some(c)
    }
}
