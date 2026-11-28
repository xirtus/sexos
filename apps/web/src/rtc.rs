// SPDX-License-Identifier: GPL-3.0-only
//! WebRTC for pages (`RTCPeerConnection`): the sockets and the codecs around sex-rtc.
//!
//! sex-rtc keeps every peer connection's JSEP, ICE, DTLS-SRTP and RTP state and answers
//! the page's op::RTC calls itself (shared with the page's Document, like the capture
//! choices). Here the browser does what needs the machine: a UDP socket per peer
//! (`sex_app::net::UdpSocket`, host candidates on the machine's addresses), STUN and
//! TURN server names resolved, datagrams in and out each tick; the camera's pictures
//! and the microphone's sound handed to the senders (VP8 and G.711 from sex-rtc); and
//! what arrives decoded with sex-media (VP8, VP9, H.264, Opus; G.711 here) into the
//! live pictures `rtc:ID:N` that stream elements show, and remote sound played.
//! TURN over TCP and TLS is not wired yet (UDP TURN is).
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use sex_app::net::{self, UdpSocket};
use sex_app::Window;
use sex_rtc::Codec;
use sex_web::Image;

const MAX_W: usize = crate::media::MAX_W;
const MAX_H: usize = crate::media::MAX_H;

struct Socket { peer: u32, socket: UdpSocket, hosts: Vec<SocketAddr> }

enum Decoder { Media(sex_media::frames::FrameDecoder), Ulaw, Alaw, None }

struct Remote {
    key: String,
    codec: Codec,
    decoder: Decoder,
    pixels: Vec<u32>,
    pub frames: u32,
    errors: u32,
}

pub struct Rtc {
    pub shared: Rc<RefCell<sex_rtc::Rtc>>,
    sockets: Vec<Socket>,
    remotes: Vec<Remote>,
    /// The newest picture of each remote video track: (source, picture, seq).
    pub pictures: Vec<(String, Rc<Image>, u32)>,
    /// Remote sound waiting to play, 48 kHz stereo, mixed.
    sound_out: Vec<[i16; 2]>,
    speaker: Option<sex_app::sound::Sound>,
    /// Remote sound sources a shown, unmuted element plays.
    pub audible: Vec<String>,
    buf: Vec<u8>,
    logged: u64,
}

fn to_core(ip: &net::IpAddr) -> IpAddr {
    match ip { net::IpAddr::V4(a) => IpAddr::V4(Ipv4Addr::from(*a)), net::IpAddr::V6(a) => IpAddr::V6(Ipv6Addr::from(*a)) }
}
fn to_app(ip: IpAddr) -> net::IpAddr {
    match ip { IpAddr::V4(a) => net::IpAddr::V4(a.octets()), IpAddr::V6(a) => net::IpAddr::V6(a.octets()) }
}

/// A seed for certificates, ICE credentials and SSRCs: the CPU's random numbers when it
/// has them, mixed with its cycle counter.
fn seed() -> [u8; 32] {
    let mut s = [0u8; 32];
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let rdrand = core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0;
        for (i, c) in s.chunks_mut(8).enumerate() {
            let mut v = core::arch::x86_64::_rdtsc().rotate_left(i as u32 * 17) ^ sex_pdx::get_ticks().wrapping_mul(0x9e37_79b9_7f4a_7c15);
            if rdrand {
                let mut r = 0u64;
                for _ in 0..10 { if core::arch::x86_64::_rdrand64_step(&mut r) == 1 { break; } }
                v ^= r;
            }
            c.copy_from_slice(&v.to_le_bytes());
        }
    }
    s
}

impl Rtc {
    pub fn new() -> Rtc {
        Rtc { shared: Rc::new(RefCell::new(sex_rtc::Rtc::new(seed()))), sockets: Vec::new(), remotes: Vec::new(), pictures: Vec::new(),
            sound_out: Vec::new(), speaker: None, audible: Vec::new(), buf: alloc::vec![0; 2048], logged: 0 }
    }

    /// What a page's Document calls for op::RTC.
    pub fn hook(&self) -> Rc<dyn Fn(u64, &[&str]) -> Option<String>> {
        let shared = self.shared.clone();
        Rc::new(move |owner, args: &[&str]| {
            let mut r = shared.borrow_mut();
            r.now = sex_pdx::get_ticks();
            let out = r.call(owner, args);
            if args.first().is_some_and(|v| *v != "stats" && *v != "desc") {
                sex_pdx::serial_record!("[web.rtc.call] verb={} ok={}", args[0], out.as_ref().is_some_and(|o| !o.starts_with('!')) as u8);
            }
            out
        })
    }

    pub fn idle(&self) -> bool { self.shared.borrow().peers.is_empty() }

    /// The page went away: its peers close (their last datagrams still go out).
    pub fn page_left(&mut self, owner: u64) {
        let mut r = self.shared.borrow_mut();
        r.now = sex_pdx::get_ticks();
        r.page_left(owner);
    }

    fn bind(&mut self, window: &mut Window, id: u32) {
        let servers = self.shared.borrow_mut().peer(id).map(|p| p.config().servers.clone()).unwrap_or_default();
        let socket = match UdpSocket::bind(window, 0) {
            Ok(s) => s,
            Err(e) => {
                sex_pdx::serial_record!("[web.rtc.socket] peer={} ok=0 error={:?}", id, e);
                let mut r = self.shared.borrow_mut();
                r.now = sex_pdx::get_ticks();
                r.start_transport(id, &[], &[]);
                return;
            }
        };
        let port = socket.local_port();
        let mut hosts: Vec<SocketAddr> = Vec::new();
        for (ip, _) in net::addresses(window).unwrap_or_default() {
            let ip = to_core(&ip);
            let usable = match ip {
                IpAddr::V4(a) => !a.is_unspecified() && !a.is_loopback(),
                // Link-local IPv6 needs a scope the candidate cannot carry.
                IpAddr::V6(a) => !a.is_unspecified() && !a.is_loopback() && (a.segments()[0] & 0xffc0) != 0xfe80,
            };
            if usable { hosts.push(SocketAddr::new(ip, port)); }
        }
        // ICE servers by name: the first address of the family the sockets have.
        let mut resolved: Vec<Option<SocketAddr>> = Vec::new();
        for s in &servers {
            let got = net::resolve(window, s.host.as_bytes()).ok().and_then(|list| {
                let list: Vec<IpAddr> = list.iter().map(to_core).collect();
                list.iter().find(|a| hosts.iter().any(|h| h.is_ipv4() == a.is_ipv4())).or(list.first()).copied()
            });
            resolved.push(got.map(|ip| SocketAddr::new(ip, s.port)));
        }
        sex_pdx::serial_record!("[web.rtc.socket] peer={} ok=1 port={} hosts={} servers={}/{}", id, port, hosts.len(), resolved.iter().filter(|r| r.is_some()).count(), servers.len());
        let mut r = self.shared.borrow_mut();
        r.now = sex_pdx::get_ticks();
        r.start_transport(id, &hosts, &resolved);
        drop(r);
        self.sockets.push(Socket { peer: id, socket, hosts });
    }

    /// Moves everything along. `camera`: a new camera picture to send, if one came;
    /// `mic`: microphone sound since the last tick. Returns the dispatches for pages
    /// (owner, text) and how soon to come back.
    pub fn tick(&mut self, window: &mut Window, camera: Option<&Rc<Image>>, mic: &[[i16; 2]]) -> (Vec<(u64, String)>, Option<u64>) {
        if self.idle() && self.sockets.is_empty() { return (Vec::new(), None); }
        let now = sex_pdx::get_ticks();
        let want = { let mut r = self.shared.borrow_mut(); r.now = now; r.wants_transport() };
        for id in want { self.bind(window, id); }
        // Datagrams in.
        for k in 0..self.sockets.len() {
            for _ in 0..256 {
                let s = &mut self.sockets[k];
                let Ok(Some((n, from, port))) = s.socket.recv_from(window, &mut self.buf) else { break };
                let from = SocketAddr::new(to_core(&from), port);
                let Some(local) = s.hosts.iter().find(|h| h.is_ipv4() == from.is_ipv4()).copied() else { continue };
                let id = s.peer;
                let mut r = self.shared.borrow_mut();
                r.now = sex_pdx::get_ticks();
                r.receive(id, local, from, &self.buf[..n]);
            }
        }
        {
            let mut r = self.shared.borrow_mut();
            r.now = sex_pdx::get_ticks();
            r.handle_timeout();
            if let Some(pic) = camera { r.send_picture("camera", &pic.pixels, pic.width, pic.height); }
            if !mic.is_empty() { r.send_sound("mic", mic); }
        }
        self.flush(window);
        self.receive_frames();
        self.play();
        // Closed peers with nothing left to send let their sockets go.
        let gone = self.shared.borrow_mut().take_closed();
        for id in gone {
            self.shared.borrow_mut().remove(id);
            while let Some(i) = self.sockets.iter().position(|s| s.peer == id) { let s = self.sockets.swap_remove(i); s.socket.close(window); }
            let prefix = alloc::format!("rtc:{id}:");
            self.remotes.retain(|r| !r.key.starts_with(&prefix));
            self.pictures.retain(|p| !p.0.starts_with(&prefix));
            sex_pdx::serial_record!("[web.rtc.closed] peer={}", id);
        }
        if now >= self.logged + 2000 {
            self.logged = now;
            let r = self.shared.borrow();
            for e in &r.peers {
                let p = &e.peer;
                let v = |send: bool| p.transceivers.iter().filter(|t| t.kind == sex_rtc::Kind::Video).map(|t| if send { t.sent.bytes } else { t.received.bytes }).sum::<u64>();
                sex_pdx::serial_record!("[web.rtc.stats] peer={} connection={} ice={} sent={} received={} video_sent={} video_received={} target={}",
                    e.id, p.connection, p.ice_state, p.bytes_sent, p.bytes_received, v(true), v(false), p.bwe.target_bps());
            }
        }
        let mut r = self.shared.borrow_mut();
        r.collect();
        let events = core::mem::take(&mut r.events);
        for (_, e) in &events {
            if e.contains("\tconnection\t") || e.contains("\tice\t") || e.contains("\tunmute\t") { sex_pdx::serial_record!("[web.rtc.event] {}", e.replace('\t', " ")); }
        }
        let next = r.poll_timeout().map(|t| t.saturating_sub(sex_pdx::get_ticks()).clamp(1, 20)).or(if r.peers.is_empty() { None } else { Some(20) });
        (events, next)
    }

    fn flush(&mut self, window: &mut Window) {
        let ids: Vec<u32> = self.shared.borrow().peers.iter().map(|e| e.id).collect();
        for id in ids {
            loop {
                let t = self.shared.borrow_mut().peer(id).and_then(|p| p.poll_transmit());
                let Some((route, data)) = t else { break };
                let sex_ice::Route::Udp { to, .. } = route else { continue };
                if let Some(s) = self.sockets.iter_mut().find(|s| s.peer == id) {
                    let _ = s.socket.send_to(window, &data, &to_app(to.ip()), to.port());
                }
            }
        }
    }

    fn receive_frames(&mut self) {
        loop {
            let f = self.shared.borrow_mut().poll_frame();
            let Some((_, key, f)) = f else { break };
            let i = match self.remotes.iter().position(|r| r.key == key && r.codec == f.codec) {
                Some(i) => i,
                None => {
                    self.remotes.retain(|r| r.key != key);
                    let decoder = match f.codec {
                        Codec::Pcmu => Decoder::Ulaw,
                        Codec::Pcma => Decoder::Alaw,
                        c => {
                            let mc = match c { Codec::Vp8 => sex_media::frames::Codec::Vp8, Codec::Vp9 => sex_media::frames::Codec::Vp9, Codec::H264 => sex_media::frames::Codec::H264, _ => sex_media::frames::Codec::Opus };
                            match sex_media::frames::FrameDecoder::new(mc) {
                                Ok(d) => Decoder::Media(d),
                                Err(e) => { sex_pdx::serial_record!("[web.rtc.decoder] key={} codec={:?} ok=0 error={}", key, c, e.0); Decoder::None }
                            }
                        }
                    };
                    self.remotes.push(Remote { key: key.clone(), codec: f.codec, decoder, pixels: Vec::new(), frames: 0, errors: 0 });
                    self.remotes.len() - 1
                }
            };
            let r = &mut self.remotes[i];
            match &mut r.decoder {
                Decoder::Media(d) if !f.codec.is_audio() => match d.picture(&f.data, &mut r.pixels, MAX_W, MAX_H) {
                    Ok(Some(p)) => {
                        r.frames += 1;
                        let img = Rc::new(Image { width: p.shown_w, height: p.shown_h, pixels: core::mem::take(&mut r.pixels) });
                        if r.frames <= 3 || r.frames.is_power_of_two() { sex_pdx::serial_record!("[web.rtc.frame] key={} n={} w={} h={} key_frame={}", key, r.frames, p.width, p.height, f.keyframe as u8); }
                        self.shared.borrow_mut().decoded(&key, p.width as u32, p.height as u32);
                        match self.pictures.iter_mut().find(|x| x.0 == key) {
                            Some(x) => { x.1 = img; x.2 = x.2.wrapping_add(1); }
                            None => self.pictures.push((key.clone(), img, 1)),
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        r.errors += 1;
                        let mut s = self.shared.borrow_mut();
                        s.now = sex_pdx::get_ticks();
                        s.request_keyframe(&key);
                    }
                },
                Decoder::Media(d) => {
                    let mut pcm = Vec::new();
                    if d.sound(&f.data, &mut pcm).is_ok() && self.audible.contains(&key) { mix(&mut self.sound_out, &pcm); }
                }
                Decoder::Ulaw | Decoder::Alaw => {
                    let u = matches!(r.decoder, Decoder::Ulaw);
                    let s: Vec<i16> = f.data.iter().map(|&b| if u { sex_rtc::g711::ulaw_decode(b) } else { sex_rtc::g711::alaw_decode(b) }).collect();
                    let mut pcm = Vec::new();
                    let mut last = 0;
                    sex_rtc::g711::upsample(&s, &mut last, &mut pcm);
                    r.frames += 1;
                    if r.frames == 1 || r.frames.is_power_of_two() && r.frames >= 64 { sex_pdx::serial_record!("[web.rtc.sound] key={} packets={}", key, r.frames); }
                    if self.audible.contains(&key) { mix(&mut self.sound_out, &pcm); }
                }
                Decoder::None => {}
            }
        }
    }

    /// Remote sound to the speakers, when the page shows (and has not muted) it.
    fn play(&mut self) {
        if self.sound_out.is_empty() { return; }
        if self.speaker.is_none() { self.speaker = sex_app::sound::Sound::open(); }
        let out = core::mem::take(&mut self.sound_out);
        if let Some(s) = self.speaker.as_mut() {
            let room = s.room().min(out.len());
            s.write(&out[..room]);
            s.kick();
        }
    }

    /// The remote picture `key` shows now, if one came: (picture, seq).
    pub fn picture(&self, key: &str) -> Option<(Rc<Image>, u32)> {
        self.pictures.iter().find(|p| p.0 == key).map(|p| (p.1.clone(), p.2))
    }
}

/// Adds `pcm` into `out`, sample by sample from the start.
fn mix(out: &mut Vec<[i16; 2]>, pcm: &[[i16; 2]]) {
    if out.len() < pcm.len() { out.resize(pcm.len(), [0; 2]); }
    for (o, p) in out.iter_mut().zip(pcm) {
        o[0] = o[0].saturating_add(p[0]);
        o[1] = o[1].saturating_add(p[1]);
    }
}
