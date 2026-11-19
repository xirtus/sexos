// SPDX-License-Identifier: GPL-3.0-only
//! The device runtime over std::net: it announces itself by UDP, takes links on its
//! TCP port, opens links to devices it hears, runs the pairing, and moves files on
//! payload connections. Events come out of a channel; the person's actions are
//! [`Daemon`]'s methods. Hookup runs it in the guest; the host tests run the same code
//! against a Python peer.
//!
//! Links are plain TCP for now (see the crate's note on encryption): every
//! [`Event::Linked`] says `encrypted: false`.
use crate::plugins::{self, Action, Media, Notification, Share};
use crate::{discovered, pair_packet, safe_file_name, Discovery, Framer, Identity, Packet, PairOutcome, Pairing, Trusted, PAIR, PING};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct Config {
    pub device_id: String,
    pub name: String,
    /// Where identity datagrams and links are taken (1716 normally).
    pub port: u16,
    /// Where to send identity datagrams besides the broadcast address.
    pub announce: Vec<SocketAddr>,
    /// Also announce to 255.255.255.255.
    pub broadcast: bool,
    /// The folder: `trusted.txt` and `received/` live here.
    pub folder: PathBuf,
    /// The first port tried when offering a file.
    pub payload_port: u16,
}

#[derive(Debug, Clone)]
pub enum Event {
    Listening { port: u16 },
    Announced { to: SocketAddr, ok: bool },
    Linked { device: Identity, addr: SocketAddr, incoming: bool, encrypted: bool, paired: bool },
    Unlinked { device_id: String },
    PairAsked { device_id: String, name: String },
    Paired { device_id: String, name: String },
    Unpaired { device_id: String, reason: &'static str },
    Notification { device_id: String, notification: Notification },
    Clipboard { device_id: String, text: String },
    Media { device_id: String, media: Media },
    Ping { device_id: String },
    SharedText { device_id: String, text: String },
    /// A file arrived and is in the folder at `path`.
    Received { device_id: String, name: String, path: PathBuf, bytes: u64 },
    ReceiveFailed { device_id: String, name: String, reason: String },
    /// A file offered was fetched by the peer.
    Sent { device_id: String, name: String, bytes: u64 },
    SendFailed { device_id: String, name: String, reason: String },
    /// A packet from a device that is not paired, left unanswered.
    Ignored { device_id: String, kind: String },
    Problem(String),
}

struct Link {
    identity: Identity,
    writer: Arc<Mutex<TcpStream>>,
    pairing: Pairing,
    serial: u64,
}

struct Shared {
    own: Identity,
    config: Config,
    links: Mutex<HashMap<String, Link>>,
    trusted: Mutex<Trusted>,
    events: Mutex<Sender<Event>>,
    started: Instant,
    serial: Mutex<u64>,
}

/// The running device. Cloning it shares the same one.
#[derive(Clone)]
pub struct Daemon { shared: Arc<Shared> }

fn packet_id() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

impl Shared {
    fn emit(&self, e: Event) { let _ = self.events.lock().unwrap_or_else(|e| e.into_inner()).send(e); }
    fn now(&self) -> u64 { self.started.elapsed().as_millis() as u64 }
    fn trusted_path(&self) -> PathBuf { self.config.folder.join("trusted.txt") }
    fn save_trusted(&self, t: &Trusted) {
        if let Err(e) = std::fs::write(self.trusted_path(), t.serialize()) { self.emit(Event::Problem(format!("could not keep trusted.txt: {e}"))); }
    }
    fn send(&self, device_id: &str, p: &Packet) -> Result<(), String> {
        let writer = { let links = self.links.lock().unwrap(); links.get(device_id).map(|l| l.writer.clone()) };
        let Some(w) = writer else { return Err("not linked".into()) };
        let mut w = w.lock().unwrap_or_else(|e| e.into_inner());
        w.write_all(p.to_line().as_bytes()).map_err(|e| e.to_string())
    }
    /// Carries out pairing outcomes for a device.
    fn outcomes(&self, device_id: &str, name: &str, out: Vec<PairOutcome>) {
        for o in out {
            match o {
                PairOutcome::Send(b) => { if let Err(e) = self.send(device_id, &pair_packet(b, packet_id())) { self.emit(Event::Problem(format!("pair answer not sent: {e}"))); } }
                PairOutcome::Ask => self.emit(Event::PairAsked { device_id: device_id.into(), name: name.into() }),
                PairOutcome::NowPaired => {
                    let mut t = self.trusted.lock().unwrap();
                    t.add(device_id, name);
                    self.save_trusted(&t);
                    drop(t);
                    self.emit(Event::Paired { device_id: device_id.into(), name: name.into() });
                }
                PairOutcome::NowUnpaired(reason) => {
                    let mut t = self.trusted.lock().unwrap();
                    if t.contains(device_id) { t.remove(device_id); self.save_trusted(&t); }
                    drop(t);
                    self.emit(Event::Unpaired { device_id: device_id.into(), reason });
                }
            }
        }
    }
    fn with_pairing(&self, device_id: &str, f: impl FnOnce(&mut Pairing, u64) -> Vec<PairOutcome>) {
        let now = self.now();
        let (name, out) = {
            let mut links = self.links.lock().unwrap();
            let Some(l) = links.get_mut(device_id) else { return };
            (l.identity.device_name.clone(), f(&mut l.pairing, now))
        };
        self.outcomes(device_id, &name, out);
    }
    fn paired(&self, device_id: &str) -> bool { self.links.lock().unwrap().get(device_id).is_some_and(|l| l.pairing.paired()) }
}

impl Daemon {
    /// Starts the device: its UDP and TCP listeners, and one announcement.
    pub fn start(config: Config) -> std::io::Result<(Daemon, Receiver<Event>)> {
        let (tx, rx) = channel();
        let _ = std::fs::create_dir_all(config.folder.join("received"));
        let trusted = Trusted::parse(&std::fs::read_to_string(config.folder.join("trusted.txt")).unwrap_or_default());
        let own = Identity::hookup(&config.device_id, &config.name, Some(config.port));
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, config.port))?;
        let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, config.port))?;
        let shared = Arc::new(Shared { own, config, links: Mutex::new(HashMap::new()), trusted: Mutex::new(trusted), events: Mutex::new(tx), started: Instant::now(), serial: Mutex::new(0) });
        let d = Daemon { shared };
        d.shared.emit(Event::Listening { port: d.shared.config.port });
        let me = d.clone();
        std::thread::spawn(move || loop {
            match listener.accept() {
                Ok((s, addr)) => { let me = me.clone(); std::thread::spawn(move || me.incoming(s, addr)); }
                Err(e) => { me.shared.emit(Event::Problem(format!("accept: {e}"))); std::thread::sleep(Duration::from_millis(500)); }
            }
        });
        let me = d.clone();
        let udp2 = udp.try_clone()?;
        std::thread::spawn(move || me.listen_udp(udp2));
        d.announce_on(&udp);
        Ok((d, rx))
    }

    pub fn identity(&self) -> &Identity { &self.shared.own }

    fn announce_on(&self, udp: &UdpSocket) {
        let line = self.shared.own.packet(packet_id()).to_line();
        let mut to = self.shared.config.announce.clone();
        if self.shared.config.broadcast {
            let _ = udp.set_broadcast(true);
            to.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), self.shared.config.port));
        }
        for addr in to {
            let ok = udp.send_to(line.as_bytes(), addr).is_ok();
            self.shared.emit(Event::Announced { to: addr, ok });
        }
    }

    /// Sends the identity datagram again.
    pub fn announce(&self) {
        match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
            Ok(u) => self.announce_on(&u),
            Err(e) => self.shared.emit(Event::Problem(format!("announce: {e}"))),
        }
    }

    fn listen_udp(&self, udp: UdpSocket) {
        let mut buf = vec![0u8; 65536];
        loop {
            let (n, from) = match udp.recv_from(&mut buf) { Ok(x) => x, Err(_) => { std::thread::sleep(Duration::from_millis(200)); continue; } };
            let Ok(p) = Packet::parse(&buf[..n]) else { continue };
            let Ok(peer) = Identity::from_packet(&p) else { continue };
            let linked = self.shared.links.lock().unwrap().contains_key(&peer.device_id);
            if let Discovery::Connect(port) = discovered(&self.shared.own.device_id, &peer, linked) {
                let me = self.clone();
                let addr = SocketAddr::new(from.ip(), port);
                std::thread::spawn(move || me.outgoing(addr));
            }
        }
    }

    /// A link we open to a device we heard: we speak first.
    fn outgoing(&self, addr: SocketAddr) {
        let mut s = match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
            Ok(s) => s,
            Err(e) => { self.shared.emit(Event::Problem(format!("link to {addr}: {e}"))); return; }
        };
        if s.write_all(self.shared.own.packet(packet_id()).to_line().as_bytes()).is_err() { return; }
        // Their identity is the first thing they say back over the link.
        self.run_link(s, addr, false);
    }

    /// A link a device opened to us after hearing our announcement.
    fn incoming(&self, s: TcpStream, addr: SocketAddr) { self.run_link(s, addr, true); }

    fn run_link(&self, mut s: TcpStream, addr: SocketAddr, incoming: bool) {
        let mut framer = Framer::new();
        let mut buf = vec![0u8; 16384];
        let mut pending: Vec<Vec<u8>> = Vec::new();
        let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
        // The first line names the device.
        let identity = loop {
            if !pending.is_empty() {
                let line = pending.remove(0);
                match Packet::parse(&line).ok().and_then(|p| Identity::from_packet(&p).ok()) {
                    Some(i) => break i,
                    None => { self.shared.emit(Event::Problem(format!("link from {addr} did not start with an identity"))); return; }
                }
            }
            match s.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => match framer.push(&buf[..n]) { Ok(lines) => pending.extend(lines), Err(_) => return },
            }
        };
        if identity.device_id == self.shared.own.device_id { return; }
        if incoming {
            // A device that opened the link to us gets our identity in turn.
            if s.write_all(self.shared.own.packet(packet_id()).to_line().as_bytes()).is_err() { return; }
        }
        let _ = s.set_read_timeout(None);
        let Ok(writer) = s.try_clone() else { return };
        let trusted = self.shared.trusted.lock().unwrap().contains(&identity.device_id);
        let serial = { let mut n = self.shared.serial.lock().unwrap(); *n += 1; *n };
        let id = identity.device_id.clone();
        let name = identity.device_name.clone();
        {
            let mut links = self.shared.links.lock().unwrap();
            if let Some(old) = links.insert(id.clone(), Link { identity: identity.clone(), writer: Arc::new(Mutex::new(writer)), pairing: Pairing::new(trusted), serial }) {
                let _ = old.writer.lock().map(|w| w.shutdown(Shutdown::Both));
            }
        }
        self.shared.emit(Event::Linked { device: identity, addr, incoming, encrypted: false, paired: trusted });
        'link: loop {
            for line in std::mem::take(&mut pending) {
                match Packet::parse(&line) {
                    Ok(p) => self.packet(&id, &name, addr, p),
                    Err(_) => self.shared.emit(Event::Problem(format!("{name} sent a line that is not a packet"))),
                }
            }
            match s.read(&mut buf) {
                Ok(0) | Err(_) => break 'link,
                Ok(n) => match framer.push(&buf[..n]) { Ok(lines) => pending = lines, Err(_) => break 'link },
            }
        }
        let mut links = self.shared.links.lock().unwrap();
        if links.get(&id).is_some_and(|l| l.serial == serial) {
            links.remove(&id);
            drop(links);
            self.shared.emit(Event::Unlinked { device_id: id });
        }
    }

    fn packet(&self, id: &str, name: &str, addr: SocketAddr, p: Packet) {
        let sh = &self.shared;
        if p.kind == PAIR {
            let pair = p.bool("pair").unwrap_or(false);
            sh.with_pairing(id, |pg, now| pg.received(pair, now));
            return;
        }
        if !sh.paired(id) {
            sh.emit(Event::Ignored { device_id: id.into(), kind: p.kind.clone() });
            return;
        }
        let dev = || id.to_string();
        if p.kind == PING { sh.emit(Event::Ping { device_id: dev() }); return; }
        if let Some(n) = plugins::notification(&p) { sh.emit(Event::Notification { device_id: dev(), notification: n }); return; }
        if let Some(t) = plugins::clipboard(&p) { sh.emit(Event::Clipboard { device_id: dev(), text: t }); return; }
        if let Some(m) = plugins::media(&p) { sh.emit(Event::Media { device_id: dev(), media: m }); return; }
        match plugins::share(&p) {
            Some(Share::File { name: file, size, port: Some(port) }) => {
                let me = self.clone();
                let from = SocketAddr::new(addr.ip(), port);
                let device_id = dev();
                std::thread::spawn(move || me.fetch(&device_id, &file, size, from));
            }
            Some(Share::File { name: file, .. }) => sh.emit(Event::ReceiveFailed { device_id: dev(), name: file, reason: "no payload port given".into() }),
            Some(Share::Text(t)) | Some(Share::Url(t)) => sh.emit(Event::SharedText { device_id: dev(), text: t }),
            None => sh.emit(Event::Ignored { device_id: dev(), kind: format!("{} (from {name})", p.kind) }),
        }
    }

    /// Fetches a file's bytes from the peer's payload port into `received/`.
    fn fetch(&self, device_id: &str, name: &str, size: u64, from: SocketAddr) {
        let dir = self.shared.config.folder.join("received");
        let path = free_name(&dir, &safe_file_name(name));
        let fail = |reason: String| self.shared.emit(Event::ReceiveFailed { device_id: device_id.into(), name: name.into(), reason });
        let mut s = match TcpStream::connect_timeout(&from, Duration::from_secs(10)) { Ok(s) => s, Err(e) => return fail(format!("payload {from}: {e}")) };
        let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
        let mut data = Vec::with_capacity(size.min(64 << 20) as usize);
        let mut buf = vec![0u8; 32768];
        while (data.len() as u64) < size {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&buf[..n.min((size - data.len() as u64) as usize)]),
                Err(e) => return fail(format!("payload read: {e}")),
            }
        }
        if (data.len() as u64) != size { return fail(format!("payload ended after {} of {size} bytes", data.len())); }
        if let Err(e) = std::fs::write(&path, &data) { return fail(format!("could not write {}: {e}", path.display())); }
        let shown = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.shared.emit(Event::Received { device_id: device_id.into(), name: shown, path, bytes: size });
    }

    // ---- What the person does ----

    pub fn pair(&self, device_id: &str) { self.shared.with_pairing(device_id, |p, now| p.request(now)); }
    pub fn accept(&self, device_id: &str) { self.shared.with_pairing(device_id, |p, _| p.accept()); }
    pub fn reject(&self, device_id: &str) { self.shared.with_pairing(device_id, |p, _| p.reject()); }
    pub fn unpair(&self, device_id: &str) { self.shared.with_pairing(device_id, |p, _| p.unpair()); }
    /// Lets unanswered pair requests lapse; call now and then.
    pub fn tick(&self) {
        let ids: Vec<String> = self.shared.links.lock().unwrap().keys().cloned().collect();
        for id in ids { self.shared.with_pairing(&id, |p, now| p.tick(now)); }
    }
    pub fn linked(&self) -> Vec<(Identity, bool)> {
        self.shared.links.lock().unwrap().values().map(|l| (l.identity.clone(), l.pairing.paired())).collect()
    }

    fn send_paired(&self, device_id: &str, p: &Packet) -> Result<(), String> {
        if !self.shared.paired(device_id) { return Err("not paired".into()); }
        self.shared.send(device_id, p)
    }
    pub fn ping(&self, device_id: &str) -> Result<(), String> { self.send_paired(device_id, &Packet::new(PING, crate::json!({}), packet_id())) }
    pub fn send_clipboard(&self, device_id: &str, text: &str) -> Result<(), String> { self.send_paired(device_id, &plugins::clipboard_packet(text, packet_id())) }
    pub fn request_notifications(&self, device_id: &str) -> Result<(), String> { self.send_paired(device_id, &plugins::request_notifications(packet_id())) }
    pub fn request_players(&self, device_id: &str) -> Result<(), String> { self.send_paired(device_id, &plugins::request_players(packet_id())) }
    pub fn media(&self, device_id: &str, player: &str, action: Action) -> Result<(), String> { self.send_paired(device_id, &plugins::media_action(player, action, packet_id())) }

    /// Offers the file at `path` to the device; its bytes go when it connects to the
    /// payload port, in the background.
    pub fn send_file(&self, device_id: &str, path: &Path) -> Result<(), String> {
        if !self.shared.paired(device_id) { return Err("not paired".into()); }
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        let first = self.shared.config.payload_port;
        let (listener, port) = (first..=first.saturating_add(25)).find_map(|p| TcpListener::bind((Ipv4Addr::UNSPECIFIED, p)).ok().map(|l| (l, p)))
            .ok_or("no free payload port")?;
        self.shared.send(device_id, &plugins::share_file(&name, data.len() as u64, port, packet_id()))?;
        let me = self.clone();
        let device_id = device_id.to_string();
        std::thread::spawn(move || {
            let fail = |reason: String| me.shared.emit(Event::SendFailed { device_id: device_id.clone(), name: name.clone(), reason });
            let _ = listener.set_nonblocking(true);
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut s = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                    Err(e) => return fail(format!("nobody fetched it: {e}")),
                }
            };
            let _ = s.set_nonblocking(false);
            if let Err(e) = s.write_all(&data) { return fail(format!("payload write: {e}")); }
            let _ = s.flush();
            let _ = s.shutdown(Shutdown::Write);
            // Wait for the peer to close, so every byte is taken before ours goes.
            let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
            let mut sink = [0u8; 64];
            while matches!(s.read(&mut sink), Ok(n) if n > 0) {}
            me.shared.emit(Event::Sent { device_id: device_id.clone(), name: name.clone(), bytes: data.len() as u64 });
        });
        Ok(())
    }
}

/// `name`, or `name (2)`, ... whichever is not taken in `dir`.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if std::fs::metadata(&first).is_err() { return first; }
    let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name, "") };
    (2..1000).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| std::fs::metadata(p).is_err()).unwrap_or(first)
}
