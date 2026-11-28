// SPDX-License-Identifier: GPL-3.0-only
//! RTP and RTCP for WebRTC media in SexOS (RFC 3550). No I/O and no crypto: packets come in
//! and go out as bytes, and sex-srtp protects them.
//!
//! - `packet`: RTP headers, one- and two-byte header extensions, abs-send-time,
//!   transport-wide-cc numbers, sequence unwrapping.
//! - `rtcp`: SR, RR, SDES, BYE, NACK, PLI, FIR, REMB, transport-cc feedback, receive stats.
//! - `codec`: Opus, VP8, VP9 (non-flexible) and H.264 packetizers and depacketizers.
//! - `jitter`: jitter buffer with NACK, and the send stream with its retransmit history.
//! - `bwe`: send-side bandwidth estimate (GCC-like delay and loss) for the encoder target.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod bwe;
pub mod codec;
pub mod jitter;
pub mod packet;
pub mod rtcp;

pub use bwe::BandwidthEstimator;
pub use codec::{Depacketizer, Frame, Packetizer};
pub use jitter::{JitterBuffer, JitterConfig, RtpStream};
pub use packet::{Error, Packet, SeqUnwrapper};
pub use rtcp::{ReportBlock, Rtcp, TransportCc, TwccRecorder};

/// RTP or RTCP? (RFC 5761: payload types 192..=223 in the second byte are RTCP.)
pub fn is_rtcp(b: &[u8]) -> bool {
    b.len() >= 2 && (192..=223).contains(&b[1])
}

/// Is this datagram RTP or RTCP? (RFC 7983: first byte 128..=191.)
pub fn is_rtp_or_rtcp(first_byte: u8) -> bool {
    (128..=191).contains(&first_byte)
}
