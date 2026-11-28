// SPDX-License-Identifier: GPL-3.0-only
//! All of one browser's peer connections, behind sex-js `op::RTC`.
//!
//! The page calls in with text (`call`) and gets text back at once (JSON, or `!Name\t
//! message` for an error); what happens later reaches it as `rtc\tID\t...` dispatches
//! from `events`. The browser owns the sockets and the devices: it binds a socket for
//! every peer in `wants_transport`, moves datagrams with `receive` / `poll_transmit`,
//! gives camera pictures and microphone sound to `send_picture` / `send_sound`, and
//! decodes and shows what `poll_frame` hands out.
use crate::peer::{Config, Encoding, Error, Event, Peer, Received, SdpType, Server};
use crate::{g711, vp8, Codec, Kind};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;
use core::net::SocketAddr;
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use sex_sdp::Direction;

pub struct Entry { pub id: u32, pub owner: u64, pub peer: Peer }

pub struct Rtc {
    pub peers: Vec<Entry>,
    next_id: u32,
    rng: ChaCha20Rng,
    /// Milliseconds, set by the browser before each call.
    pub now: u64,
    /// Dispatches for pages: (page owner, text).
    pub events: Vec<(u64, String)>,
    /// Most peers one page may hold.
    pub max_per_page: usize,
}

/// Text safe inside a JSON string.
pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => { let _ = write!(o, "\\u{:04x}", c as u32); }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn fail(e: Error) -> String { format!("!{}\t{}", e.name, e.message) }
fn kind_of(s: &str) -> Option<Kind> { match s { "audio" => Some(Kind::Audio), "video" => Some(Kind::Video), _ => None } }
fn dir_str(d: Option<Direction>) -> String { d.map_or(String::from("null"), |d| json_str(d.as_str())) }

/// `rid,active,maxBitrate,scaleResolutionDownBy,maxFramerate` entries, `;` between.
pub fn parse_encodings(s: &str) -> Vec<Encoding> {
    s.split(';').filter(|e| !e.is_empty()).map(|e| {
        let f: Vec<&str> = e.split(',').collect();
        let g = |i: usize| f.get(i).copied().unwrap_or("");
        Encoding {
            rid: g(0).into(), active: g(1) != "0", max_bitrate: g(2).parse().ok(),
            scale_down: g(3).parse::<f32>().ok().filter(|v| *v >= 1.0).unwrap_or(1.0), max_framerate: g(4).parse().ok(),
        }
    }).collect()
}

impl Rtc {
    pub fn new(seed: [u8; 32]) -> Rtc {
        Rtc { peers: Vec::new(), next_id: 1, rng: ChaCha20Rng::from_seed(seed), now: 0, events: Vec::new(), max_per_page: 32 }
    }

    pub fn peer(&mut self, id: u32) -> Option<&mut Peer> { self.peers.iter_mut().find(|e| e.id == id).map(|e| &mut e.peer) }

    fn state(e: &Entry) -> String {
        let p = &e.peer;
        let mut s = format!("{{\"sig\":{},\"ice\":{},\"gather\":{},\"conn\":{},\"needed\":{},\"t\":[",
            json_str(p.signaling.as_str()), json_str(p.ice_state), json_str(p.gathering), json_str(p.connection), p.negotiation_needed());
        for (i, t) in p.transceivers.iter().enumerate() {
            if i > 0 { s.push(','); }
            let _ = write!(s, "{{\"i\":{i},\"mid\":{},\"kind\":{},\"dir\":{},\"cur\":{},\"stopped\":{},\"rtrack\":{},\"rstreams\":[",
                t.mid.as_deref().map_or(String::from("null"), json_str), json_str(if t.kind == Kind::Audio { "audio" } else { "video" }),
                json_str(t.direction.as_str()), dir_str(t.current_direction), t.stopped, json_str(&t.remote_track));
            for (k, st) in t.remote_streams.iter().enumerate() { if k > 0 { s.push(','); } s.push_str(&json_str(st)); }
            let _ = write!(s, "],\"ssrc\":{},\"codecs\":[", t.ssrc);
            for (k, c) in t.codecs.iter().enumerate() {
                if k > 0 { s.push(','); }
                let _ = write!(s, "{{\"mimeType\":{},\"clockRate\":{},\"payloadType\":{}", json_str(&mime_case(c.codec)), c.codec.clock(), c.pt);
                if let Some(ch) = c.codec.channels() { let _ = write!(s, ",\"channels\":{ch}"); }
                if let Some(f) = &c.fmtp { let _ = write!(s, ",\"sdpFmtpLine\":{}", json_str(f)); }
                s.push('}');
            }
            s.push_str("]}");
        }
        s.push_str("]}");
        s
    }

    fn entry(&mut self, owner: u64, id: &str) -> Option<usize> {
        let id: u32 = id.parse().ok()?;
        self.peers.iter().position(|e| e.id == id && e.owner == owner)
    }

    /// One `op::RTC` call from page `owner`. See sex-js `op::RTC` for the verbs.
    pub fn call(&mut self, owner: u64, args: &[&str]) -> Option<String> {
        let a = |i: usize| args.get(i).copied().unwrap_or("");
        let now = self.now;
        if a(0) == "new" {
            if self.peers.iter().filter(|e| e.owner == owner).count() >= self.max_per_page { return Some(String::from("!UnknownError\tToo many peer connections.")); }
            let mut cfg = Config { relay_only: a(2) == "relay", ..Default::default() };
            for line in a(1).lines().filter(|l| !l.is_empty()) {
                let f: Vec<&str> = line.split('\t').collect();
                let url = f[0].trim();
                match Server::parse(url, f.get(1).copied().unwrap_or(""), f.get(2).copied().unwrap_or("")) {
                    Some(s) => cfg.servers.push(s),
                    None => return Some(format!("!SyntaxError\tFailed to construct 'RTCPeerConnection': '{url}' is not a valid URL.")),
                }
            }
            let mut seed = [0u8; 32];
            self.rng.fill_bytes(&mut seed);
            let id = self.next_id;
            self.next_id += 1;
            self.peers.push(Entry { id, owner, peer: Peer::new(cfg, seed) });
            let fp = self.peers.last().unwrap().peer.certificate.fingerprint().to_sdp();
            return Some(format!("{{\"id\":{id},\"fingerprint\":{}}}", json_str(&fp)));
        }
        let k = self.entry(owner, a(1))?;
        let r: Result<String, Error> = {
            let e = &mut self.peers[k];
            let p = &mut e.peer;
            let ti = || a(2).parse::<usize>().ok();
            match a(0) {
                "offer" => p.create_offer(a(2) == "1").map(|sdp| format!("{{\"sdp\":{}}}", json_str(&sdp))),
                "answer" => p.create_answer().map(|sdp| format!("{{\"sdp\":{}}}", json_str(&sdp))),
                "local" | "remote" => match SdpType::parse(a(2)) {
                    None => Err(Error { name: "TypeError", message: format!("The provided value '{}' is not a valid enum value of type RTCSdpType.", a(2)) }),
                    Some(ty) => {
                        let res = if a(0) == "local" {
                            // Implicit: make the description it should be.
                            let sdp = if a(3).is_empty() && ty != SdpType::Rollback {
                                let made = if ty == SdpType::Offer { p.create_offer(false) } else { p.create_answer() };
                                match made { Ok(s) => s, Err(e) => return Some(fail(e)) }
                            } else { a(3).into() };
                            p.set_local_description(now, ty, &sdp).map(|_| Vec::new())
                        } else { p.set_remote_description(now, ty, a(3)) };
                        res.map(|tracks| {
                            let mut s = String::from("{\"tracks\":[");
                            for (i, t) in tracks.iter().enumerate() { if i > 0 { s.push(','); } let _ = write!(s, "{t}"); }
                            s.push_str("]}");
                            s
                        })
                    }
                },
                "addtrack" | "addtransceiver" => {
                    let Some(kind) = kind_of(a(2)) else { return Some(String::from("!TypeError\tbad kind")) };
                    let dir = crate::peer::parse_direction(a(3)).unwrap_or(Direction::SendRecv);
                    let source = (!a(4).is_empty()).then(|| String::from(a(4)));
                    let streams: Vec<String> = a(5).split(',').filter(|s| !s.is_empty()).map(String::from).collect();
                    let i = p.add_transceiver(kind, dir, source, a(6).into(), streams, parse_encodings(a(7)), a(0) == "addtrack");
                    Ok(format!("{{\"index\":{i}}}"))
                }
                "removetrack" => { if let Some(i) = ti() { p.remove_track(i); } Ok(String::from("{}")) }
                "direction" => {
                    if let (Some(i), Some(d)) = (ti(), crate::peer::parse_direction(a(3))) { if let Some(t) = p.transceivers.get_mut(i) { if !t.stopped { t.direction = d; } } }
                    Ok(String::from("{}"))
                }
                "stop" => { if let Some(i) = ti() { p.stop_transceiver(i); } Ok(String::from("{}")) }
                "prefs" => {
                    if let Some(t) = ti().and_then(|i| p.transceivers.get_mut(i)) { t.preferences = a(3).split(',').filter(|s| !s.is_empty()).map(|s| s.to_ascii_lowercase()).collect(); }
                    Ok(String::from("{}"))
                }
                "replace" => {
                    if let Some(t) = ti().and_then(|i| p.transceivers.get_mut(i)) {
                        t.source = (!a(3).is_empty()).then(|| String::from(a(3)));
                        if !a(4).is_empty() { t.track_id = a(4).into(); }
                        t.want_keyframe = true;
                    }
                    Ok(String::from("{}"))
                }
                "params" => {
                    if let Some(t) = ti().and_then(|i| p.transceivers.get_mut(i)) {
                        let e = parse_encodings(a(3));
                        if !e.is_empty() { t.encodings = e; }
                    }
                    Ok(String::from("{}"))
                }
                "candidate" => p.add_ice_candidate(now, a(2)).map(|_| String::from("{}")),
                "close" => { p.close(now); Ok(String::from("{}")) }
                "restart" => Ok(String::from("{}")),
                "desc" => {
                    let d = |x: Option<(SdpType, String)>| x.map_or(String::from("null"), |(t, s)| format!("{{\"type\":{},\"sdp\":{}}}", json_str(t.as_str()), json_str(&s)));
                    let local = p.local_with_candidates();
                    let pending_local = if p.local != p.current_local { local.clone() } else { None };
                    let current_local = if p.local == p.current_local { local.clone() } else { None };
                    let pending_remote = if p.remote != p.current_remote { p.remote.clone() } else { None };
                    Ok(format!("{{\"local\":{},\"remote\":{},\"currentLocal\":{},\"currentRemote\":{},\"pendingLocal\":{},\"pendingRemote\":{}}}",
                        d(local.clone()), d(p.remote.clone()), d(current_local), d(p.current_remote.clone()), d(pending_local), d(pending_remote)))
                }
                "stats" => Ok(format!("{{\"stats\":{}}}", stats(e.id, p, now))),
                "state" => Ok(String::new()),
                _ => return None,
            }
        };
        self.collect();
        Some(match r {
            Ok(body) => {
                let st = Self::state(&self.peers[k]);
                if body.is_empty() { st } else if body == "{}" { format!("{{\"state\":{st}}}") } else { format!("{},\"state\":{st}}}", &body[..body.len() - 1]) }
            }
            Err(e) => fail(e),
        })
    }

    /// Moves peer events into page dispatches.
    pub fn collect(&mut self) {
        for e in self.peers.iter_mut() {
            while let Some(ev) = e.peer.poll_event() {
                let text = match ev {
                    Event::Candidate { line, mid } => format!("rtc\t{}\tcandidate\t{mid}\t{line}", e.id),
                    Event::Gathering(s) => format!("rtc\t{}\tgathering\t{s}", e.id),
                    Event::Ice(s) => format!("rtc\t{}\tice\t{s}", e.id),
                    Event::Connection(s) => format!("rtc\t{}\tconnection\t{s}", e.id),
                    Event::Unmute(t) => format!("rtc\t{}\tunmute\t{t}", e.id),
                };
                self.events.push((e.owner, text));
            }
        }
    }

    /// The page went away: its peers close and are dropped.
    pub fn page_left(&mut self, owner: u64) {
        let now = self.now;
        for e in self.peers.iter_mut().filter(|e| e.owner == owner) { e.peer.close(now); }
        self.events.retain(|(o, _)| *o != owner);
    }

    /// Peers that are closed and have nothing left to send.
    pub fn take_closed(&mut self) -> Vec<u32> {
        let gone: Vec<u32> = self.peers.iter().filter(|e| e.peer.closed && e.peer.poll_timeout().is_none()).map(|e| e.id).collect();
        gone
    }
    pub fn remove(&mut self, id: u32) { self.peers.retain(|e| e.id != id); }

    pub fn wants_transport(&self) -> Vec<u32> { self.peers.iter().filter(|e| e.peer.wants_transport()).map(|e| e.id).collect() }

    pub fn start_transport(&mut self, id: u32, hosts: &[SocketAddr], servers: &[Option<SocketAddr>]) -> Vec<usize> {
        let now = self.now;
        let r = self.peer(id).map(|p| p.start_transport(now, hosts, servers)).unwrap_or_default();
        self.collect();
        r
    }

    pub fn receive(&mut self, id: u32, local: SocketAddr, from: SocketAddr, data: &[u8]) {
        let now = self.now;
        if let Some(p) = self.peer(id) { p.receive(now, local, from, data); }
    }

    pub fn handle_timeout(&mut self) {
        let now = self.now;
        for e in self.peers.iter_mut() {
            if e.peer.poll_timeout().is_some_and(|t| t <= now) || !e.peer.closed { e.peer.handle_timeout(now); }
        }
        self.collect();
    }

    pub fn poll_timeout(&self) -> Option<u64> { self.peers.iter().filter_map(|e| e.peer.poll_timeout()).min() }

    /// Encodes and sends a camera (or other) picture on every transceiver sending `source`.
    /// Returns how many frames went out.
    pub fn send_picture(&mut self, source: &str, px: &[u32], w: usize, h: usize) -> usize {
        let now = self.now;
        let mut sent = 0;
        let mut scaled = Vec::new();
        for e in self.peers.iter_mut() {
            let p = &mut e.peer;
            if !p.media_ready() { continue; }
            for ti in 0..p.transceivers.len() {
                let target = p.target_bps(ti);
                let t = &mut p.transceivers[ti];
                if t.kind != Kind::Video || !t.sending() || t.source.as_deref() != Some(source) || t.send_codec().map(|c| c.codec) != Some(Codec::Vp8) { continue; }
                let enc0 = t.encodings.first().cloned().unwrap_or_default();
                if !enc0.active || now < t.next_frame_ms { continue; }
                let fps = enc0.max_framerate.unwrap_or(15).clamp(1, 30);
                t.next_frame_ms = now + 1000 / fps as u64;
                // Smaller pictures when the bitrate is low.
                let mut scale = enc0.scale_down.max(1.0);
                if target < 300_000 && w as f32 / scale > 640.0 { scale *= 2.0; }
                let (ew, eh) = (((w as f32 / scale) as usize).max(16) & !1, ((h as f32 / scale) as usize).max(16) & !1);
                let pic: &[u32] = if (ew, eh) == (w, h) { px } else {
                    scaled.resize(ew * eh, 0);
                    for y in 0..eh { for x in 0..ew { scaled[y * ew + x] = px[(y * h / eh) * w + x * w / ew]; } }
                    &scaled
                };
                let enc = t.encoder.get_or_insert_with(|| vp8::Encoder::new(ew, eh));
                if (enc.width, enc.height) != (ew, eh) { *enc = vp8::Encoder::new(ew, eh); }
                if core::mem::take(&mut t.want_keyframe) { enc.request_keyframe(); }
                enc.set_rate(target, fps);
                let f = enc.encode_argb(pic, ew, eh);
                let base = *t.video_ts_base.get_or_insert(now);
                let ts = ((now - base) * 90) as u32 ^ t.ssrc;
                t.sent.width = ew as u32;
                t.sent.height = eh as u32;
                p.send_frame(now, ti, &f.data, f.keyframe, ts);
                sent += 1;
            }
        }
        sent
    }

    /// Microphone (or other) sound, 48 kHz stereo, for every transceiver sending `source`.
    pub fn send_sound(&mut self, source: &str, frames: &[[i16; 2]]) {
        let now = self.now;
        let mut down = Vec::new();
        g711::downsample(frames, &mut down);
        for e in self.peers.iter_mut() {
            let p = &mut e.peer;
            if !p.media_ready() { continue; }
            for ti in 0..p.transceivers.len() {
                let t = &mut p.transceivers[ti];
                if t.kind != Kind::Audio || !t.sending() || t.source.as_deref() != Some(source) { continue; }
                let Some(codec) = t.send_codec().map(|c| c.codec) else { continue };
                if !matches!(codec, Codec::Pcmu | Codec::Pcma) { continue; }
                t.audio_buf.extend_from_slice(&down);
                // Keep at most a second waiting.
                let n = t.audio_buf.len();
                if n > 8000 { t.audio_buf.drain(..n - 8000); }
                while p.transceivers[ti].audio_buf.len() >= 160 {
                    let t = &mut p.transceivers[ti];
                    let chunk: Vec<u8> = t.audio_buf.drain(..160).map(|s| if codec == Codec::Pcmu { g711::ulaw_encode(s) } else { g711::alaw_encode(s) }).collect();
                    let ts = t.audio_ts;
                    t.audio_ts = ts.wrapping_add(160);
                    p.send_frame(now, ti, &chunk, true, ts);
                }
            }
        }
    }

    /// The next frame that arrived: the page owner, the track's source name
    /// (`rtc:ID:N`, what a stream element shows) and the frame.
    pub fn poll_frame(&mut self) -> Option<(u64, String, Received)> {
        for e in self.peers.iter_mut() {
            if let Some(f) = e.peer.poll_frame() { return Some((e.owner, format!("rtc:{}:{}", e.id, f.transceiver), f)); }
        }
        None
    }

    fn by_key(&mut self, key: &str) -> Option<(&mut Peer, usize)> {
        let mut f = key.strip_prefix("rtc:")?.split(':');
        let (id, t): (u32, usize) = (f.next()?.parse().ok()?, f.next()?.parse().ok()?);
        Some((self.peer(id)?, t))
    }

    /// The browser decoded a picture of track `key`.
    pub fn decoded(&mut self, key: &str, w: u32, h: u32) {
        if let Some((p, t)) = self.by_key(key) {
            if let Some(t) = p.transceivers.get_mut(t) { t.received.decoded += 1; t.received.width = w; t.received.height = h; }
        }
    }

    /// The decoder of track `key` needs a key frame.
    pub fn request_keyframe(&mut self, key: &str) {
        let now = self.now;
        if let Some((p, t)) = self.by_key(key) { p.request_keyframe(now, t); }
    }
}

fn mime_case(c: Codec) -> String { format!("{}/{}", if c.is_audio() { "audio" } else { "video" }, c.name()) }

/// getStats: RTCStatsReport entries as a JSON array.
pub fn stats(id: u32, p: &Peer, now: u64) -> String {
    let ts = now as f64;
    let mut v: Vec<String> = Vec::new();
    let tid = format!("T{id}");
    let sel = p.selected();
    v.push(format!("{{\"type\":\"peer-connection\",\"id\":\"P{id}\",\"timestamp\":{ts},\"dataChannelsOpened\":0,\"dataChannelsClosed\":0}}"));
    let pair = sel.map(|(l, r)| {
        let rtt = p.rtt_ms.map_or(String::new(), |r| format!(",\"currentRoundTripTime\":{}", r as f64 / 1000.0));
        format!("{{\"type\":\"candidate-pair\",\"id\":\"CP{id}\",\"timestamp\":{ts},\"transportId\":\"{tid}\",\"localCandidateId\":\"L{id}\",\"remoteCandidateId\":\"R{id}\",\"state\":\"succeeded\",\"nominated\":true,\"bytesSent\":{},\"bytesReceived\":{},\"packetsSent\":{},\"packetsReceived\":{},\"availableOutgoingBitrate\":{}{rtt}}}",
            p.bytes_sent, p.bytes_received, p.packets_sent, p.packets_received, p.bwe.target_bps())
            + &format!(",{{\"type\":\"local-candidate\",\"id\":\"L{id}\",\"timestamp\":{ts},\"transportId\":\"{tid}\",\"address\":\"{}\",\"port\":{},\"protocol\":\"udp\",\"candidateType\":\"host\"}}", l.ip(), l.port())
            + &format!(",{{\"type\":\"remote-candidate\",\"id\":\"R{id}\",\"timestamp\":{ts},\"transportId\":\"{tid}\",\"address\":\"{}\",\"port\":{},\"protocol\":\"udp\",\"candidateType\":\"host\"}}", r.ip(), r.port())
    });
    v.push(format!("{{\"type\":\"transport\",\"id\":\"{tid}\",\"timestamp\":{ts},\"bytesSent\":{},\"bytesReceived\":{},\"packetsSent\":{},\"packetsReceived\":{},\"dtlsState\":\"{}\",\"iceState\":\"{}\",\"iceRole\":\"{}\"{}}}",
        p.bytes_sent, p.bytes_received, p.packets_sent, p.packets_received, p.dtls_state, p.ice_state, "controlling", if sel.is_some() { format!(",\"selectedCandidatePairId\":\"CP{id}\"") } else { String::new() }));
    if let Some(pair) = pair { v.push(pair); }
    for (i, t) in p.transceivers.iter().enumerate() {
        let kind = if t.kind == Kind::Audio { "audio" } else { "video" };
        let mid = t.mid.as_deref().map_or(String::from("null"), json_str);
        if let Some(c) = t.send_codec() {
            v.push(format!("{{\"type\":\"codec\",\"id\":\"C{id}_{i}\",\"timestamp\":{ts},\"transportId\":\"{tid}\",\"payloadType\":{},\"mimeType\":{},\"clockRate\":{}}}", c.pt, json_str(&mime_case(c.codec)), c.codec.clock()));
        }
        if t.sent.packets > 0 || t.sending() {
            let s = &t.sent;
            let mut o = format!("{{\"type\":\"outbound-rtp\",\"id\":\"OT{id}_{i}\",\"timestamp\":{ts},\"kind\":\"{kind}\",\"mediaType\":\"{kind}\",\"ssrc\":{},\"mid\":{mid},\"transportId\":\"{tid}\",\"codecId\":\"C{id}_{i}\",\"bytesSent\":{},\"packetsSent\":{},\"headerBytesSent\":{},\"retransmittedBytesSent\":{},\"retransmittedPacketsSent\":{},\"nackCount\":{},\"pliCount\":{},\"firCount\":{},\"active\":true",
                t.ssrc, s.bytes, s.packets, s.header_bytes, s.retransmitted_bytes, s.retransmitted_packets, s.nacks, s.plis, s.firs);
            if t.kind == Kind::Video {
                let _ = write!(o, ",\"framesEncoded\":{},\"framesSent\":{},\"keyFramesEncoded\":{},\"frameWidth\":{},\"frameHeight\":{},\"targetBitrate\":{},\"encoderImplementation\":\"Kaleidoscope VP8\"",
                    s.frames, s.frames, s.keyframes, s.width, s.height, p.target_bps(i));
                if let Some(r) = t.encodings.first().filter(|e| !e.rid.is_empty()) { let _ = write!(o, ",\"rid\":{}", json_str(&r.rid)); }
            }
            o.push('}');
            v.push(o);
        }
        if t.received.packets > 0 || t.receiving() {
            let r = &t.received;
            let mut o = format!("{{\"type\":\"inbound-rtp\",\"id\":\"IT{id}_{i}\",\"timestamp\":{ts},\"kind\":\"{kind}\",\"mediaType\":\"{kind}\",\"ssrc\":{},\"mid\":{mid},\"transportId\":\"{tid}\",\"trackIdentifier\":{},\"bytesReceived\":{},\"packetsReceived\":{},\"headerBytesReceived\":{},\"packetsLost\":0,\"nackCount\":{},\"pliCount\":{},\"firCount\":{}",
                t.remote_ssrc.unwrap_or(0), json_str(&t.remote_track), r.bytes, r.packets, r.header_bytes, r.nacks, r.plis, r.firs);
            if r.packets > 0 { let _ = write!(o, ",\"lastPacketReceivedTimestamp\":{}", r.last_packet_ms); }
            if t.kind == Kind::Video {
                let _ = write!(o, ",\"framesReceived\":{},\"framesDecoded\":{},\"keyFramesDecoded\":{},\"frameWidth\":{},\"frameHeight\":{}", r.frames, r.decoded, r.keyframes, r.width, r.height);
            }
            o.push('}');
            v.push(o);
        }
    }
    format!("[{}]", v.join(","))
}
