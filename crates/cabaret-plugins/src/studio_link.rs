//! Pure-Rust LAN **CabaretLink** protocol (`DistroAV` / `obs-ndi` + `obs-teleport` replacement)
//! with zero C/proprietary dependencies.
//!
//! Provides mDNS-style service announcement payloads, fast lossless 32-bit RLE RGBA8 & YUV422
//! video compression, 48 kHz PCM audio frames, Program/Preview **Tally** signaling, CRC-32
//! protected packet framing, and a streaming socket deframer.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use cabaret_core::compositor::{CompositorError, FrameBuffer};
use cabaret_core::transitions::TallyState;

use crate::util::{clamp01, roundf};

/// Standard mDNS service type string for CabaretLink discovery.
pub const CABARET_LINK_SERVICE: &str = "_cabaret-link._tcp.local.";
pub const CABARET_LINK_MDNS_PORT: u16 = 5353;
pub const CABARET_LINK_DEFAULT_PORT: u16 = 5960;

/// 4-byte frame header magic `"CBL1"`.
pub const LINK_PACKET_MAGIC: [u8; 4] = *b"CBL1";
pub const LINK_PROTOCOL_VERSION: u8 = 1;
const LINK_HEADER_LEN: usize = 24;

/// Errors emitted during CabaretLink encoding, decoding, and framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    UnexpectedEof,
    BadMagic,
    UnsupportedVersion(u8),
    UnknownPacketKind(u8),
    ChecksumMismatch { expected: u32, actual: u32 },
    CorruptRleStream,
    InvalidDimensions,
    InvalidUtf8,
    Compositor(CompositorError),
}

impl From<CompositorError> for LinkError {
    fn from(e: CompositorError) -> Self {
        Self::Compositor(e)
    }
}

/// mDNS-style service announcement payload advertising a live CabaretLink stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceAnnouncement {
    pub instance_name: String,
    pub host_name: String,
    pub ipv4: [u8; 4],
    pub port: u16,
    pub groups: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub ttl_sec: u32,
}

impl Default for ServiceAnnouncement {
    fn default() -> Self {
        Self {
            instance_name: String::from("Cabaret-Program"),
            host_name: String::from("cabaret.local"),
            ipv4: [127, 0, 0, 1],
            port: CABARET_LINK_DEFAULT_PORT,
            groups: vec![String::from("Public")],
            width: 1920,
            height: 1080,
            fps_num: 60,
            fps_den: 1,
            sample_rate_hz: 48_000,
            channels: 2,
            ttl_sec: 120,
        }
    }
}

impl ServiceAnnouncement {
    /// Encodes this service advertisement as a deterministic DNS-label + TXT payload.
    pub fn encode_mdns_payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // DNS-style QNAME labels for `_cabaret-link._tcp.local.`
        encode_dns_name(CABARET_LINK_SERVICE, &mut out);
        out.extend_from_slice(&self.ipv4);
        out.extend_from_slice(&self.port.to_le_bytes());
        out.extend_from_slice(&self.ttl_sec.to_le_bytes());
        write_len_str(&self.instance_name, &mut out);
        write_len_str(&self.host_name, &mut out);

        let txt_entries = [
            format!("v=1"),
            format!("groups={}", self.groups.join(",")),
            format!("res={}x{}", self.width, self.height),
            format!("fps={}/{}", self.fps_num, self.fps_den.max(1)),
            format!("audio={}/{}", self.sample_rate_hz, self.channels),
        ];
        out.push(txt_entries.len() as u8);
        for entry in &txt_entries {
            write_len_str(entry, &mut out);
        }
        out
    }

    /// Decodes a service advertisement emitted by [`ServiceAnnouncement::encode_mdns_payload`].
    pub fn decode_mdns_payload(bytes: &[u8]) -> Result<Self, LinkError> {
        let mut cur = 0usize;
        let svc = decode_dns_name(bytes, &mut cur)?;
        if svc != "_cabaret-link._tcp.local" && svc != CABARET_LINK_SERVICE.trim_end_matches('.') {
            return Err(LinkError::BadMagic);
        }
        if cur + 10 > bytes.len() {
            return Err(LinkError::UnexpectedEof);
        }
        let ipv4 = [bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]];
        cur += 4;
        let port = u16::from_le_bytes(bytes[cur..cur + 2].try_into().map_err(|_| LinkError::UnexpectedEof)?);
        cur += 2;
        let ttl_sec = u32::from_le_bytes(bytes[cur..cur + 4].try_into().map_err(|_| LinkError::UnexpectedEof)?);
        cur += 4;
        let instance_name = read_len_str(bytes, &mut cur)?;
        let host_name = read_len_str(bytes, &mut cur)?;

        if cur >= bytes.len() {
            return Err(LinkError::UnexpectedEof);
        }
        let txt_count = bytes[cur] as usize;
        cur += 1;

        let mut groups = Vec::new();
        let mut width = 1920u32;
        let mut height = 1080u32;
        let mut fps_num = 60u32;
        let mut fps_den = 1u32;
        let mut sample_rate_hz = 48_000u32;
        let mut channels = 2u8;

        for _ in 0..txt_count {
            let entry = read_len_str(bytes, &mut cur)?;
            if let Some(val) = entry.strip_prefix("groups=") {
                if !val.is_empty() {
                    groups = val.split(',').map(String::from).collect();
                }
            } else if let Some(val) = entry.strip_prefix("res=") {
                if let Some((w_s, h_s)) = val.split_once('x') {
                    width = w_s.parse().unwrap_or(width);
                    height = h_s.parse().unwrap_or(height);
                }
            } else if let Some(val) = entry.strip_prefix("fps=") {
                if let Some((n_s, d_s)) = val.split_once('/') {
                    fps_num = n_s.parse().unwrap_or(fps_num);
                    fps_den = d_s.parse().unwrap_or(fps_den).max(1);
                }
            } else if let Some(val) = entry.strip_prefix("audio=") {
                if let Some((sr_s, ch_s)) = val.split_once('/') {
                    sample_rate_hz = sr_s.parse().unwrap_or(sample_rate_hz);
                    channels = ch_s.parse().unwrap_or(channels);
                }
            }
        }

        Ok(Self {
            instance_name,
            host_name,
            ipv4,
            port,
            groups,
            width,
            height,
            fps_num,
            fps_den,
            sample_rate_hz,
            channels,
            ttl_sec,
        })
    }
}

/// Wire pixel encoding format for CabaretLink video frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LinkPixelFormat {
    #[default]
    RleRgba8 = 1,
    RleYuv422 = 2,
}

/// Encoded LAN video frame payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkVideoFrame {
    pub width: u32,
    pub height: u32,
    pub format: LinkPixelFormat,
    pub rle_data: Vec<u8>,
}

impl LinkVideoFrame {
    /// Encodes a [`FrameBuffer`] using lossless fast 32-bit RLE (`RleRgba8` or `RleYuv422`).
    pub fn encode(frame: &FrameBuffer, format: LinkPixelFormat) -> Result<Self, LinkError> {
        let w = frame.width;
        let h = frame.height;
        if w == 0 || h == 0 {
            return Err(LinkError::InvalidDimensions);
        }
        let rgba8 = frame.to_srgb_rgba8();
        let rle_data = match format {
            LinkPixelFormat::RleRgba8 => rle_compress_words(&rgba8),
            LinkPixelFormat::RleYuv422 => {
                let yuv = rgba8_to_yuv422(&rgba8, w as usize, h as usize);
                rle_compress_words(&yuv)
            }
        };
        Ok(Self {
            width: w,
            height: h,
            format,
            rle_data,
        })
    }

    /// Decodes this [`LinkVideoFrame`] back into a linear-light [`FrameBuffer`].
    pub fn decode(&self) -> Result<FrameBuffer, LinkError> {
        if self.width == 0 || self.height == 0 {
            return Err(LinkError::InvalidDimensions);
        }
        let w = self.width as usize;
        let h = self.height as usize;
        match self.format {
            LinkPixelFormat::RleRgba8 => {
                let expected_bytes = w.checked_mul(h).and_then(|p| p.checked_mul(4)).ok_or(LinkError::InvalidDimensions)?;
                let rgba = rle_decompress_words(&self.rle_data, expected_bytes)?;
                Ok(FrameBuffer::from_srgb_rgba8(self.width, self.height, &rgba)?)
            }
            LinkPixelFormat::RleYuv422 => {
                let pairs_per_row = w.div_ceil(2);
                let expected_bytes = pairs_per_row
                    .checked_mul(h)
                    .and_then(|p| p.checked_mul(4))
                    .ok_or(LinkError::InvalidDimensions)?;
                let yuv = rle_decompress_words(&self.rle_data, expected_bytes)?;
                let rgba = yuv422_to_rgba8(&yuv, w, h);
                Ok(FrameBuffer::from_srgb_rgba8(self.width, self.height, &rgba)?)
            }
        }
    }
}

/// 48 kHz PCM audio frame transmitted over CabaretLink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkAudioFrame {
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub samples_i16: Vec<i16>,
}

impl LinkAudioFrame {
    /// Converts floating-point stereo samples `[[L, R]]` into 16-bit PCM `LinkAudioFrame`.
    pub fn from_stereo_f32(sample_rate_hz: u32, stereo: &[[f32; 2]]) -> Self {
        let mut samples_i16 = Vec::with_capacity(stereo.len() * 2);
        for &[l, r] in stereo {
            let ql = roundf((l.clamp(-1.0, 1.0)) * 32767.0) as i16;
            let qr = roundf((r.clamp(-1.0, 1.0)) * 32767.0) as i16;
            samples_i16.push(ql);
            samples_i16.push(qr);
        }
        Self {
            sample_rate_hz,
            channels: 2,
            samples_i16,
        }
    }

    /// Converts interleaved 16-bit PCM samples back into `[[L, R]]` floating-point stereo.
    pub fn to_stereo_f32(&self) -> Vec<[f32; 2]> {
        if self.channels == 1 {
            self.samples_i16
                .iter()
                .map(|&m| {
                    let v = m as f32 / 32767.0;
                    [v, v]
                })
                .collect()
        } else {
            let ch = self.channels.max(2) as usize;
            self.samples_i16
                .chunks_exact(ch)
                .map(|c| [c[0] as f32 / 32767.0, c[1] as f32 / 32767.0])
                .collect()
        }
    }
}

/// Program/Preview Tally signal integrated with [`cabaret_core::transitions::TallyState`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTally {
    pub program: bool,
    pub preview: bool,
    pub stream_name: String,
}

impl LinkTally {
    pub fn from_tally_state(state: TallyState, stream_name: impl Into<String>) -> Self {
        match state {
            TallyState::ProgramRed => Self {
                program: true,
                preview: false,
                stream_name: stream_name.into(),
            },
            TallyState::PreviewGreen => Self {
                program: false,
                preview: true,
                stream_name: stream_name.into(),
            },
            TallyState::None => Self {
                program: false,
                preview: false,
                stream_name: stream_name.into(),
            },
        }
    }

    pub const fn to_tally_state(&self) -> TallyState {
        if self.program {
            TallyState::ProgramRed
        } else if self.preview {
            TallyState::PreviewGreen
        } else {
            TallyState::None
        }
    }
}

/// Payload carried inside a framed CabaretLink LAN packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkPayload {
    Video(LinkVideoFrame),
    Audio(LinkAudioFrame),
    Tally(LinkTally),
    Announce(ServiceAnnouncement),
    Heartbeat,
}

/// Complete sequence-numbered, timestamped, CRC-32 framed CabaretLink packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkPacket {
    pub seq: u32,
    pub timestamp_us: i64,
    pub payload: LinkPayload,
}

impl LinkPacket {
    /// Encodes and frames this packet with magic `"CBL1"` and trailing IEEE CRC-32 checksum.
    pub fn encode(&self) -> Vec<u8> {
        let (kind, payload_bytes) = match &self.payload {
            LinkPayload::Video(v) => {
                let mut buf = Vec::with_capacity(9 + v.rle_data.len());
                buf.extend_from_slice(&v.width.to_le_bytes());
                buf.extend_from_slice(&v.height.to_le_bytes());
                buf.push(v.format as u8);
                buf.extend_from_slice(&v.rle_data);
                (1u8, buf)
            }
            LinkPayload::Audio(a) => {
                let mut buf = Vec::with_capacity(5 + a.samples_i16.len() * 2);
                buf.extend_from_slice(&a.sample_rate_hz.to_le_bytes());
                buf.push(a.channels);
                for &s in &a.samples_i16 {
                    buf.extend_from_slice(&s.to_le_bytes());
                }
                (2u8, buf)
            }
            LinkPayload::Tally(t) => {
                let mut buf = Vec::with_capacity(3 + t.stream_name.len());
                let flags = (t.program as u8) | ((t.preview as u8) << 1);
                buf.push(flags);
                write_len_str(&t.stream_name, &mut buf);
                (3u8, buf)
            }
            LinkPayload::Announce(ann) => (4u8, ann.encode_mdns_payload()),
            LinkPayload::Heartbeat => (5u8, Vec::new()),
        };

        let mut frame = Vec::with_capacity(LINK_HEADER_LEN + payload_bytes.len() + 4);
        frame.extend_from_slice(&LINK_PACKET_MAGIC);
        frame.push(LINK_PROTOCOL_VERSION);
        frame.push(kind);
        frame.extend_from_slice(&[0u8, 0u8]);
        frame.extend_from_slice(&self.seq.to_le_bytes());
        frame.extend_from_slice(&self.timestamp_us.to_le_bytes());
        frame.extend_from_slice(&(payload_bytes.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload_bytes);

        let crc = ieee_crc32(&frame);
        frame.extend_from_slice(&crc.to_le_bytes());
        frame
    }

    /// Validates header, payload length, and CRC-32 and decodes a single framed packet.
    pub fn decode(bytes: &[u8]) -> Result<Self, LinkError> {
        if bytes.len() < LINK_HEADER_LEN + 4 {
            return Err(LinkError::UnexpectedEof);
        }
        if bytes[0..4] != LINK_PACKET_MAGIC {
            return Err(LinkError::BadMagic);
        }
        let ver = bytes[4];
        if ver != LINK_PROTOCOL_VERSION {
            return Err(LinkError::UnsupportedVersion(ver));
        }
        let kind = bytes[5];
        let seq = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| LinkError::UnexpectedEof)?);
        let timestamp_us = i64::from_le_bytes(bytes[12..20].try_into().map_err(|_| LinkError::UnexpectedEof)?);
        let payload_len = u32::from_le_bytes(bytes[20..24].try_into().map_err(|_| LinkError::UnexpectedEof)?) as usize;

        let total_len = LINK_HEADER_LEN
            .checked_add(payload_len)
            .and_then(|l| l.checked_add(4))
            .ok_or(LinkError::UnexpectedEof)?;
        if bytes.len() < total_len {
            return Err(LinkError::UnexpectedEof);
        }

        let expected_crc = ieee_crc32(&bytes[..LINK_HEADER_LEN + payload_len]);
        let actual_crc = u32::from_le_bytes(
            bytes[LINK_HEADER_LEN + payload_len..total_len]
                .try_into()
                .map_err(|_| LinkError::UnexpectedEof)?,
        );
        if expected_crc != actual_crc {
            return Err(LinkError::ChecksumMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        let p = &bytes[LINK_HEADER_LEN..LINK_HEADER_LEN + payload_len];
        let payload = match kind {
            1 => {
                if p.len() < 9 {
                    return Err(LinkError::UnexpectedEof);
                }
                let width = u32::from_le_bytes(p[0..4].try_into().map_err(|_| LinkError::UnexpectedEof)?);
                let height = u32::from_le_bytes(p[4..8].try_into().map_err(|_| LinkError::UnexpectedEof)?);
                let format = match p[8] {
                    1 => LinkPixelFormat::RleRgba8,
                    2 => LinkPixelFormat::RleYuv422,
                    _ => return Err(LinkError::UnknownPacketKind(p[8])),
                };
                LinkPayload::Video(LinkVideoFrame {
                    width,
                    height,
                    format,
                    rle_data: p[9..].to_vec(),
                })
            }
            2 => {
                if p.len() < 5 {
                    return Err(LinkError::UnexpectedEof);
                }
                let sample_rate_hz = u32::from_le_bytes(p[0..4].try_into().map_err(|_| LinkError::UnexpectedEof)?);
                let channels = p[4];
                let pcm_bytes = &p[5..];
                if pcm_bytes.len() % 2 != 0 {
                    return Err(LinkError::UnexpectedEof);
                }
                let mut samples_i16 = Vec::with_capacity(pcm_bytes.len() / 2);
                for ch in pcm_bytes.chunks_exact(2) {
                    samples_i16.push(i16::from_le_bytes([ch[0], ch[1]]));
                }
                LinkPayload::Audio(LinkAudioFrame {
                    sample_rate_hz,
                    channels,
                    samples_i16,
                })
            }
            3 => {
                if p.is_empty() {
                    return Err(LinkError::UnexpectedEof);
                }
                let flags = p[0];
                let mut cur = 1usize;
                let stream_name = read_len_str(p, &mut cur)?;
                LinkPayload::Tally(LinkTally {
                    program: (flags & 1) != 0,
                    preview: (flags & 2) != 0,
                    stream_name,
                })
            }
            4 => LinkPayload::Announce(ServiceAnnouncement::decode_mdns_payload(p)?),
            5 => LinkPayload::Heartbeat,
            other => return Err(LinkError::UnknownPacketKind(other)),
        };

        Ok(Self {
            seq,
            timestamp_us,
            payload,
        })
    }
}

/// Incremental streaming socket deframer for coalesced or fragmented LAN TCP/UDP reads.
#[derive(Clone, Debug, Default)]
pub struct LinkStreamDecoder {
    buf: Vec<u8>,
}

impl LinkStreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_bytes(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    pub fn buffered_len(&self) -> usize {
        self.buf.len()
    }

    /// Attempts to deframe and validate the next complete packet from the stream buffer.
    pub fn next_packet(&mut self) -> Result<Option<LinkPacket>, LinkError> {
        // Resynchronize to LINK_PACKET_MAGIC if garbage bytes precede a frame.
        if let Some(pos) = self
            .buf
            .windows(4)
            .position(|w| w == LINK_PACKET_MAGIC)
        {
            if pos > 0 {
                self.buf.drain(..pos);
            }
        } else {
            if self.buf.len() > 3 {
                let keep = self.buf.len() - 3;
                self.buf.drain(..keep);
            }
            return Ok(None);
        }

        if self.buf.len() < LINK_HEADER_LEN + 4 {
            return Ok(None);
        }

        let payload_len = u32::from_le_bytes(
            self.buf[20..24]
                .try_into()
                .map_err(|_| LinkError::UnexpectedEof)?,
        ) as usize;
        let total_len = LINK_HEADER_LEN
            .checked_add(payload_len)
            .and_then(|l| l.checked_add(4))
            .ok_or(LinkError::UnexpectedEof)?;

        if self.buf.len() < total_len {
            return Ok(None);
        }

        let res = LinkPacket::decode(&self.buf[..total_len]);
        self.buf.drain(..total_len);
        res.map(Some)
    }
}

// =========================================================================
// Fast 32-bit RLE + BT.709 YUV422 Helpers + IEEE CRC-32
// =========================================================================

/// Compresses 4-byte aligned pixels/macropixels using deterministic PackBits-style 32-bit RLE.
pub fn rle_compress_words(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let words: Vec<[u8; 4]> = bytes
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    let n = words.len();
    let mut i = 0usize;

    while i < n {
        // Check for repeat run.
        let mut run = 1usize;
        while i + run < n && run < 128 && words[i + run] == words[i] {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | ((run - 1) as u8));
            out.extend_from_slice(&words[i]);
            i += run;
        } else {
            let start = i;
            let mut lit = 0usize;
            while i < n && lit < 128 {
                if i + 1 < n && words[i] == words[i + 1] {
                    break;
                }
                i += 1;
                lit += 1;
            }
            if lit == 0 {
                lit = 1;
                i += 1;
            }
            out.push((lit - 1) as u8);
            for w in &words[start..start + lit] {
                out.extend_from_slice(w);
            }
        }
    }
    out
}

/// Decompresses 32-bit RLE stream into `expected_bytes`.
pub fn rle_decompress_words(rle: &[u8], expected_bytes: usize) -> Result<Vec<u8>, LinkError> {
    let mut out = Vec::with_capacity(expected_bytes);
    let mut i = 0usize;
    while i < rle.len() {
        let hdr = rle[i];
        i += 1;
        let count = ((hdr & 0x7F) as usize) + 1;
        if (hdr & 0x80) != 0 {
            if i + 4 > rle.len() {
                return Err(LinkError::CorruptRleStream);
            }
            let word = [rle[i], rle[i + 1], rle[i + 2], rle[i + 3]];
            i += 4;
            for _ in 0..count {
                if out.len() + 4 > expected_bytes {
                    return Err(LinkError::CorruptRleStream);
                }
                out.extend_from_slice(&word);
            }
        } else {
            let need = count * 4;
            if i + need > rle.len() || out.len() + need > expected_bytes {
                return Err(LinkError::CorruptRleStream);
            }
            out.extend_from_slice(&rle[i..i + need]);
            i += need;
        }
    }
    if out.len() != expected_bytes {
        return Err(LinkError::CorruptRleStream);
    }
    Ok(out)
}

fn rgba8_to_yuv422(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let pairs = w.div_ceil(2);
    let mut out = Vec::with_capacity(pairs * h * 4);
    for y in 0..h {
        let row_off = y * w * 4;
        for px_pair in 0..pairs {
            let x0 = px_pair * 2;
            let x1 = (x0 + 1).min(w - 1);
            let p0 = &rgba[row_off + x0 * 4..row_off + x0 * 4 + 4];
            let p1 = &rgba[row_off + x1 * 4..row_off + x1 * 4 + 4];

            let (y0, cb0, cr0) = rgb8_to_ycbcr709(p0[0], p0[1], p0[2]);
            let (y1, cb1, cr1) = rgb8_to_ycbcr709(p1[0], p1[1], p1[2]);
            let cb = roundf(0.5 * (cb0 + cb1)).clamp(0.0, 255.0) as u8;
            let cr = roundf(0.5 * (cr0 + cr1)).clamp(0.0, 255.0) as u8;
            out.push(y0);
            out.push(cb);
            out.push(y1);
            out.push(cr);
        }
    }
    out
}

fn yuv422_to_rgba8(yuv: &[u8], w: usize, h: usize) -> Vec<u8> {
    let pairs = w.div_ceil(2);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row_off = y * pairs * 4;
        for px_pair in 0..pairs {
            let off = row_off + px_pair * 4;
            let y0 = yuv[off];
            let cb = yuv[off + 1];
            let y1 = yuv[off + 2];
            let cr = yuv[off + 3];
            let [r0, g0, b0] = ycbcr709_to_rgb8(y0, cb, cr);
            out.extend_from_slice(&[r0, g0, b0, 255]);
            if px_pair * 2 + 1 < w {
                let [r1, g1, b1] = ycbcr709_to_rgb8(y1, cb, cr);
                out.extend_from_slice(&[r1, g1, b1, 255]);
            }
        }
    }
    out
}

#[inline]
fn rgb8_to_ycbcr709(r: u8, g: u8, b: u8) -> (u8, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let y = clamp01(0.2126 * rf + 0.7152 * gf + 0.0722 * bf);
    let cb = ((bf - y) / 1.8556 + 0.5) * 255.0;
    let cr = ((rf - y) / 1.5748 + 0.5) * 255.0;
    (roundf(y * 255.0).clamp(0.0, 255.0) as u8, cb, cr)
}

#[inline]
fn ycbcr709_to_rgb8(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    let yf = y as f32 / 255.0;
    let cbf = (cb as f32 / 255.0) - 0.5;
    let crf = (cr as f32 / 255.0) - 0.5;
    let r = clamp01(yf + 1.5748 * crf);
    let b = clamp01(yf + 1.8556 * cbf);
    let g = clamp01((yf - 0.2126 * r - 0.0722 * b) / 0.7152);
    [
        roundf(r * 255.0) as u8,
        roundf(g * 255.0) as u8,
        roundf(b * 255.0) as u8,
    ]
}

fn encode_dns_name(name: &str, out: &mut Vec<u8>) {
    for label in name.trim_end_matches('.').split('.') {
        let bytes = label.as_bytes();
        let len = bytes.len().min(63);
        out.push(len as u8);
        out.extend_from_slice(&bytes[..len]);
    }
    out.push(0);
}

fn decode_dns_name(bytes: &[u8], cur: &mut usize) -> Result<String, LinkError> {
    let mut parts = Vec::new();
    loop {
        if *cur >= bytes.len() {
            return Err(LinkError::UnexpectedEof);
        }
        let len = bytes[*cur] as usize;
        *cur += 1;
        if len == 0 {
            break;
        }
        if *cur + len > bytes.len() {
            return Err(LinkError::UnexpectedEof);
        }
        let label = core::str::from_utf8(&bytes[*cur..*cur + len]).map_err(|_| LinkError::InvalidUtf8)?;
        parts.push(label);
        *cur += len;
    }
    Ok(parts.join("."))
}

fn write_len_str(s: &str, out: &mut Vec<u8>) {
    let b = s.as_bytes();
    let len = b.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&b[..len]);
}

fn read_len_str(bytes: &[u8], cur: &mut usize) -> Result<String, LinkError> {
    if *cur + 2 > bytes.len() {
        return Err(LinkError::UnexpectedEof);
    }
    let len = u16::from_le_bytes(bytes[*cur..*cur + 2].try_into().map_err(|_| LinkError::UnexpectedEof)?) as usize;
    *cur += 2;
    if *cur + len > bytes.len() {
        return Err(LinkError::UnexpectedEof);
    }
    let s = core::str::from_utf8(&bytes[*cur..*cur + len]).map_err(|_| LinkError::InvalidUtf8)?;
    *cur += len;
    Ok(String::from(s))
}

/// Deterministic pure-Rust IEEE 802.3 CRC-32 checksum.
pub fn ieee_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
