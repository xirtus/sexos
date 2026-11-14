// SPDX-License-Identifier: GPL-3.0-only
//! The window around sex_kdeconnect::net::Daemon.
use crate::draw::{clip, Canvas, Face};
use sex_kdeconnect::net::{Config, Daemon, Event as Net};
use sex_kdeconnect::plugins::{Action, Media, NowPlaying};
use sex_kdeconnect::{device_id_from, valid_device_id, Identity};
use sex_std::sex_app::{self, Event, Nav};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const BACK: u32 = 0x0017_191E;
const PANEL: u32 = 0x0022_252C;
const SELECT: u32 = 0x002C_3A55;
const ACCENT: u32 = 0x00E8_6A9A;
const TEXT: u32 = 0x00E8_EAED;
const DIM: u32 = 0x0080_868E;
const WARN: u32 = 0x00E8_B03A;
const GOOD: u32 = 0x006F_CF8F;

#[derive(Clone)]
struct Device { identity: Identity, paired: bool, asking: bool, linked: bool }

struct App {
    daemon: Daemon,
    devices: Vec<Device>,
    picked: usize,
    notifications: Vec<(String, String, String)>,
    clipboard: Option<String>,
    playing: Option<NowPlaying>,
    transfers: Vec<String>,
    offers: Vec<PathBuf>,
    status: String,
    face: Face,
    small: Face,
}

/// FNV-1a over a file's bytes: the gate compares it with the bytes it sent.
fn fnv(data: &[u8]) -> u64 { data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)) }

fn record(line: String) { sex_std::record(&line); }

fn device_id() -> String {
    if let Ok(id) = std::fs::read_to_string("device-id") {
        let id = id.trim().to_string();
        if valid_device_id(&id) { return id; }
    }
    // The clock's nanoseconds and this run's ids, mixed: unique enough for a name.
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let (target, task) = sex_std::identity();
    let a = fnv(&t.to_le_bytes()) ^ (target as u64) << 32 ^ task as u64;
    let b = fnv(&a.to_le_bytes()).rotate_left(17) ^ fnv(&(t >> 7).to_be_bytes());
    let mut seed = [0u8; 16];
    seed[..8].copy_from_slice(&a.to_le_bytes());
    seed[8..].copy_from_slice(&b.to_le_bytes());
    let id = device_id_from(&seed);
    let _ = std::fs::write("device-id", &id);
    id
}

impl App {
    fn current(&self) -> Option<&Device> { self.devices.get(self.picked) }
    fn device_mut(&mut self, id: &str) -> Option<&mut Device> { self.devices.iter_mut().find(|d| d.identity.device_id == id) }
    fn name_of(&self, id: &str) -> String { self.devices.iter().find(|d| d.identity.device_id == id).map(|d| d.identity.device_name.clone()).unwrap_or_else(|| id.into()) }

    fn net(&mut self, e: Net) {
        match e {
            Net::Listening { port } => { record(format!("[hookup.listening] port={port}")); }
            Net::Announced { to, ok } => record(format!("[hookup.announced] to={to} ok={}", ok as u8)),
            Net::Linked { device, addr, incoming, encrypted, paired } => {
                record(format!("[hookup.linked] device={} name={} type={} protocol={} incoming={} encrypted={} paired={} from={}",
                    device.device_id, device.device_name, device.device_type, device.protocol_version, incoming as u8, encrypted as u8, paired as u8, addr.ip()));
                self.status = format!("{} is linked.", device.device_name);
                let d = Device { identity: device, paired, asking: false, linked: true };
                match self.devices.iter_mut().find(|x| x.identity.device_id == d.identity.device_id) { Some(x) => *x = d, None => self.devices.push(d) }
            }
            Net::Unlinked { device_id } => {
                record(format!("[hookup.unlinked] device={device_id}"));
                if let Some(d) = self.device_mut(&device_id) { d.linked = false; d.asking = false; }
            }
            Net::PairAsked { device_id, name } => {
                record(format!("[hookup.pair.asked] device={device_id} name={name}"));
                self.status = format!("{name} asks to pair: y to accept, n to refuse.");
                if let Some(i) = self.devices.iter().position(|d| d.identity.device_id == device_id) { self.picked = i; self.devices[i].asking = true; }
            }
            Net::Paired { device_id, name } => {
                record(format!("[hookup.paired] device={device_id} name={name}"));
                self.status = format!("Paired with {name}.");
                if let Some(d) = self.device_mut(&device_id) { d.paired = true; d.asking = false; }
            }
            Net::Unpaired { device_id, reason } => {
                record(format!("[hookup.unpaired] device={device_id} reason={reason}"));
                self.status = format!("Not paired with {}: {reason}.", self.name_of(&device_id));
                if let Some(d) = self.device_mut(&device_id) { d.paired = false; d.asking = false; }
            }
            Net::Notification { notification: n, .. } => {
                record(format!("[hookup.notification] app={} title={} text={} cancel={}", n.app, n.title, n.text.replace('\n', " "), n.cancel as u8));
                if n.cancel { return; }
                self.notifications.insert(0, (n.app, n.title, n.text));
                self.notifications.truncate(6);
            }
            Net::Clipboard { text, .. } => {
                record(format!("[hookup.clipboard] bytes={} text={}", text.len(), text.replace('\n', " ")));
                self.clipboard = Some(text);
            }
            Net::Media { media: Media::Players(list), .. } => {
                record(format!("[hookup.media.players] {}", list.join(",")));
                if let Some(first) = list.first() {
                    if self.playing.as_ref().is_none_or(|p| !list.contains(&p.player)) {
                        self.playing = Some(NowPlaying { player: first.clone(), ..Default::default() });
                    }
                }
            }
            Net::Media { media: Media::Playing(now), .. } => {
                record(format!("[hookup.media] player={} title={} artist={} playing={}", now.player, now.title, now.artist, now.playing as u8));
                self.playing = Some(now);
            }
            Net::Ping { device_id } => { record(format!("[hookup.ping] device={device_id}")); self.status = format!("{} pinged.", self.name_of(&device_id)); }
            Net::SharedText { text, .. } => { record(format!("[hookup.shared.text] {text}")); self.status = format!("Shared: {text}"); }
            Net::Received { name, path, bytes, .. } => {
                // Read back from the folder: what the gate checks is what is on disk.
                let data = std::fs::read(&path).unwrap_or_default();
                record(format!("[hookup.received] name={name} bytes={bytes} disk={} fnv={:016x}", data.len(), fnv(&data)));
                self.transfers.insert(0, format!("Received {name} ({bytes} bytes)"));
                self.status = format!("Saved {name} in received/.");
            }
            Net::ReceiveFailed { name, reason, .. } => { record(format!("[hookup.receive.failed] name={name} reason={reason}")); self.transfers.insert(0, format!("{name}: {reason}")); }
            Net::Sent { name, bytes, .. } => { record(format!("[hookup.sent] name={name} bytes={bytes}")); self.transfers.insert(0, format!("Sent {name} ({bytes} bytes)")); }
            Net::SendFailed { name, reason, .. } => { record(format!("[hookup.send.failed] name={name} reason={reason}")); self.transfers.insert(0, format!("{name}: {reason}")); }
            Net::Ignored { device_id, kind } => record(format!("[hookup.ignored] device={device_id} type={kind}")),
            Net::Problem(p) => { record(format!("[hookup.problem] {p}")); self.status = p; }
        }
        self.transfers.truncate(5);
    }

    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        let Some(dev) = self.current().map(|d| d.identity.device_id.clone()) else {
            if sex_app::typed(key) == Some(b'r') { self.daemon.announce(); }
            return;
        };
        let say = |r: Result<(), String>, what: &str| -> String { match r { Ok(()) => what.to_string(), Err(e) => format!("Not sent: {e}.") } };
        let player = self.playing.as_ref().map(|p| p.player.clone()).unwrap_or_default();
        match (sex_app::nav(key), sex_app::typed(key)) {
            (Some(Nav::Up), _) => self.picked = self.picked.saturating_sub(1),
            (Some(Nav::Down), _) => self.picked = (self.picked + 1).min(self.devices.len().saturating_sub(1)),
            (Some(Nav::Left), _) => self.status = say(self.daemon.media(&dev, &player, Action::Previous), "Previous track."),
            (Some(Nav::Right), _) => self.status = say(self.daemon.media(&dev, &player, Action::Next), "Next track."),
            (_, Some(b' ')) => { record(format!("[hookup.media.sent] action=PlayPause player={player}")); self.status = say(self.daemon.media(&dev, &player, Action::PlayPause), "Play/pause."); }
            (_, Some(b'p')) => { record(format!("[hookup.pair.request] device={dev}")); self.daemon.pair(&dev); self.status = "Asked to pair; accept on the phone.".into(); }
            (_, Some(b'y')) => { record(format!("[hookup.pair.accept] device={dev}")); self.daemon.accept(&dev); }
            (_, Some(b'n')) => { record(format!("[hookup.pair.reject] device={dev}")); self.daemon.reject(&dev); }
            (_, Some(b'u')) => self.daemon.unpair(&dev),
            (_, Some(b'r')) => self.daemon.announce(),
            (_, Some(b'c')) => {
                let text = self.clipboard.clone().unwrap_or_default();
                self.status = say(self.daemon.send_clipboard(&dev, &text), "Clipboard sent.");
            }
            (_, Some(b's')) => {
                if self.offers.is_empty() { self.status = "No files given: run hookup @FILE.".into(); }
                for path in self.offers.clone() {
                    let r = self.daemon.send_file(&dev, &path);
                    record(format!("[hookup.offer] path={} ok={}", path.display(), r.is_ok() as u8));
                    self.status = say(r, "Offered; the device fetches it.");
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, window: &mut sex_app::Window) {
        let (w, h) = window.size();
        let px = window.pixels();
        let (fw, fh) = (self.face.w as i32, self.face.h as i32);
        let sh = self.small.h as i32;
        let mut c = Canvas { px: &mut px[..w * h], w, h };
        c.fill(0, 0, w as i32, h as i32, BACK);
        let me = self.daemon.identity();
        c.text(&mut self.face, 12, 8, "Hookup", ACCENT, w as i32);
        c.text(&mut self.small, 12 + 7 * fw, 8 + (fh - sh), &format!("{} - port {}", me.device_name, me.tcp_port.unwrap_or(0)), DIM, w as i32 - 24);
        c.text(&mut self.small, 12, 10 + fh, "Links are not encrypted yet: pair only on a network you trust.", WARN, w as i32 - 24);
        let mut y = 16 + fh + sh;
        let col = (w as i32 / 2).max(240);
        // Devices.
        c.text(&mut self.small, 12, y, "Devices", DIM, col);
        y += sh + 4;
        if self.devices.is_empty() { c.text(&mut self.small, 12, y, "Looking for phones running KDE Connect...", TEXT, col - 24); y += sh + 4; }
        for (i, d) in self.devices.iter().enumerate() {
            if i == self.picked { c.fill(8, y - 2, col - 16, fh + 4, SELECT); }
            let (state, color) = if d.asking { ("asks to pair", WARN) } else if d.paired { ("paired", GOOD) } else { ("not paired", DIM) };
            let state = if d.linked { state.to_string() } else { format!("{state}, away") };
            let next = c.text(&mut self.face, 14, y, clip(&d.identity.device_name, 24), TEXT, col - 30);
            c.text(&mut self.small, next + 10, y + (fh - sh), &format!("{} - {state}", d.identity.device_type), color, col - next);
            y += fh + 6;
        }
        // Right column: notifications, clipboard, media, transfers.
        let x = col + 8;
        let mut ry = 16 + fh + sh;
        let rw = w as i32 - x - 12;
        c.text(&mut self.small, x, ry, "Notifications", DIM, rw);
        ry += sh + 4;
        for (app, title, text) in &self.notifications {
            c.fill(x, ry - 2, rw, 2 * sh + 6, PANEL);
            c.text(&mut self.small, x + 6, ry, &format!("{app}: {title}"), TEXT, rw - 12);
            c.text(&mut self.small, x + 6, ry + sh + 2, text, DIM, rw - 12);
            ry += 2 * sh + 10;
        }
        ry += 6;
        c.text(&mut self.small, x, ry, "Clipboard", DIM, rw);
        ry += sh + 2;
        c.text(&mut self.small, x, ry, self.clipboard.as_deref().unwrap_or("-"), TEXT, rw);
        ry += sh + 10;
        c.text(&mut self.small, x, ry, "Media", DIM, rw);
        ry += sh + 2;
        let media = match &self.playing {
            Some(p) => format!("{} {} - {} ({})", if p.playing { ">" } else { "||" }, p.artist, p.title, p.player),
            None => "-".into(),
        };
        c.text(&mut self.small, x, ry, &media, TEXT, rw);
        ry += sh + 10;
        c.text(&mut self.small, x, ry, "Files", DIM, rw);
        ry += sh + 2;
        for t in &self.transfers { c.text(&mut self.small, x, ry, t, TEXT, rw); ry += sh + 2; }
        // Status and keys.
        let by = h as i32 - 2 * sh - 12;
        c.fill(0, by - 4, w as i32, h as i32 - by + 4, PANEL);
        c.text(&mut self.small, 12, by, &self.status, TEXT, w as i32 - 24);
        c.text(&mut self.small, 12, by + sh + 4, "p pair  y/n answer  u unpair  s send files  c clipboard  space play  </> track  r announce", DIM, w as i32 - 24);
        window.show_pixels(w, h);
    }
}

fn drain(app: &mut App, rx: &Receiver<Net>) -> bool {
    let mut any = false;
    while let Ok(e) = rx.try_recv() { app.net(e); any = true; }
    any
}

pub fn main() {
    let mut announce: Vec<SocketAddr> = Vec::new();
    let mut offers = Vec::new();
    for a in std::env::args().skip(1) {
        if a.starts_with('@') { offers.push(PathBuf::from(a)); continue; }
        match a.to_socket_addrs() { Ok(mut it) => announce.extend(it.next()), Err(_) => eprintln!("Not an address: {a}") }
    }
    let name = std::fs::read_to_string("name").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "SexOS".into());
    let config = Config { device_id: device_id(), name, port: sex_kdeconnect::PORT, announce, broadcast: true, folder: PathBuf::from("/"), payload_port: *sex_kdeconnect::PAYLOAD_PORTS.start() };
    let (daemon, rx) = match Daemon::start(config) {
        Ok(x) => x,
        Err(e) => {
            println!("Hookup could not listen on port {}: {e}\n\nStart it with network access: run +net hookup", sex_kdeconnect::PORT);
            sex_std::record(&format!("[hookup.fatal] listen: {e}"));
            std::process::exit(1);
        }
    };
    let (target, task) = sex_std::identity();
    record(format!("[hookup.ready] target={target} task={task} device={} offers={}", daemon.identity().device_id, offers.len()));
    let window = sex_std::window();
    window.title(b"Hookup");
    let mut app = App { daemon, devices: Vec::new(), picked: 0, notifications: Vec::new(), clipboard: None, playing: None, transfers: Vec::new(), offers,
        status: "Waiting for a phone. Open KDE Connect on it, on the same network.".into(), face: Face::new(10, 20), small: Face::new(8, 16) };
    drain(&mut app, &rx);
    app.draw(window);
    let mut last_tick = Instant::now();
    loop {
        let mut redraw = drain(&mut app, &rx);
        match window.next_within(100) {
            Some(Event::Close) => break,
            Some(Event::Resize { .. }) => redraw = true,
            Some(Event::Key(k)) => { app.key(k); redraw = true; }
            Some(Event::Pointer(p)) => {
                if let sex_std::sex_pdx::app_pointer::Kind::Down { button: 0 } = p.kind {
                    let row = (p.y as i32 - (16 + app.face.h as i32 + app.small.h as i32 + app.small.h as i32 + 4)) / (app.face.h as i32 + 6);
                    if row >= 0 && (row as usize) < app.devices.len() { app.picked = row as usize; redraw = true; }
                }
            }
            None => {}
        }
        if last_tick.elapsed().as_millis() > 1000 { app.daemon.tick(); last_tick = Instant::now(); }
        redraw |= drain(&mut app, &rx);
        if redraw { app.draw(window); }
    }
    record("[hookup.closed]".into());
    std::process::exit(0);
}
