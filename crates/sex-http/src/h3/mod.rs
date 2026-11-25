// SPDX-License-Identifier: GPL-3.0-only
//! HTTP/3 (RFC 9114) & QUIC (RFC 9000) sans-I/O framing foundation, QPACK static
//! header compression (RFC 9204), `Alt-Svc: h3` upgrade hint parser, and `H3Client`
//! state machine (`no_std` + `alloc`).

pub mod frame;
pub mod qpack;
pub mod quic;
pub mod varint;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use frame::{
    FrameError, H3Frame, SETTINGS_MAX_FIELD_SECTION_SIZE, SETTINGS_QPACK_BLOCKED_STREAMS,
    SETTINGS_QPACK_MAX_TABLE_CAPACITY, STREAM_CONTROL, STREAM_QPACK_DECODER, STREAM_QPACK_ENCODER,
};
use qpack::{QpackError, MAX_FIELD_SECTION_BYTES};
use quic::QuicFrame;
pub use crate::h2::Header;

/// Client-initiated unidirectional stream IDs (RFC 9000 Section 2.1: `4*n + 2`).
pub const CLIENT_CONTROL_STREAM: u64 = 2;
pub const CLIENT_QPACK_ENCODER_STREAM: u64 = 6;
pub const CLIENT_QPACK_DECODER_STREAM: u64 = 10;

/// Parsed `Alt-Svc` upgrade hint advertising HTTP/3 (`h3="..."` or `h3-29="..."`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AltSvcHint {
    /// Alternate host if specified, or `None` when the same origin host is used (`h3=":443"`).
    pub host: Option<Vec<u8>>,
    /// UDP port advertised for HTTP/3 / QUIC.
    pub port: u16,
    /// Optional `ma=<seconds>` max-age parameter.
    pub max_age: Option<u32>,
}

/// Parses an HTTP `Alt-Svc` response header value and returns the first HTTP/3 (`h3`)
/// upgrade hint, if present. Supports comma-separated entries such as
/// `h2=":443"; ma=3600, h3=":443"; ma=2592000` or `h3="alt.example.com:8443"`.
pub fn parse_alt_svc(value: &[u8]) -> Option<AltSvcHint> {
    let text = core::str::from_utf8(value).ok()?;
    if text.trim().eq_ignore_ascii_case("clear") {
        return None;
    }
    for entry in text.split(',') {
        let mut params = entry.split(';');
        let first = params.next()?.trim();
        let (proto, raw_authority) = first.split_once('=')?;
        let proto = proto.trim();
        if proto != "h3" && !proto.starts_with("h3-") {
            continue;
        }
        let authority = raw_authority.trim().trim_matches('"');
        let (host_str, port_str) = authority.rsplit_once(':')?;
        let port: u16 = port_str.parse().ok()?;
        if port == 0 {
            continue;
        }
        let host = if host_str.is_empty() {
            None
        } else {
            Some(host_str.as_bytes().to_vec())
        };
        let mut max_age = None;
        for param in params {
            if let Some((k, v)) = param.trim().split_once('=') {
                if k.trim().eq_ignore_ascii_case("ma") {
                    max_age = v.trim().parse::<u32>().ok();
                }
            }
        }
        return Some(AltSvcHint { host, port, max_age });
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H3Error {
    Frame(FrameError),
    Qpack(QpackError),
    GoingAway,
    MissingSettings,
    UnexpectedFrame,
    DuplicateControlStream,
    InvalidStream,
}

impl From<FrameError> for H3Error {
    fn from(e: FrameError) -> Self { Self::Frame(e) }
}
impl From<QpackError> for H3Error {
    fn from(e: QpackError) -> Self { Self::Qpack(e) }
}

/// Events produced by `H3Client` as server QUIC stream data arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum H3Event {
    Settings(Vec<(u64, u64)>),
    Headers { stream: u64, status: u16, headers: Vec<Header>, trailers: bool },
    Data { stream: u64, bytes: Vec<u8> },
    End { stream: u64 },
    GoAway { stream_id: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UniStreamKind {
    Unknown,
    Control,
    Push,
    QpackEncoder,
    QpackDecoder,
    Ignored,
}

struct UniStreamState {
    kind: UniStreamKind,
    buf: Vec<u8>,
}

struct RequestStreamState {
    got_headers: bool,
    buf: Vec<u8>,
}

/// Sans-I/O HTTP/3 client state machine (RFC 9114).
///
/// Emits `QuicFrame::Stream` frames via `poll_send()` for control/QPACK streams and
/// bidirectional HTTP/3 request streams (`0, 4, 8, ...`), and ingests server stream
/// bytes via `receive_stream()` / `receive_quic_frame()`.
pub struct H3Client {
    outbound: VecDeque<QuicFrame>,
    events: VecDeque<H3Event>,
    next_bidi_stream: u64,
    requests: BTreeMap<u64, RequestStreamState>,
    uni_streams: BTreeMap<u64, UniStreamState>,
    peer_control_stream: Option<u64>,
    peer_settings: Option<Vec<(u64, u64)>>,
    going_away: Option<u64>,
}

impl Default for H3Client {
    fn default() -> Self { Self::new() }
}

impl H3Client {
    /// Creates a new HTTP/3 client and queues the mandatory unidirectional streams:
    /// control stream (with our SETTINGS) plus QPACK encoder & decoder streams.
    pub fn new() -> Self {
        let mut client = Self {
            outbound: VecDeque::new(),
            events: VecDeque::new(),
            next_bidi_stream: 0,
            requests: BTreeMap::new(),
            uni_streams: BTreeMap::new(),
            peer_control_stream: None,
            peer_settings: None,
            going_away: None,
        };
        // Control stream (type 0x00) + SETTINGS frame.
        let mut ctrl = Vec::new();
        let _ = varint::encode(STREAM_CONTROL, &mut ctrl);
        let settings = H3Frame::Settings(alloc::vec![
            (SETTINGS_QPACK_MAX_TABLE_CAPACITY, 0),
            (SETTINGS_MAX_FIELD_SECTION_SIZE, MAX_FIELD_SECTION_BYTES as u64),
            (SETTINGS_QPACK_BLOCKED_STREAMS, 0),
        ]);
        let _ = settings.encode(&mut ctrl);
        client.outbound.push_back(QuicFrame::Stream {
            stream_id: CLIENT_CONTROL_STREAM,
            offset: 0,
            fin: false,
            data: ctrl,
        });
        // QPACK encoder stream (type 0x02) & decoder stream (type 0x03).
        let mut enc = Vec::new();
        let _ = varint::encode(STREAM_QPACK_ENCODER, &mut enc);
        client.outbound.push_back(QuicFrame::Stream {
            stream_id: CLIENT_QPACK_ENCODER_STREAM,
            offset: 0,
            fin: false,
            data: enc,
        });
        let mut dec = Vec::new();
        let _ = varint::encode(STREAM_QPACK_DECODER, &mut dec);
        client.outbound.push_back(QuicFrame::Stream {
            stream_id: CLIENT_QPACK_DECODER_STREAM,
            offset: 0,
            fin: false,
            data: dec,
        });
        client
    }

    pub fn poll_send(&mut self) -> Option<QuicFrame> { self.outbound.pop_front() }
    pub fn next_event(&mut self) -> Option<H3Event> { self.events.pop_front() }
    pub fn peer_settings(&self) -> Option<&[(u64, u64)]> { self.peer_settings.as_deref() }
    pub fn active_requests(&self) -> usize { self.requests.len() }

    /// Starts a new HTTP/3 request on the next client-initiated bidirectional QUIC stream
    /// (`0, 4, 8, ...`). Returns the allocated stream ID.
    pub fn request(
        &mut self,
        method: &[u8],
        scheme: &[u8],
        authority: &[u8],
        path: &[u8],
        headers: &[(&[u8], &[u8])],
        body: Option<&[u8]>,
    ) -> Result<u64, H3Error> {
        if self.going_away.is_some() {
            return Err(H3Error::GoingAway);
        }
        let stream_id = self.next_bidi_stream;
        self.next_bidi_stream = self.next_bidi_stream.checked_add(4).ok_or(H3Error::GoingAway)?;

        let mut all: Vec<(&[u8], &[u8])> = alloc::vec![
            (b":method", method),
            (b":scheme", scheme),
            (b":authority", authority),
            (b":path", path),
        ];
        all.extend(headers.iter().filter(|(n, _)| {
            !matches!(
                *n,
                b"connection" | b"keep-alive" | b"proxy-connection" | b"transfer-encoding" | b"upgrade" | b"host"
            )
        }));

        let mut qpack_block = Vec::new();
        qpack::encode_field_section(&all, &mut qpack_block);

        let mut stream_bytes = Vec::new();
        H3Frame::Headers(qpack_block).encode(&mut stream_bytes)?;
        if let Some(b) = body {
            if !b.is_empty() {
                H3Frame::Data(b.to_vec()).encode(&mut stream_bytes)?;
            }
        }
        self.requests.insert(stream_id, RequestStreamState { got_headers: false, buf: Vec::new() });
        self.outbound.push_back(QuicFrame::Stream {
            stream_id,
            offset: 0,
            fin: true,
            data: stream_bytes,
        });
        Ok(stream_id)
    }

    /// Sends a GOAWAY frame on the client control stream.
    pub fn goaway(&mut self, stream_id: u64) -> Result<(), H3Error> {
        let mut data = Vec::new();
        H3Frame::GoAway(stream_id).encode(&mut data)?;
        self.going_away = Some(stream_id);
        self.outbound.push_back(QuicFrame::Stream {
            stream_id: CLIENT_CONTROL_STREAM,
            offset: 0,
            fin: false,
            data,
        });
        Ok(())
    }

    /// Feeds an incoming QUIC frame to the HTTP/3 state machine.
    pub fn receive_quic_frame(&mut self, frame: &QuicFrame) -> Result<(), H3Error> {
        if let QuicFrame::Stream { stream_id, fin, data, .. } = frame {
            self.receive_stream(*stream_id, data, *fin)?;
        }
        Ok(())
    }

    /// Feeds bytes arriving on QUIC stream `stream_id`.
    pub fn receive_stream(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<(), H3Error> {
        // RFC 9000 Section 2.1: bit 1 (`stream_id & 0x02`) distinguishes unidirectional streams.
        let is_uni = (stream_id & 0x02) != 0;
        if is_uni {
            self.receive_uni(stream_id, data)
        } else {
            self.receive_bidi(stream_id, data, fin)
        }
    }

    fn receive_uni(&mut self, stream_id: u64, data: &[u8]) -> Result<(), H3Error> {
        let state = self.uni_streams.entry(stream_id).or_insert(UniStreamState {
            kind: UniStreamKind::Unknown,
            buf: Vec::new(),
        });
        state.buf.extend_from_slice(data);
        if state.kind == UniStreamKind::Unknown {
            match varint::decode(&state.buf) {
                Ok((stype, used)) => {
                    state.buf.drain(..used);
                    state.kind = match stype {
                        STREAM_CONTROL => {
                            if self.peer_control_stream.is_some() {
                                return Err(H3Error::DuplicateControlStream);
                            }
                            self.peer_control_stream = Some(stream_id);
                            UniStreamKind::Control
                        }
                        frame::STREAM_PUSH => UniStreamKind::Push,
                        STREAM_QPACK_ENCODER => UniStreamKind::QpackEncoder,
                        STREAM_QPACK_DECODER => UniStreamKind::QpackDecoder,
                        _ => UniStreamKind::Ignored,
                    };
                }
                Err(varint::VarintError::Incomplete) => return Ok(()),
                Err(e) => return Err(H3Error::Frame(FrameError::from(e))),
            }
        }
        if state.kind != UniStreamKind::Control {
            // Static-only QPACK and unknown/GREASE streams require no further framing here.
            state.buf.clear();
            return Ok(());
        }
        loop {
            let decoded = {
                let st = self.uni_streams.get(&stream_id).unwrap();
                if st.buf.is_empty() {
                    break;
                }
                H3Frame::decode(&st.buf)
            };
            match decoded {
                Ok((frame, used)) => {
                    self.uni_streams.get_mut(&stream_id).unwrap().buf.drain(..used);
                    match frame {
                        H3Frame::Settings(pairs) => {
                            if self.peer_settings.is_some() {
                                return Err(H3Error::UnexpectedFrame);
                            }
                            self.peer_settings = Some(pairs.clone());
                            self.events.push_back(H3Event::Settings(pairs));
                        }
                        H3Frame::GoAway(id) => {
                            if self.peer_settings.is_none() {
                                return Err(H3Error::MissingSettings);
                            }
                            self.going_away = Some(id);
                            self.events.push_back(H3Event::GoAway { stream_id: id });
                        }
                        H3Frame::MaxPushId(_) | H3Frame::CancelPush(_) | H3Frame::Unknown { .. } => {
                            if self.peer_settings.is_none() {
                                return Err(H3Error::MissingSettings);
                            }
                        }
                        H3Frame::Data(_) | H3Frame::Headers(_) | H3Frame::PushPromise { .. } => {
                            return Err(H3Error::UnexpectedFrame);
                        }
                    }
                }
                Err(FrameError::Incomplete) => break,
                Err(e) => return Err(H3Error::Frame(e)),
            }
        }
        Ok(())
    }

    fn receive_bidi(&mut self, stream_id: u64, data: &[u8], fin: bool) -> Result<(), H3Error> {
        let Some(req) = self.requests.get_mut(&stream_id) else {
            return Err(H3Error::InvalidStream);
        };
        req.buf.extend_from_slice(data);
        loop {
            let decoded = {
                let r = self.requests.get(&stream_id).unwrap();
                if r.buf.is_empty() {
                    break;
                }
                H3Frame::decode(&r.buf)
            };
            match decoded {
                Ok((frame, used)) => {
                    let r = self.requests.get_mut(&stream_id).unwrap();
                    r.buf.drain(..used);
                    match frame {
                        H3Frame::Headers(block) => {
                            let headers = qpack::decode_field_section(&block)?;
                            let trailers = r.got_headers;
                            let mut status = 0u16;
                            if !trailers {
                                r.got_headers = true;
                                for (n, v) in &headers {
                                    if n == b":status" {
                                        if let Ok(s) = core::str::from_utf8(v) {
                                            status = s.parse().unwrap_or(0);
                                        }
                                    }
                                }
                            }
                            self.events.push_back(H3Event::Headers { stream: stream_id, status, headers, trailers });
                        }
                        H3Frame::Data(bytes) => {
                            if !r.got_headers {
                                return Err(H3Error::UnexpectedFrame);
                            }
                            self.events.push_back(H3Event::Data { stream: stream_id, bytes });
                        }
                        H3Frame::Unknown { .. } => {}
                        _ => return Err(H3Error::UnexpectedFrame),
                    }
                }
                Err(FrameError::Incomplete) => break,
                Err(e) => return Err(H3Error::Frame(e)),
            }
        }
        if fin {
            self.requests.remove(&stream_id);
            self.events.push_back(H3Event::End { stream: stream_id });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use quic::{LongPacketType, PacketHeader, QUIC_VERSION_1};

    #[test]
    fn quic_varint_roundtrips_all_four_widths_and_rfc9000_vectors() {
        // RFC 9000 Section A.1 sample vectors.
        assert_eq!(varint::decode(&[0x25]), Ok((37, 1)));
        assert_eq!(varint::decode(&[0x7b, 0xbd]), Ok((15293, 2)));
        assert_eq!(varint::decode(&[0x9d, 0x7f, 0x3e, 0x7d]), Ok((494_878_333, 4)));
        assert_eq!(
            varint::decode(&[0xc2, 0x19, 0x7c, 0x5e, 0xff, 0x14, 0xe8, 0x8c]),
            Ok((151_288_809_941_952_652, 8))
        );
        for &(val, expected_len) in &[
            (0u64, 1usize),
            (63, 1),
            (64, 2),
            (16383, 2),
            (16384, 4),
            (1_073_741_823, 4),
            (1_073_741_824, 8),
            (varint::MAX_VARINT, 8),
        ] {
            let mut buf = Vec::new();
            assert_eq!(varint::encode(val, &mut buf), Ok(expected_len));
            assert_eq!(buf.len(), expected_len);
            assert_eq!(varint::decode(&buf), Ok((val, expected_len)));
            assert_eq!(varint::decode(&buf[..expected_len - 1]), Err(varint::VarintError::Incomplete));
        }
        let mut buf = Vec::new();
        assert_eq!(varint::encode(varint::MAX_VARINT + 1, &mut buf), Err(varint::VarintError::Overflow));
    }

    #[test]
    fn quic_headers_and_frames_roundtrip() {
        let long = PacketHeader::Long {
            packet_type: LongPacketType::Initial,
            version: QUIC_VERSION_1,
            dcid: vec![1, 2, 3, 4, 5, 6, 7, 8],
            scid: vec![9, 10, 11, 12],
            token: vec![0xAA, 0xBB],
            payload_len: 1200,
        };
        let mut wire = Vec::new();
        long.encode(&mut wire).unwrap();
        assert_eq!(PacketHeader::decode(&wire, 8), Ok((long, wire.len())));

        let short = PacketHeader::Short { spin: true, key_phase: true, dcid: vec![1, 2, 3, 4] };
        wire.clear();
        short.encode(&mut wire).unwrap();
        assert_eq!(PacketHeader::decode(&wire, 4), Ok((short, wire.len())));

        let frames = vec![
            QuicFrame::Padding,
            QuicFrame::Ping,
            QuicFrame::Ack { largest_acked: 42, ack_delay: 10, first_ack_range: 5, ranges: vec![(1, 2)] },
            QuicFrame::Crypto { offset: 0, data: b"client_hello".to_vec() },
            QuicFrame::Stream { stream_id: 4, offset: 128, fin: true, data: b"h3-data".to_vec() },
            QuicFrame::MaxData { max_data: 1_048_576 },
            QuicFrame::MaxStreamData { stream_id: 0, max_stream_data: 65536 },
            QuicFrame::ConnectionClose { is_app: true, error_code: frame::H3_NO_ERROR, frame_type: None, reason: b"bye".to_vec() },
            QuicFrame::ConnectionClose { is_app: false, error_code: 0x0a, frame_type: Some(0x06), reason: b"proto".to_vec() },
        ];
        for f in frames {
            let mut buf = Vec::new();
            f.encode(&mut buf).unwrap();
            assert_eq!(QuicFrame::decode(&buf), Ok((f, buf.len())));
        }
    }

    #[test]
    fn qpack_static_table_encodes_and_decodes_headers_and_huffman() {
        let input: &[(&[u8], &[u8])] = &[
            (b":method", b"GET"),
            (b":scheme", b"https"),
            (b":authority", b"sexos.example"),
            (b":path", b"/index.html"),
            (b"accept", b"*/*"),
            (b"user-agent", b"SexOS Browser"),
            (b"x-custom-header", b"value-123"),
        ];
        let mut encoded = Vec::new();
        qpack::encode_field_section(input, &mut encoded);
        // Prefix is [0x00, 0x00] (RIC=0, DeltaBase=0).
        assert_eq!(&encoded[..2], &[0x00, 0x00]);
        let decoded = qpack::decode_field_section(&encoded).unwrap();
        let expected: Vec<Header> = input.iter().map(|(n, v)| (n.to_vec(), v.to_vec())).collect();
        assert_eq!(decoded, expected);

        // Verify Huffman-encoded value decoding (static name index 0 = :authority, H=1).
        let mut huff_section = vec![0x00, 0x00, 0x50]; // static name ref index 0 (:authority)
        let mut huff_val = Vec::new();
        crate::h2::hpack::huffman_encode(&mut huff_val, b"www.example.com");
        crate::h2::hpack::encode_integer(&mut huff_section, 0x80, 7, huff_val.len());
        huff_section.extend_from_slice(&huff_val);
        let decoded_huff = qpack::decode_field_section(&huff_section).unwrap();
        assert_eq!(decoded_huff, vec![(b":authority".to_vec(), b"www.example.com".to_vec())]);
    }

    #[test]
    fn h3_frames_roundtrip_and_reject_h2_reserved_types() {
        let frames = vec![
            H3Frame::Data(b"hello world".to_vec()),
            H3Frame::Headers(vec![0x00, 0x00, 0xD9]),
            H3Frame::Settings(vec![(SETTINGS_QPACK_MAX_TABLE_CAPACITY, 0), (SETTINGS_MAX_FIELD_SECTION_SIZE, 65536)]),
            H3Frame::GoAway(8),
            H3Frame::MaxPushId(4),
            H3Frame::CancelPush(2),
        ];
        for f in frames {
            let mut buf = Vec::new();
            f.encode(&mut buf).unwrap();
            assert_eq!(H3Frame::decode(&buf), Ok((f, buf.len())));
        }
        // Reserved HTTP/2 frame type 0x02 (PRIORITY) must be rejected.
        assert_eq!(H3Frame::decode(&[0x02, 0x00]), Err(FrameError::ReservedType(0x02)));
    }

    #[test]
    fn h3_client_exchanges_control_settings_and_multiplexed_requests() {
        let mut client = H3Client::new();
        // Initial 3 unidirectional streams queued: control, qpack encoder, qpack decoder.
        let ctrl = client.poll_send().unwrap();
        assert!(matches!(ctrl, QuicFrame::Stream { stream_id: CLIENT_CONTROL_STREAM, fin: false, .. }));
        assert!(matches!(client.poll_send().unwrap(), QuicFrame::Stream { stream_id: CLIENT_QPACK_ENCODER_STREAM, .. }));
        assert!(matches!(client.poll_send().unwrap(), QuicFrame::Stream { stream_id: CLIENT_QPACK_DECODER_STREAM, .. }));
        assert!(client.poll_send().is_none());

        // Server opens control stream (stream_id = 3) with SETTINGS.
        let mut srv_ctrl = Vec::new();
        varint::encode(STREAM_CONTROL, &mut srv_ctrl).unwrap();
        H3Frame::Settings(vec![(SETTINGS_MAX_FIELD_SECTION_SIZE, 131072)]).encode(&mut srv_ctrl).unwrap();
        client.receive_stream(3, &srv_ctrl, false).unwrap();
        assert_eq!(client.next_event(), Some(H3Event::Settings(vec![(SETTINGS_MAX_FIELD_SECTION_SIZE, 131072)])));

        // Issue two concurrent HTTP/3 requests (streams 0 and 4).
        let s0 = client.request(b"GET", b"https", b"sexos.org", b"/", &[], None).unwrap();
        let s4 = client.request(b"POST", b"https", b"sexos.org", b"/api", &[(b"content-type", b"application/json")], Some(b"{}")).unwrap();
        assert_eq!((s0, s4), (0, 4));
        assert_eq!(client.active_requests(), 2);
        assert!(matches!(client.poll_send(), Some(QuicFrame::Stream { stream_id: 0, fin: true, .. })));
        assert!(matches!(client.poll_send(), Some(QuicFrame::Stream { stream_id: 4, fin: true, .. })));

        // Server responds on stream 0 in pieces.
        let mut resp_hdr = Vec::new();
        qpack::encode_field_section(&[(b":status", b"200"), (b"content-type", b"text/html; charset=utf-8")], &mut resp_hdr);
        let mut resp_wire = Vec::new();
        H3Frame::Headers(resp_hdr).encode(&mut resp_wire).unwrap();
        H3Frame::Data(b"<h1>HTTP/3!</h1>".to_vec()).encode(&mut resp_wire).unwrap();
        client.receive_stream(0, &resp_wire[..5], false).unwrap();
        client.receive_stream(0, &resp_wire[5..], true).unwrap();
        match client.next_event() {
            Some(H3Event::Headers { stream: 0, status: 200, trailers: false, .. }) => {}
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(client.next_event(), Some(H3Event::Data { stream: 0, bytes: b"<h1>HTTP/3!</h1>".to_vec() }));
        assert_eq!(client.next_event(), Some(H3Event::End { stream: 0 }));
        assert_eq!(client.active_requests(), 1);
    }

    #[test]
    fn alt_svc_parser_extracts_h3_hints() {
        assert_eq!(
            parse_alt_svc(b"h3=\":443\"; ma=2592000"),
            Some(AltSvcHint { host: None, port: 443, max_age: Some(2_592_000) })
        );
        assert_eq!(
            parse_alt_svc(b"h2=\":443\"; ma=60, h3=\"quic.example.com:8443\"; ma=86400"),
            Some(AltSvcHint { host: Some(b"quic.example.com".to_vec()), port: 8443, max_age: Some(86400) })
        );
        assert_eq!(parse_alt_svc(b"h3-29=\":443\""), Some(AltSvcHint { host: None, port: 443, max_age: None }));
        assert_eq!(parse_alt_svc(b"clear"), None);
        assert_eq!(parse_alt_svc(b"h2=\":443\""), None);
    }
}
