//! A simulated A2DP headset end to end, through a scripted controller:
//!
//! - the host pages the headset, pairs (Secure Simple Pairing, Just Works), encrypts,
//!   finds its A2DP Sink with SDP (the headset answers in two parts, a continuation),
//!   discovers its stream end points (a video sink, a busy audio sink, a free one),
//!   reads the free one's capabilities, configures 48 kHz joint stereo SBC, opens the
//!   stream and its transport channel and starts it;
//! - PCM played into the host comes out of the headset's own SBC decoder (the
//!   floating-point reference in `common/sbc_ref.rs`) as the same tone;
//! - the controller has four ACL buffers and gives them back only when the test says:
//!   the host never sends more than it has credits for;
//! - suspend, resume, close; encryption off stops the audio;
//! - the headset goes away and pages us later: the stored link key, the headset sets
//!   the stream up itself (we are acceptor) and audio flows again;
//! - malformed SDP and AVDTP from the peer: rejects and failures, never a panic.
mod common;
use common::sbc_ref::Decoder;
use sex_bt::a2dp;
use sex_bt::avdtp::{self, category, kind, signal};
use sex_bt::hci::{self, op as hop, Addr};
use sex_bt::host::{code, Host, Note};
use sex_bt::l2cap::{self, ChanEvent, Channels, Reassembly};
use sex_bt::report::{Found, Transport};
use sex_bt::sbc::{self, Allocation, Mode};
use sex_bt::sdp;
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

const HOST_ADDR: [u8; 6] = [0x01, 0x00, 0x5e, 0x10, 0x87, 0x80];
const HEADSET: Addr = Addr([0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
const ACL_BUFFERS: usize = 4;

/// The headset's SDP record, written out by hand (not with `sdp::Element`):
/// a list of one attribute list: 0x0001 [0x110B], 0x0004 [[0x0100, 0x0019],
/// [0x0019, 0x0103]], 0x0009 [[0x110D, 0x0103]], 0x0311 0x0001.
const SINK_RECORD: &[u8] = &[
    0x35, 0x32, // all records
    0x35, 0x30, // one record's attributes
    0x09, 0x00, 0x01, 0x35, 0x03, 0x19, 0x11, 0x0B,
    0x09, 0x00, 0x04, 0x35, 0x10, 0x35, 0x06, 0x19, 0x01, 0x00, 0x09, 0x00, 0x19, 0x35, 0x06, 0x19, 0x00, 0x19, 0x09, 0x01, 0x03,
    0x09, 0x00, 0x09, 0x35, 0x08, 0x35, 0x06, 0x19, 0x11, 0x0D, 0x09, 0x01, 0x03,
    0x09, 0x03, 0x11, 0x09, 0x00, 0x01,
];

struct Headset {
    chans: Channels,
    link_key: Option<[u8; 16]>,
    signal: Option<u16>,
    media: Option<u16>,
    /// Signals it received as commands, in order.
    commands: Vec<u8>,
    /// Its configuration (codec info element) and state.
    config: Option<[u8; 4]>,
    streaming: bool,
    decoder: Decoder,
    left: Vec<f64>,
    right: Vec<f64>,
    seqs: Vec<u16>,
    stamps: Vec<u32>,
    frames: usize,
    /// It sets the stream up itself (after paging us).
    initiator: bool,
    label: u8,
    /// Its own commands' replies (signal, accepted).
    replies: Vec<(u8, bool)>,
    /// Raw signaling packets it got (for checks on rejects).
    raw: Vec<Vec<u8>>,
    sdp_requests: usize,
    /// Only record what arrives (the malformed-input test).
    quiet: bool,
}

impl Headset {
    fn new() -> Headset {
        Headset { chans: Channels::default(), link_key: None, signal: None, media: None, commands: vec![], config: None, streaming: false,
            decoder: Decoder::new(), left: vec![], right: vec![], seqs: vec![], stamps: vec![], frames: 0, initiator: false, label: 0,
            replies: vec![], raw: vec![], sdp_requests: 0, quiet: false }
    }
}

struct Controller {
    out: VecDeque<Vec<u8>>,
    opcodes: Vec<u16>,
    hs: Headset,
    rx: Reassembly,
    handle: u16,
    /// Hold the buffers' completions until `release`.
    hold: bool,
    outstanding: usize,
    max_outstanding: usize,
    held: usize,
}

impl Controller {
    fn new() -> Controller {
        Controller { out: VecDeque::new(), opcodes: vec![], hs: Headset::new(), rx: Reassembly::default(), handle: 0x000b, hold: false,
            outstanding: 0, max_outstanding: 0, held: 0 }
    }
    fn cc(&mut self, opcode: u16, ret: &[u8]) { self.out.push_back(hci::command_complete(opcode, ret)); }
    fn status(&mut self, opcode: u16) { self.out.push_back(hci::command_status(opcode, 0)); }
    fn ev(&mut self, code: u8, p: &[u8]) { self.out.push_back(hci::event(code, p)); }
    fn send(&mut self, cid: u16, pdu: &[u8]) { for p in l2cap::fragment(self.handle, &l2cap::frame(cid, pdu), 1021) { self.out.push_back(p); } }
    fn completed(&mut self, n: usize) {
        let h = self.handle;
        self.ev(hci::ev::NUMBER_OF_COMPLETED_PACKETS, &[1, h as u8, (h >> 8) as u8, n as u8, 0]);
    }
    fn release(&mut self) {
        let n = self.held;
        self.held = 0;
        self.outstanding -= n;
        if n > 0 { self.completed(n); }
    }

    fn signal_remote(&self) -> u16 { self.hs.chans.by_local(self.hs.signal.unwrap()).unwrap().remote }

    /// The headset sends one AVDTP message (single packet).
    fn avdtp(&mut self, label: u8, kind: u8, sig: u8, params: &[u8]) {
        let mut p = vec![label << 4 | kind, sig];
        p.extend_from_slice(params);
        let r = self.signal_remote();
        self.send(r, &p);
    }
    /// The headset sends a command of its own (as initiator).
    fn hs_command(&mut self, sig: u8, params: &[u8]) {
        self.hs.label = (self.hs.label + 1) & 0x0F;
        let l = self.hs.label;
        self.avdtp(l, kind::COMMAND, sig, params);
    }

    fn chan_events(&mut self, events: Vec<ChanEvent>) {
        for e in events {
            match e {
                ChanEvent::Send(p) => self.send(l2cap::CID_SIGNALING, &p),
                ChanEvent::Open { psm: avdtp::PSM, local } => {
                    if self.hs.signal.is_none() || self.hs.signal == Some(local) {
                        self.hs.signal = Some(local);
                        if self.hs.initiator { self.hs_command(signal::DISCOVER, &[]); }
                    } else {
                        self.hs.media = Some(local);
                        // As initiator the headset starts the stream once its transport is open.
                        if self.hs.initiator { self.hs_command(signal::START, &[1 << 2]); }
                    }
                }
                ChanEvent::Closed { local, .. } => {
                    if self.hs.media == Some(local) { self.hs.media = None; self.hs.streaming = false; }
                    if self.hs.signal == Some(local) { self.hs.signal = None; }
                }
                _ => {}
            }
        }
    }

    /// SDP: the A2DP Sink search, answered in two parts.
    fn sdp(&mut self, remote: u16, req: &[u8]) {
        self.hs.sdp_requests += 1;
        assert_eq!(req[0], sdp::pdu::SERVICE_SEARCH_ATTRIBUTE_REQ);
        let tid = [req[1], req[2]];
        // Pattern: one UUID16, 0x110B.
        assert_eq!(&req[5..10], &[0x35, 0x03, 0x19, 0x11, 0x0B]);
        let cont_len = *req.last().unwrap();
        let (part, cont): (&[u8], &[u8]) = if cont_len == 0 { (&SINK_RECORD[..20], &[2, 0xAB, 0xCD]) } else {
            assert_eq!(&req[req.len() - 3..], &[2, 0xAB, 0xCD], "our continuation state comes back");
            (&SINK_RECORD[20..], &[0])
        };
        let mut params = (part.len() as u16).to_be_bytes().to_vec();
        params.extend_from_slice(part);
        params.extend_from_slice(cont);
        let mut r = vec![sdp::pdu::SERVICE_SEARCH_ATTRIBUTE_RSP, tid[0], tid[1]];
        r.extend_from_slice(&(params.len() as u16).to_be_bytes());
        r.extend_from_slice(&params);
        self.send(remote, &r);
    }

    /// AVDTP on the headset's signaling channel.
    fn hs_signal(&mut self, p: &[u8]) {
        self.hs.raw.push(p.to_vec());
        if self.hs.quiet { return; }
        let (label, k, sig) = (p[0] >> 4, p[0] & 3, p[1] & 0x3F);
        assert_eq!(p[0] >> 2 & 3, 0, "single packets");
        let params = &p[2..];
        if k != kind::COMMAND {
            // Replies to the headset's own commands, as initiator.
            self.hs.replies.push((sig, k == kind::ACCEPT));
            match sig {
                signal::DISCOVER => {
                    assert_eq!(params, &[1 << 2, 0x00], "one audio source end point, free");
                    self.hs_command(signal::GET_CAPABILITIES, &[1 << 2]);
                }
                signal::GET_CAPABILITIES => {
                    let caps = avdtp::parse_capabilities(params).unwrap();
                    assert!(caps.contains(&(category::MEDIA_TRANSPORT, &[][..])));
                    let codec = caps.iter().find(|c| c.0 == category::MEDIA_CODEC).unwrap().1;
                    assert_eq!(codec[..2], [0x00, 0x00], "audio, SBC");
                    assert_eq!(codec[2] & 0xF0, 0x10, "48 kHz only");
                    // Configure the source: 48 kHz, joint stereo, 16 blocks, 8 subbands, loudness, 2..53.
                    self.hs.config = Some([0x11, 0x15, 2, 53]);
                    self.hs_command(signal::SET_CONFIGURATION, &[1 << 2, 5 << 2, 1, 0, 7, 6, 0x00, 0x00, 0x11, 0x15, 2, 53]);
                }
                signal::SET_CONFIGURATION => { assert_eq!(k, kind::ACCEPT); self.hs_command(signal::OPEN, &[1 << 2]); }
                signal::OPEN => {
                    assert_eq!(k, kind::ACCEPT);
                    let p = self.hs.chans.connect(avdtp::PSM);
                    self.send(l2cap::CID_SIGNALING, &p);
                }
                signal::START => { assert_eq!(k, kind::ACCEPT); self.hs.streaming = true; }
                _ => {}
            }
            return;
        }
        self.hs.commands.push(sig);
        match sig {
            signal::DISCOVER => {
                // A video sink, a busy audio sink, a free audio sink.
                self.avdtp(label, kind::ACCEPT, sig, &[2 << 2, 0x18, 3 << 2 | 2, 0x08, 5 << 2, 0x08]);
            }
            signal::GET_CAPABILITIES | signal::GET_ALL_CAPABILITIES => {
                assert_eq!(params, &[5 << 2], "asks the free audio sink");
                self.avdtp(label, kind::ACCEPT, sig, &[1, 0, 7, 6, 0x00, 0x00, 0xFF, 0xFF, 2, 53, 8, 0]);
            }
            signal::SET_CONFIGURATION => {
                assert_eq!(params[0] >> 2, 5);
                let int = params[1] >> 2;
                assert!((1..=0x3E).contains(&int));
                let caps = avdtp::parse_capabilities(&params[2..]).unwrap();
                assert_eq!(caps.len(), 2, "transport and codec, no delay reporting: {caps:?}");
                let codec = caps.iter().find(|c| c.0 == category::MEDIA_CODEC).unwrap().1;
                self.hs.config = Some(codec[2..6].try_into().unwrap());
                self.avdtp(label, kind::ACCEPT, sig, &[]);
            }
            signal::OPEN | signal::CLOSE => { if sig == signal::CLOSE { self.hs.config = None; } self.avdtp(label, kind::ACCEPT, sig, &[]) }
            signal::START => { assert_eq!(params, &[5 << 2]); self.hs.streaming = true; self.avdtp(label, kind::ACCEPT, sig, &[]); }
            signal::SUSPEND => { self.hs.streaming = false; self.avdtp(label, kind::ACCEPT, sig, &[]); }
            _ => self.avdtp(label, kind::GENERAL_REJECT, sig, &[]),
        }
    }

    /// One media packet: RTP, then SBC frames for the headset's decoder.
    fn hs_media(&mut self, p: &[u8]) {
        assert!(self.hs.streaming, "media only while streaming");
        let (seq, ts, n, mut data) = a2dp::parse(p).expect("an RTP packet");
        assert_eq!(p[1] & 0x7F, 96);
        self.hs.seqs.push(seq);
        self.hs.stamps.push(ts);
        for _ in 0..n {
            assert!(sbc::check_crc(data), "frame CRC");
            let (info, pcm) = self.hs.decoder.decode(data).expect("decodes");
            assert_eq!((info.rate, info.channels, info.blocks, info.subbands, info.joint), (48_000, 2, 16, 8, true));
            self.hs.left.extend(&pcm[0]);
            self.hs.right.extend(&pcm[1]);
            self.hs.frames += 1;
            data = &data[info.bytes..];
        }
        assert!(data.is_empty(), "frame count matches the payload");
    }

    fn take(&mut self, p: &[u8]) {
        if let Some((h, pb, data)) = hci::parse_acl(p) {
            assert_eq!(h, self.handle);
            self.outstanding += 1;
            self.max_outstanding = self.max_outstanding.max(self.outstanding);
            assert!(self.outstanding <= ACL_BUFFERS, "the host overran the controller's ACL buffers");
            if self.hold { self.held += 1; } else { self.outstanding -= 1; self.completed(1); }
            let Some(f) = self.rx.push(pb, data) else { return };
            let (cid, payload) = l2cap::parse_frame(&f).unwrap();
            if cid == l2cap::CID_SIGNALING {
                for (c, id, d) in l2cap::commands(payload) {
                    let allow = |psm: u16| if psm == avdtp::PSM || psm == sdp::PSM { Ok(()) } else { Err(l2cap::PSM_NOT_SUPPORTED) };
                    let e = self.hs.chans.on_command(c, id, d, &allow).unwrap_or_default();
                    self.chan_events(e);
                }
                return;
            }
            let ch = *self.hs.chans.by_local(cid).expect("a headset channel");
            match ch.psm {
                sdp::PSM => self.sdp(ch.remote, payload),
                avdtp::PSM if Some(cid) == self.hs.signal => self.hs_signal(payload),
                avdtp::PSM => self.hs_media(payload),
                psm => panic!("psm {psm:#x}"),
            }
            return;
        }
        let (opcode, params) = hci::parse_command(p).expect("a command");
        self.opcodes.push(opcode);
        let params = params.to_vec();
        let h = self.handle;
        let complete = |c: &mut Controller| {
            let mut r = vec![0, h as u8, (h >> 8) as u8];
            r.extend_from_slice(&HEADSET.0);
            r.extend_from_slice(&[0x01, 0x00]);
            c.ev(hci::ev::CONNECTION_COMPLETE, &r);
        };
        match opcode {
            hop::READ_LOCAL_VERSION => self.cc(opcode, &[0, 0x0c, 0x00, 0x01, 0x0c, 0x02, 0x00, 0x00, 0x01]),
            hop::READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0xfe, 0x8f, 0xfe, 0xd8, 0x3f, 0x5b, 0x87]),
            // 1021-byte ACL packets, four buffers.
            hop::READ_BUFFER_SIZE => self.cc(opcode, &[0, 0xfd, 0x03, 0x40, ACL_BUFFERS as u8, 0x00, 0x00, 0x00]),
            hop::READ_BD_ADDR => { let mut r = vec![0]; r.extend_from_slice(&HOST_ADDR); self.cc(opcode, &r) }
            hop::LE_READ_LOCAL_FEATURES => self.cc(opcode, &[0, 0xff, 0x59, 0, 0, 0, 0, 0, 0]),
            // No LE buffers of its own: one shared pool.
            hop::LE_READ_BUFFER_SIZE => self.cc(opcode, &[0, 0x00, 0x00, 0x00]),
            hop::CREATE_CONNECTION => { assert_eq!(&params[..6], &HEADSET.0); self.status(opcode); complete(self); }
            code::ACCEPT_CONNECTION_REQUEST => { assert_eq!(&params[..6], &HEADSET.0); self.status(opcode); complete(self); }
            hop::AUTHENTICATION_REQUESTED => { self.status(opcode); self.ev(hci::ev::LINK_KEY_REQUEST, &HEADSET.0); }
            hop::LINK_KEY_REQUEST_NEGATIVE_REPLY => { self.cc(opcode, &[0]); self.ev(hci::ev::IO_CAPABILITY_REQUEST, &HEADSET.0); }
            hop::LINK_KEY_REQUEST_REPLY => {
                self.cc(opcode, &[0]);
                assert_eq!(Some(params[6..22].try_into().unwrap()), self.hs.link_key, "the stored link key");
                self.ev(code::EV_AUTHENTICATION_COMPLETE, &[0, h as u8, (h >> 8) as u8]);
            }
            hop::IO_CAPABILITY_REQUEST_REPLY => {
                self.cc(opcode, &[0]);
                let mut r = HEADSET.0.to_vec();
                r.extend_from_slice(&[0x03, 0x00, 0x04]); // NoInputNoOutput: Just Works
                self.ev(hci::ev::IO_CAPABILITY_RESPONSE, &r);
                let mut u = HEADSET.0.to_vec();
                u.extend_from_slice(&424242u32.to_le_bytes());
                self.ev(hci::ev::USER_CONFIRMATION_REQUEST, &u);
            }
            hop::USER_CONFIRMATION_REQUEST_REPLY => {
                self.cc(opcode, &[0]);
                let mut s = vec![0];
                s.extend_from_slice(&HEADSET.0);
                self.ev(hci::ev::SIMPLE_PAIRING_COMPLETE, &s);
                let key = [0xa2; 16];
                self.hs.link_key = Some(key);
                let mut n = HEADSET.0.to_vec();
                n.extend_from_slice(&key);
                n.push(0x04);
                self.ev(hci::ev::LINK_KEY_NOTIFICATION, &n);
                self.ev(code::EV_AUTHENTICATION_COMPLETE, &[0, h as u8, (h >> 8) as u8]);
            }
            code::SET_CONNECTION_ENCRYPTION => {
                self.status(opcode);
                self.ev(hci::ev::ENCRYPTION_CHANGE, &[0, h as u8, (h >> 8) as u8, 1]);
                if self.hs.initiator {
                    // A headset that paged us opens AVDTP once the link is encrypted.
                    self.hs.chans = Channels::default();
                    self.hs.signal = None;
                    self.hs.media = None;
                    let p = self.hs.chans.connect(avdtp::PSM);
                    self.send(l2cap::CID_SIGNALING, &p);
                }
            }
            _ => self.cc(opcode, &[0]),
        }
    }
}

fn host() -> Host {
    let n = Rc::new(Cell::new(1u8));
    Host::new(Box::new(move |b: &mut [u8]| { for x in b.iter_mut() { *x = n.get(); n.set(n.get().wrapping_mul(31).wrapping_add(7)); } }))
}

fn pump(h: &mut Host, c: &mut Controller) -> Vec<Note> {
    let mut notes = vec![];
    for _ in 0..100_000 {
        let mut moved = false;
        while let Some(p) = h.poll() { c.take(&p); moved = true; }
        while let Some(p) = c.out.pop_front() { h.on_packet(&p); moved = true; }
        while let Some(n) = h.note() {
            if let Note::Compare { handle, .. } = n { h.confirm(handle, true); }
            notes.push(n);
            moved = true;
        }
        if !moved { return notes; }
    }
    panic!("did not settle");
}

fn ready() -> (Host, Controller) {
    let mut h = host();
    let mut c = Controller::new();
    let n = pump(&mut h, &mut c);
    assert!(matches!(n.first(), Some(Note::Ready(i)) if i.le_shared), "{n:?}");
    (h, c)
}

fn headset() -> Found {
    // Class of device 0x240404: Audio service, Audio/Video major class, headphones.
    Found { addr: HEADSET, transport: Transport::Classic, addr_type: 0, rssi: Some(-45), class: Some(0x240404),
        name: Some("Cuff Headphones".into()), appearance: None, flags: None, connectable: true }
}

/// A 1 kHz tone left, 1.5 kHz right.
fn tone(n: usize) -> Vec<[i16; 2]> {
    (0..n).map(|i| {
        let t = i as f64 / 48_000.0;
        [((2.0 * std::f64::consts::PI * 1000.0 * t).sin() * 9000.0) as i16, ((2.0 * std::f64::consts::PI * 1500.0 * t).sin() * 6000.0) as i16]
    }).collect()
}

/// Play all of `pcm` as fast as the host takes it.
fn play_all(h: &mut Host, c: &mut Controller, handle: u16, pcm: &[[i16; 2]]) {
    let mut at = 0;
    for _ in 0..10_000 {
        if pcm.len() - at < 128 { return; }
        at += h.play(handle, &pcm[at..]);
        pump(h, c);
    }
    panic!("the host stopped taking audio");
}

fn snr(want: &[f64], got: &[f64]) -> f64 {
    let mut best = f64::MIN;
    for delay in 0..300 {
        let (mut sig, mut err) = (0.0, 0.0);
        for i in 2000..want.len().min(got.len().saturating_sub(delay)) {
            sig += want[i] * want[i];
            let e = want[i] - got[i + delay];
            err += e * e;
        }
        if sig > 0.0 { best = best.max(10.0 * (sig / err.max(1e-9)).log10()); }
    }
    best
}

/// Connect, pair and start streaming to the headset; the stream's handle.
fn streaming() -> (Host, Controller, u16) {
    let (mut h, mut c) = ready();
    assert!(Host::is_audio(&headset()));
    assert!(!Host::is_hid(&headset()));
    h.connect_audio(&headset());
    let notes = pump(&mut h, &mut c);
    let handle = c.handle;
    assert!(notes.contains(&Note::Paired { addr: HEADSET, le: false }), "{notes:?}");
    assert!(notes.contains(&Note::Encrypted { handle }), "{notes:?}");
    assert!(notes.contains(&Note::AudioSink { handle, sink: sdp::A2dpSink { psm: 0x0019, avdtp_version: 0x0103, a2dp_version: 0x0103, features: 1 } }), "{notes:?}");
    let config = sbc::Config { rate: 48_000, blocks: 16, mode: Mode::JointStereo, allocation: Allocation::Loudness, subbands: 8, bitpool: 53 };
    // Default L2CAP MTU 672: five 119-byte frames after the 13-byte headers.
    assert!(notes.contains(&Note::AudioReady { handle, addr: HEADSET, config, frames_per_packet: 5 }), "{notes:?}");
    assert!(notes.contains(&Note::AudioStreaming { handle }), "{notes:?}");
    assert_eq!(c.hs.sdp_requests, 2, "the record came in two parts");
    assert_eq!(c.hs.commands, [signal::DISCOVER, signal::GET_CAPABILITIES, signal::SET_CONFIGURATION, signal::OPEN, signal::START]);
    assert_eq!(c.hs.config, Some([0x11, 0x15, 2, 53]));
    assert!(c.hs.streaming);
    // No HID channels on a headset; SDP was closed after the search.
    assert!(c.hs.chans.by_psm(0x0011).is_none() && c.hs.chans.by_psm(sdp::PSM).is_none());
    assert_eq!(h.streaming().collect::<Vec<_>>(), [handle]);
    (h, c, handle)
}

#[test]
fn headset_pairs_streams_and_hears_the_tone() {
    let (mut h, mut c, handle) = streaming();
    let pcm = tone(48_000 / 2);
    play_all(&mut h, &mut c, handle, &pcm);
    assert!(c.hs.frames >= 180, "{} frames", c.hs.frames);
    let l: Vec<f64> = pcm.iter().map(|p| p[0] as f64).collect();
    let r: Vec<f64> = pcm.iter().map(|p| p[1] as f64).collect();
    let (sl, sr) = (snr(&l, &c.hs.left), snr(&r, &c.hs.right));
    assert!(sl > 30.0 && sr > 30.0, "headset hears the tone: {sl:.1} / {sr:.1} dB");
    // RTP: sequence numbers count up by one, timestamps by the samples sent.
    for w in c.hs.seqs.windows(2) { assert_eq!(w[1], w[0].wrapping_add(1)); }
    for w in c.hs.stamps.windows(2) { assert_eq!(w[1] - w[0], 5 * 128); }
    assert!(c.max_outstanding <= ACL_BUFFERS);
}

#[test]
fn acl_credits_bound_what_reaches_the_controller() {
    let (mut h, mut c, handle) = streaming();
    c.hold = true;
    let pcm = tone(96_000);
    let mut at = 0;
    for _ in 0..20 { at += h.play(handle, &pcm[at..]); pump(&mut h, &mut c); }
    // Four buffers in the controller; the host keeps at most the media window.
    assert_eq!(c.max_outstanding, ACL_BUFFERS);
    assert!(at <= sex_bt::host::MEDIA_WINDOW * 5 * 128, "took {at} pairs with the controller full");
    assert!(at > 0);
    let before = at;
    // The controller frees its buffers: more goes out, still never more than four.
    for _ in 0..30 { c.release(); pump(&mut h, &mut c); at += h.play(handle, &pcm[at..]); pump(&mut h, &mut c); }
    assert!(at > before + 10 * 128, "audio flows again ({before} -> {at})");
    assert_eq!(c.max_outstanding, ACL_BUFFERS);
    // A controller that reports more completions than were sent (or for a link that
    // does not exist) gives no extra credit.
    c.release();
    pump(&mut h, &mut c);
    c.release();
    pump(&mut h, &mut c);
    assert_eq!(c.outstanding, 0);
    c.completed(50);
    c.ev(hci::ev::NUMBER_OF_COMPLETED_PACKETS, &[2, 0x77, 0x07, 9, 0, 0x0b, 0x00, 9, 0]);
    c.ev(hci::ev::NUMBER_OF_COMPLETED_PACKETS, &[200, 0x0b, 0x00]);
    pump(&mut h, &mut c);
    for _ in 0..3 { at += h.play(handle, &pcm[at..]); }
    pump(&mut h, &mut c);
    assert!(at < pcm.len());
    assert!(c.max_outstanding <= ACL_BUFFERS);
}

#[test]
fn suspend_resume_close_and_encryption_off() {
    let (mut h, mut c, handle) = streaming();
    h.suspend_audio(handle);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::AudioSuspended { handle }), "{notes:?}");
    assert!(!c.hs.streaming);
    assert_eq!(h.play(handle, &tone(1280)), 0, "nothing while suspended");
    h.resume_audio(handle);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::AudioStreaming { handle }), "{notes:?}");
    assert!(h.play(handle, &tone(1280)) > 0);
    pump(&mut h, &mut c);
    // Encryption off: no audio until it is back.
    c.ev(hci::ev::ENCRYPTION_CHANGE, &[0, handle as u8, 0, 0]);
    pump(&mut h, &mut c);
    assert_eq!(h.play(handle, &tone(1280)), 0, "no audio on an unencrypted link");
    c.ev(hci::ev::ENCRYPTION_CHANGE, &[0, handle as u8, 0, 1]);
    pump(&mut h, &mut c);
    assert!(h.play(handle, &tone(1280)) > 0);
    pump(&mut h, &mut c);
    h.close_audio(handle);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::AudioClosed { handle }), "{notes:?}");
    assert_eq!(c.hs.commands.last(), Some(&signal::CLOSE));
    assert!(c.hs.media.is_none(), "transport channel closed");
    assert_eq!(h.play(handle, &tone(1280)), 0);
}

#[test]
fn bonded_headset_pages_us_and_sets_the_stream_up() {
    let (mut h, mut c, _) = streaming();
    c.ev(hci::ev::DISCONNECTION_COMPLETE, &[0, 0x0b, 0x00, 0x13]);
    let notes = pump(&mut h, &mut c);
    assert!(notes.contains(&Note::AudioClosed { handle: 0x000b }), "{notes:?}");
    // Later the headset pages us; it will set the stream up itself.
    c.handle = 0x000c;
    c.hs.initiator = true;
    c.hs.commands.clear();
    let mut req = HEADSET.0.to_vec();
    req.extend_from_slice(&[0x04, 0x04, 0x24, 0x01]);
    c.ev(code::EV_CONNECTION_REQUEST, &req);
    let notes = pump(&mut h, &mut c);
    assert!(c.opcodes.contains(&code::ACCEPT_CONNECTION_REQUEST));
    assert!(!notes.iter().any(|n| matches!(n, Note::Paired { .. })), "stored key: {notes:?}");
    assert!(notes.iter().any(|n| matches!(n, Note::AudioReady { handle: 0x000c, .. })), "{notes:?}");
    assert!(notes.contains(&Note::AudioStreaming { handle: 0x000c }), "{notes:?}");
    assert!(c.hs.replies.iter().all(|r| r.1), "every headset command accepted: {:?}", c.hs.replies);
    assert!(c.hs.commands.is_empty(), "we sent no commands of our own: {:?}", c.hs.commands);
    let frames = c.hs.frames;
    play_all(&mut h, &mut c, 0x000c, &tone(12_800));
    assert!(c.hs.frames >= frames + 90);
}

#[test]
fn stranger_cannot_open_avdtp() {
    let (mut h, mut c) = ready();
    // An unbonded device pages us: refused at the link level.
    let mut req = HEADSET.0.to_vec();
    req.extend_from_slice(&[0x04, 0x04, 0x24, 0x01]);
    c.ev(code::EV_CONNECTION_REQUEST, &req);
    pump(&mut h, &mut c);
    assert!(c.opcodes.contains(&code::REJECT_CONNECTION_REQUEST));
    // A link we made but that is not encrypted yet: AVDTP is refused (security block).
    let mut ch = Channels::default();
    let r = ch.on_command(l2cap::sig::CONNECTION_REQUEST, 1, &[0x19, 0x00, 0x40, 0x00], &|_| Err(l2cap::SECURITY_BLOCK)).unwrap();
    assert!(matches!(&r[0], ChanEvent::Send(p) if p[0] == l2cap::sig::CONNECTION_RESPONSE && p[8] == 3));
}

#[test]
fn malformed_avdtp_gets_rejects_not_panics() {
    let (mut h, mut c, handle) = streaming();
    c.hs.raw.clear();
    c.hs.quiet = true;
    // Too short, wrong SEID, wrong state, unknown signal, bad capability lengths.
    c.avdtp(1, kind::COMMAND, signal::SET_CONFIGURATION, &[]);
    c.avdtp(2, kind::COMMAND, signal::GET_CAPABILITIES, &[9 << 2]);
    c.avdtp(3, kind::COMMAND, signal::OPEN, &[1 << 2]);
    c.avdtp(4, kind::COMMAND, 0x2F, &[1, 2, 3]);
    c.avdtp(5, kind::COMMAND, signal::SET_CONFIGURATION, &[1 << 2, 5 << 2, 7, 200, 0]);
    c.avdtp(6, kind::COMMAND, signal::START, &[]);
    c.avdtp(7, kind::COMMAND, signal::START, &[4 << 2]);
    c.avdtp(8, kind::ACCEPT, signal::START, &[]); // a reply to nothing
    // A fragmented message whose packet count lies, then a continuation out of nowhere.
    let r = c.signal_remote();
    c.send(r, &[9 << 4 | 1 << 2, 5, signal::DELAY_REPORT, 1, 2]);
    c.send(r, &[9 << 4 | 3 << 2, 3, 4]);
    c.send(r, &[10 << 4 | 2 << 2, 1]);
    c.send(r, &[]);
    pump(&mut h, &mut c);
    let rej: Vec<(u8, u8, Vec<u8>)> = c.hs.raw.iter().map(|p| (p[0] >> 4, p[0] & 3, p[2..].to_vec())).collect();
    assert_eq!(rej, [
        (1, kind::REJECT, vec![0, avdtp::error::BAD_LENGTH]),
        (2, kind::REJECT, vec![avdtp::error::BAD_ACP_SEID]),
        (3, kind::REJECT, vec![avdtp::error::BAD_STATE]),
        (4, kind::GENERAL_REJECT, vec![]),
        (5, kind::REJECT, vec![0, avdtp::error::SEP_IN_USE]),
        (6, kind::REJECT, vec![0, avdtp::error::BAD_LENGTH]),
        (7, kind::REJECT, vec![4 << 2, avdtp::error::BAD_ACP_SEID]),
    ]);
    // Still streaming.
    c.hs.quiet = false;
    assert_eq!(h.audio_state(handle), Some(avdtp::State::Streaming));
    assert!(h.play(handle, &tone(640)) > 0);
}

#[test]
fn acceptor_checks_the_configuration() {
    let mut s = avdtp::Source::new(672);
    let reply = |out: Vec<avdtp::Out>| out.into_iter().find_map(|o| match o { avdtp::Out::Send(p) => Some(p), _ => None }).unwrap();
    // 44.1 kHz: we only stream 48 kHz.
    let p = reply(s.on_packet(&[0x10, signal::SET_CONFIGURATION, 1 << 2, 2 << 2, 1, 0, 7, 6, 0, 0, 0x21, 0x15, 2, 53]));
    assert_eq!(p, [0x13, signal::SET_CONFIGURATION, category::MEDIA_CODEC, avdtp::error::UNSUPPORTED_CONFIGURATION]);
    // Two channel modes at once is not a configuration.
    let p = reply(s.on_packet(&[0x20, signal::SET_CONFIGURATION, 1 << 2, 2 << 2, 1, 0, 7, 6, 0, 0, 0x13, 0x15, 2, 53]));
    assert_eq!(p[3], avdtp::error::UNSUPPORTED_CONFIGURATION);
    // Content protection is not offered.
    let p = reply(s.on_packet(&[0x30, signal::SET_CONFIGURATION, 1 << 2, 2 << 2, 1, 0, 4, 2, 2, 0, 7, 6, 0, 0, 0x11, 0x15, 2, 53]));
    assert_eq!(&p[2..], &[category::CONTENT_PROTECTION, avdtp::error::BAD_SERV_CATEGORY]);
    // A bitpool range above ours: we use 53, the top of ours.
    let out = s.on_packet(&[0x40, signal::SET_CONFIGURATION, 1 << 2, 2 << 2, 1, 0, 7, 6, 0, 0, 0x11, 0x15, 2, 250]);
    assert!(out.iter().any(|o| matches!(o, avdtp::Out::Configured(c) if c.bitpool == 53)), "{out:?}");
    // A range entirely above ours is refused.
    let mut t = avdtp::Source::new(672);
    let p = reply(t.on_packet(&[0x50, signal::SET_CONFIGURATION, 1 << 2, 2 << 2, 1, 0, 7, 6, 0, 0, 0x11, 0x15, 60, 250]));
    assert_eq!(p[3], avdtp::error::UNSUPPORTED_CONFIGURATION);
    // Capability choice: a sink offering only 44.1 kHz cannot be streamed to.
    assert_eq!(avdtp::SbcInfo([0x2F, 0xFF, 2, 53]).choose(), None);
    let (cfg, info) = avdtp::SbcInfo([0x1A, 0x6A, 10, 35]).choose().unwrap();
    assert_eq!((cfg.mode, cfg.blocks, cfg.subbands, cfg.allocation, cfg.bitpool), (Mode::Stereo, 12, 4, Allocation::Snr, 35));
    assert_eq!(info.0, [0x12, 0x2A, 10, 35]);
}

#[test]
fn avdtp_fragments_round_trip() {
    let params: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
    let packets = avdtp::packets(7, kind::ACCEPT, signal::GET_ALL_CAPABILITIES, &params, 48);
    assert!(packets.len() > 1 && packets.iter().all(|p| p.len() <= 48));
    let mut r = avdtp::Reassembly::default();
    let mut got = None;
    for p in &packets { if let Some(m) = r.push(p) { got = Some(m); } }
    assert_eq!(got, Some(avdtp::Message { label: 7, kind: kind::ACCEPT, signal: signal::GET_ALL_CAPABILITIES, params }));
}

#[test]
fn l2cap_mtu_option_bounds_media_packets() {
    let mut ch = Channels::default();
    let p = ch.connect(avdtp::PSM);
    let local = u16::from_le_bytes([p[6], p[7]]);
    // The peer's configuration request with an MTU option of 895.
    let e = ch.on_command(l2cap::sig2::CONFIGURATION_REQUEST, 3, &[local as u8, (local >> 8) as u8, 0, 0, 0x01, 2, 0x7F, 0x03], &|_| Ok(())).unwrap();
    assert!(matches!(&e[0], ChanEvent::Send(_)));
    assert_eq!(ch.by_local(local).unwrap().mtu, 895);
    let cfg = sbc::Config::HIGH_QUALITY_48K;
    assert_eq!(a2dp::Packetizer::new(cfg, 895).unwrap().frames_per_packet(), 7);
    assert_eq!(a2dp::Packetizer::new(cfg, 672).unwrap().frames_per_packet(), 5);
    assert!(a2dp::Packetizer::new(cfg, 100).is_none());
    // A silly MTU is raised to the minimum.
    let e = ch.on_command(l2cap::sig2::CONFIGURATION_REQUEST, 4, &[local as u8, (local >> 8) as u8, 0, 0, 0x01, 2, 0x01, 0x00], &|_| Ok(())).unwrap();
    assert!(!e.is_empty());
    assert_eq!(ch.by_local(local).unwrap().mtu, l2cap::MIN_MTU);
}

#[test]
fn sdp_elements_and_records() {
    // Our encoder and parser agree with the hand-written sink record.
    let (e, n) = sdp::Element::decode(SINK_RECORD).unwrap();
    assert_eq!(n, SINK_RECORD.len());
    assert_eq!(e.to_bytes(), SINK_RECORD);
    // Our own source record survives a round trip and is found by its class.
    let ours = sdp::a2dp_source_record();
    let req = {
        let mut p = vec![sdp::pdu::SERVICE_SEARCH_ATTRIBUTE_REQ, 0, 9];
        let params = [0x35, 0x03, 0x19, 0x11, 0x0A, 0x00, 0x10, 0x35, 0x05, 0x0A, 0x00, 0x00, 0xFF, 0xFF, 0x00];
        p.extend_from_slice(&(params.len() as u16).to_be_bytes());
        p.extend_from_slice(&params);
        p
    };
    // 16 bytes at a time: our server continues; our client collects.
    let mut client = sdp::Client::new(sdp::uuid::AUDIO_SOURCE, &[0x0000, 0x0001, 0x0004, 0x0005, 0x0009, 0x0311], 16);
    let _ = client.start();
    let mut rsp = sdp::serve(&req, &[ours.clone()]);
    let mut rounds = 0;
    let records = loop {
        rsp[1..3].copy_from_slice(&(rounds as u16 + 1).to_be_bytes());
        match client.on_pdu(&rsp) {
            sdp::Progress::Send(next) => { rounds += 1; rsp = sdp::serve(&next, &[ours.clone()]); }
            sdp::Progress::Done(r) => break r,
            sdp::Progress::Failed(e) => panic!("failed {e:#x}"),
        }
    };
    assert!(rounds > 3);
    assert_eq!(records, [ours.clone()]);
    assert_eq!(records[0].protocol(sdp::uuid::AVDTP), Some((0x0019, 0x0103)));
    // A search for something we do not have finds nothing.
    let mut none = req.clone();
    none[9] = 0x0B;
    assert_eq!(&sdp::serve(&none, &[ours])[5..], &[0x00, 0x02, 0x35, 0x00, 0x00]);
}

#[test]
fn malformed_sdp_fails_cleanly() {
    // Nesting deeper than the bound, lengths past the end, odd attribute lists.
    let mut deep = vec![];
    for _ in 0..40 { deep.extend_from_slice(&[0x35, 0x00]); }
    for i in 0..40 { deep[2 * i + 1] = (2 * (40 - i - 1)) as u8; }
    assert!(sdp::Element::decode(&deep).is_none());
    assert!(sdp::Element::decode(&[0x35, 0x05, 0x19, 0x11]).is_none());
    assert!(sdp::Element::decode(&[0x36, 0xFF]).is_none());
    assert!(sdp::Element::decode(&[0x1C]).is_none(), "16-byte integers refused");
    let rsp = |tid: u16, lists: &[u8], cont: &[u8]| {
        let mut params = (lists.len() as u16).to_be_bytes().to_vec();
        params.extend_from_slice(lists);
        params.extend_from_slice(cont);
        let mut r = vec![sdp::pdu::SERVICE_SEARCH_ATTRIBUTE_RSP, (tid >> 8) as u8, tid as u8];
        r.extend_from_slice(&(params.len() as u16).to_be_bytes());
        r.extend_from_slice(&params);
        r
    };
    let mut c = sdp::Client::a2dp_sink();
    c.start();
    assert!(matches!(c.on_pdu(&rsp(1, &[0x35, 0x03, 0x09, 0x00, 0x01], &[0])), sdp::Progress::Failed(_)), "odd attribute list");
    c.start();
    assert!(matches!(c.on_pdu(&rsp(2, &[0x35], &[17, 0])), sdp::Progress::Failed(sdp::error::INVALID_CONTINUATION)));
    c.start();
    assert!(matches!(c.on_pdu(&rsp(9, &SINK_RECORD, &[0])), sdp::Progress::Failed(_)), "wrong transaction");
    c.start();
    assert!(matches!(c.on_pdu(&[1, 0, 4, 0, 2, 0, 3]), sdp::Progress::Failed(3)), "error response");
    // A peer that continues forever is cut off.
    c.start();
    let mut rounds = 0;
    let mut tid = 5u16;
    loop {
        match c.on_pdu(&rsp(tid, &[0x35], &[1, 7])) {
            sdp::Progress::Send(_) => { rounds += 1; tid += 1; }
            sdp::Progress::Failed(_) => break,
            other => panic!("{other:?}"),
        }
        assert!(rounds < 100);
    }
    // No sink record: none found.
    let ok = sdp::Element::decode(SINK_RECORD).unwrap().0;
    let mut records = vec![];
    for r in ok.items() {
        let mut rec = sdp::Record::default();
        for p in r.items().chunks(2) { rec.0.push((p[0].as_uint().unwrap() as u16, p[1].clone())); }
        records.push(rec);
    }
    assert!(sdp::a2dp_sink(&records).is_some());
    records[0].0.retain(|(id, _)| *id != sdp::attr::PROTOCOL_DESCRIPTOR_LIST);
    assert!(sdp::a2dp_sink(&records).is_none());
    // Server: garbage requests get error responses.
    for req in [&[][..], &[6, 0, 1, 0, 0], &[6, 0, 1, 0, 3, 0x35, 0x00, 0x00], &[6, 0, 1, 0, 9, 0x35, 0x03, 0x19, 0x11, 0x0A, 0x00, 0x10, 0x35, 0x00],
        &[0x42, 0, 1, 0, 0]] {
        assert_eq!(sdp::serve(req, &[sdp::a2dp_source_record()])[0], sdp::pdu::ERROR_RSP, "{req:?}");
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut seed = 0x9e37_79b9u32;
    let mut next = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed };
    let mut src = avdtp::Source::new(672);
    let mut client = sdp::Client::a2dp_sink();
    client.start();
    for _ in 0..20_000 {
        let len = (next() % 64) as usize;
        let mut b: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        // Bias towards plausible headers.
        if len > 2 && next() % 2 == 0 { b[0] &= 0xF3; b[1] = (next() % 14) as u8; }
        let _ = src.on_packet(&b);
        let _ = sdp::Element::decode(&b);
        let _ = sdp::serve(&b, &[sdp::a2dp_source_record()]);
        let _ = client.on_pdu(&b);
        let _ = a2dp::parse(&b);
        let _ = avdtp::parse_capabilities(&b);
        let _ = sbc::parse_header(&b);
        let _ = sbc::check_crc(&b);
        if next() % 64 == 0 { src = avdtp::Source::new(672); client.start(); }
    }
}

#[test]
fn media_waiting_for_buffers_is_dropped_when_encryption_goes_off() {
    let (mut h, mut c, handle) = streaming();
    c.hold = true;
    let pcm = tone(96_000);
    let mut at = 0;
    for _ in 0..10 { at += h.play(handle, &pcm[at..]); pump(&mut h, &mut c); }
    assert!(at > 0 && c.outstanding == ACL_BUFFERS, "controller full, media queued in the host");
    let frames = c.hs.frames;
    c.ev(hci::ev::ENCRYPTION_CHANGE, &[0, handle as u8, 0, 0]);
    pump(&mut h, &mut c);
    // Buffers come back: what was queued must not go out on the unencrypted link.
    c.hold = false;
    for _ in 0..5 { c.release(); pump(&mut h, &mut c); }
    assert_eq!(c.hs.frames, frames, "no media after encryption went off");
}
