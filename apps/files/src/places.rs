// SPDX-License-Identifier: GPL-3.0-only
//! Network places in Files: SFTP servers, WebDAV shares (Nextcloud, ownCloud...) and
//! S3-compatible buckets, browsed like folders through `loom-remote`'s `Place`.
//!
//! Ctrl+Shift+P lists the places, with rows to add an SFTP, WebDAV or S3 place (host,
//! port, user, folder, and a password, secret key or SSH key file). Enter opens one;
//! Enter and Backspace move through its folders; Ctrl+Shift+X copies the selected item
//! down into the folder Files was in; Ctrl+V uploads what Ctrl+C picked; Esc leaves.
//!
//! The network is the person's to give: Files reaches it only in a run started with
//! `run +net files`; otherwise opening a place says so and nothing else happens.
//! Passwords and keys live in memory only, for this run (wiped when forgotten or when
//! Files closes); they are never written down or put in a record.
//!
//! Connections go through the network service's `Duplex` (TCP, and TLS checked by the
//! service for `https`), the same path HTTP and WebSocket clients take. SFTP runs SSH
//! (`sex-ssh`) over it; the server's host key is trusted the first time and must stay the
//! same for the rest of the run.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use loom_remote::{http::Http1, Connect, Kind, Place, Source, Stream};
use sex_app::files::{self, direct, Entry};
use sex_app::net::{self, Duplex};
use sex_app::{Nav, Window};
use sex_archive::{Flow, Progress};
use sha2::{Digest, Sha256};

use crate::archive::{mkdirs, End, Ticker};
use crate::{at, base, join, shown, App, Undo, BACKSPACE, ENTER, ESC};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Proto { Sftp, WebDav, S3 }
impl Proto {
    fn name(self) -> &'static str { match self { Proto::Sftp => "SFTP", Proto::WebDav => "WebDAV", Proto::S3 => "S3" } }
}

/// A secret typed for this run; wiped when dropped.
#[derive(Default)]
struct Secret(String);
impl Drop for Secret {
    fn drop(&mut self) { unsafe { for b in self.0.as_bytes_mut() { core::ptr::write_volatile(b, 0); } } }
}

/// A place the person added.
pub struct Saved {
    proto: Proto,
    host: String,
    port: u16,
    tls: bool,
    user: String,
    /// SFTP: the folder ("" is the login's home). WebDAV: the URL path. S3: bucket/prefix.
    path: String,
    /// S3 region.
    region: String,
    /// SFTP: an OpenSSH private key file on this computer.
    key_file: String,
    secret: Secret,
}
impl Saved {
    fn label(&self) -> String {
        let scheme = match self.proto { Proto::Sftp => "sftp", Proto::WebDav => if self.tls { "davs" } else { "dav" }, Proto::S3 => "s3" };
        let user = if self.user.is_empty() || self.proto == Proto::S3 { String::new() } else { format!("{}@", self.user) };
        format!("{}://{}{}:{}/{}", scheme, user, self.host, self.port, self.path.trim_start_matches('/'))
    }
}

struct Field { label: &'static str, value: String, secret: bool }
pub struct Form { proto: Proto, fields: Vec<Field>, at: usize }
impl Form {
    fn new(proto: Proto) -> Form {
        let f = |label, value: &str, secret| Field { label, value: value.into(), secret };
        let fields = match proto {
            Proto::Sftp => vec![f("Host", "", false), f("Port", "22", false), f("User", "", false), f("Folder (empty: home)", "", false),
                f("SSH key file on this computer (empty: none)", "", false), f("Password (empty: none)", "", true)],
            Proto::WebDav => vec![f("Host (http:// before it for plain HTTP)", "", false), f("Port", "443", false), f("User", "", false),
                f("Path (e.g. /remote.php/dav/files/NAME)", "/", false), f("Password", "", true)],
            Proto::S3 => vec![f("Endpoint host (http:// before it for plain HTTP)", "s3.amazonaws.com", false), f("Port", "443", false),
                f("Access key", "", false), f("Bucket/prefix", "", false), f("Region", "us-east-1", false), f("Secret key", "", true)],
        };
        Form { proto, fields, at: 0 }
    }
    fn into_saved(self) -> Result<Saved, String> {
        let v: Vec<String> = self.fields.into_iter().map(|f| f.value).collect();
        let (mut host, tls) = match v[0].strip_prefix("http://") { Some(h) => (h.to_string(), false), None => (v[0].strip_prefix("https://").unwrap_or(&v[0]).to_string(), true) };
        host = host.trim_end_matches('/').into();
        if host.is_empty() { return Err("a host is needed".into()); }
        let port: u16 = v[1].trim().parse().map_err(|_| String::from("the port is a number"))?;
        let mut s = Saved { proto: self.proto, host, port, tls: tls && self.proto != Proto::Sftp, user: v[2].clone(), path: v[3].clone(), region: String::new(), key_file: String::new(), secret: Secret::default() };
        match self.proto {
            Proto::Sftp => { s.key_file = v[4].clone(); s.secret.0 = v[5].clone(); }
            Proto::WebDav => s.secret.0 = v[4].clone(),
            Proto::S3 => { s.region = v[4].clone(); s.secret.0 = v[5].clone(); if s.path.trim_matches('/').is_empty() { return Err("a bucket is needed".into()); } }
        }
        Ok(s)
    }
}

pub enum View {
    /// The places, and rows to add one.
    List,
    /// Inside place `place`, at folder `dir` ("" is its top).
    Browse { place: usize, conn: Box<dyn Place>, dir: String },
}

#[derive(Default)]
pub struct Places {
    saved: Vec<Saved>,
    pub view: Option<View>,
    pub form: Option<Form>,
    /// The folder Files was in: copies come down here.
    home: Vec<u8>,
    /// SSH host keys seen this run: "host:port" and SHA256 fingerprint.
    known: Rc<RefCell<Vec<(String, String)>>>,
}
impl Places {
    pub fn active(&self) -> bool { self.view.is_some() || self.form.is_some() }
}

const ADD_ROWS: [(&str, Proto); 3] = [("+ Add SFTP place", Proto::Sftp), ("+ Add WebDAV place", Proto::WebDav), ("+ Add S3 place", Proto::S3)];

// --- The network, as loom-remote's byte streams. ---

/// The window the network calls queue keys for while they wait. Set by every Places
/// operation before it touches a connection; connections are used only inside those.
static mut WINDOW: *mut Window = core::ptr::null_mut();
fn window<'a>() -> &'a mut Window { unsafe { &mut *WINDOW } }

fn net_error(e: net::Error) -> loom_remote::Error { loom_remote::Error::Transport(String::from(shown(net::message(e)))) }

/// One connection through the network service (TLS when `tls`).
struct NetStream { d: Duplex, got: Vec<u8>, at: usize }
impl NetStream {
    fn open(host: &str, port: u16, tls: bool) -> loom_remote::Result<NetStream> {
        let w = window();
        let mut d = Duplex::start(w, host, port, tls, Vec::new()).map_err(net_error)?;
        let mut got = Vec::new();
        while !d.ready() { if !d.poll(w, &mut got).map_err(net_error)? { sex_pdx::sys_yield(); } }
        Ok(NetStream { d, got, at: 0 })
    }
}
impl Stream for NetStream {
    fn write_all(&mut self, bytes: &[u8]) -> loom_remote::Result<()> { self.d.send(window(), bytes).map_err(net_error) }
    fn read(&mut self, buf: &mut [u8]) -> loom_remote::Result<usize> {
        let began = sex_pdx::get_ticks();
        let mut quiet = 0u32;
        while self.at >= self.got.len() {
            self.got.clear();
            self.at = 0;
            match self.d.poll(window(), &mut self.got) {
                Ok(true) => quiet = 0,
                Ok(false) => {
                    quiet += 1;
                    if sex_pdx::get_ticks().saturating_sub(began) > 60_000 { return Err(loom_remote::Error::Transport("the server stopped answering".into())); }
                    if quiet > 64 { let until = sex_pdx::get_ticks() + 2; while sex_pdx::get_ticks() < until { sex_pdx::sys_yield(); } } else { sex_pdx::sys_yield(); }
                }
                // The other end closed (or the connection went).
                Err(net::Error::Failed) => return Ok(0),
                Err(e) => return Err(net_error(e)),
            }
        }
        let n = (self.got.len() - self.at).min(buf.len());
        buf[..n].copy_from_slice(&self.got[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}
impl Drop for NetStream {
    fn drop(&mut self) { if unsafe { !WINDOW.is_null() } { self.d.close(window()); } }
}
struct NetConnect { host: String, port: u16, tls: bool }
impl Connect for NetConnect {
    fn connect(&mut self) -> loom_remote::Result<Box<dyn Stream>> { Ok(Box::new(NetStream::open(&self.host, self.port, self.tls)?)) }
}

/// Key material for SSH: the CPU's hardware generator, hashed with a counter.
fn random() -> Option<Box<dyn FnMut(&mut [u8])>> {
    if core::arch::x86_64::__cpuid(1).ecx & (1 << 30) == 0 { return None; }
    fn rdrand(out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let mut v = 0u64;
            for _ in 0..10 { if unsafe { core::arch::x86_64::_rdrand64_step(&mut v) } == 1 { break; } }
            chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
        }
    }
    let mut pool = [0u8; 32];
    rdrand(&mut pool);
    let mut n = 0u64;
    Some(Box::new(move |out: &mut [u8]| {
        for chunk in out.chunks_mut(32) {
            let mut fresh = [0u8; 32];
            rdrand(&mut fresh);
            n += 1;
            let block: [u8; 32] = Sha256::new().chain_update(pool).chain_update(fresh).chain_update(n.to_le_bytes()).finalize().into();
            chunk.copy_from_slice(&block[..chunk.len()]);
            pool = Sha256::digest(block).into();
        }
    }))
}

fn remote_err(e: loom_remote::Error) -> String {
    use loom_remote::Error as E;
    match e {
        E::NotFound => "not found".into(), E::Exists => "it already exists".into(), E::Denied => "the server said no (check the user, password or key)".into(),
        E::NotEmpty => "the folder isn't empty".into(), E::BadPath => "that path can't be used".into(), E::Unsupported => "the server can't do that".into(),
        E::Transport(m) => m, E::Protocol(m) => format!("the server broke the protocol ({})", m), E::Remote(code, m) => format!("the server refused ({} {})", code, m),
        E::Source => "the local file couldn't be read".into(),
    }
}
fn rjoin(dir: &str, name: &str) -> String { if dir.is_empty() { name.into() } else { format!("{}/{}", dir, name) } }

/// A local file uploaded a piece at a time, telling the progress line as it goes.
struct Upload<'a> { path: Vec<u8>, off: u64, size: u64, tick: &'a mut dyn FnMut(u64) -> bool }
impl Source for Upload<'_> {
    fn read(&mut self, buf: &mut [u8]) -> loom_remote::Result<usize> {
        if self.off >= self.size { return Ok(0); }
        let want = ((self.size - self.off) as usize).min(buf.len());
        let n = direct::read_at(&at(&self.path), self.off, &mut buf[..want]).map_err(|_| loom_remote::Error::Source)?;
        if n == 0 { return Err(loom_remote::Error::Source); }
        self.off += n as u64;
        if !(self.tick)(n as u64) { return Err(loom_remote::Error::Transport("stopped".into())); }
        Ok(n)
    }
}

impl App {
    /// Ctrl+Shift+P: the list of places.
    pub(crate) fn places_open(&mut self, window: &mut Window) {
        let _ = window;
        if !matches!(self.loom.places.view, Some(View::Browse { .. })) { self.loom.places.home = self.cwd.clone(); }
        self.loom.places.view = Some(View::List);
        self.status = "Places: Enter opens one or adds one, Del forgets one, Esc goes back.".into();
        self.places_list();
    }
    fn places_list(&mut self) {
        let p = &self.loom.places;
        let (cwd, list): (String, Vec<Entry>) = match &p.view {
            Some(View::Browse { place, conn: _, dir }) => {
                let label = p.saved[*place].label();
                let mut cwd = label.clone();
                if !dir.is_empty() { cwd = format!("{}/{}", label.trim_end_matches('/'), dir); }
                let place = *place;
                let dir = dir.clone();
                let Some(View::Browse { conn, .. }) = self.loom.places.view.as_mut() else { return };
                set_window_none();
                let r = conn.list(&dir);
                match r {
                    Ok(items) => {
                        let mut v: Vec<Entry> = items.into_iter().map(|e| Entry { dir: e.kind == Kind::Dir, size: e.size, name: e.name.into_bytes() }).collect();
                        v.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
                        sex_pdx::serial_record!("[files.app.places.list] place={} dir={} count={}", place, if dir.is_empty() { "/" } else { &dir }, v.len());
                        (cwd, v)
                    }
                    Err(e) => { self.status = format!("Can't list this folder: {}.", remote_err(e)); (cwd, Vec::new()) }
                }
            }
            _ => {
                let mut v: Vec<Entry> = ADD_ROWS.iter().map(|(l, _)| Entry { dir: false, size: 0, name: l.as_bytes().to_vec() }).collect();
                v.extend(p.saved.iter().map(|s| Entry { dir: true, size: 0, name: s.label().into_bytes() }));
                sex_pdx::serial_record!("[files.app.places.list] place=- dir=/ count={}", v.len());
                (String::from("Places"), v)
            }
        };
        self.entries = list;
        self.cwd = cwd.into_bytes();
        self.found = None;
        self.origins.clear();
        self.cursor = 0;
        self.top = 0;
    }
    fn places_leave(&mut self, window: &mut Window) {
        set_window(window);
        self.loom.places.view = None;
        self.loom.places.form = None;
        set_window_none();
        sex_pdx::serial_record!("[files.app.places.left]");
        let home = core::mem::take(&mut self.loom.places.home);
        self.go(window, if home.is_empty() { b"/".to_vec() } else { home });
    }
    fn form_status(&mut self) {
        let Some(f) = &self.loom.places.form else { return };
        let field = &f.fields[f.at];
        let shown_text = if field.secret { "*".repeat(field.value.chars().count()) } else { field.value.clone() };
        self.status = format!("New {} place ({} of {}). {}: {}|   (Enter next, Esc cancels)", f.proto.name(), f.at + 1, f.fields.len(), field.label, shown_text);
    }

    /// Keys while Places is open. False lets Files move the cursor.
    pub(crate) fn places_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) -> bool {
        if let Some(f) = self.loom.places.form.as_mut() {
            let field = &mut f.fields[f.at];
            match key.code {
                ESC => { self.loom.places.form = None; self.status = "No place added.".into(); return true; }
                ENTER => {
                    f.at += 1;
                    if f.at == f.fields.len() {
                        let form = self.loom.places.form.take().unwrap();
                        match form.into_saved() {
                            Ok(s) => {
                                sex_pdx::serial_record!("[files.app.places.added] proto={} host={} port={} tls={} path={}", s.proto.name(), s.host, s.port, s.tls as u8, s.path);
                                self.loom.places.saved.push(s);
                                let i = self.loom.places.saved.len() - 1;
                                self.places_connect(window, i);
                            }
                            Err(why) => self.status = format!("No place added: {}.", why),
                        }
                        return true;
                    }
                }
                BACKSPACE => { field.value.pop(); }
                _ => if let Some(c) = sex_app::text(key) { if field.value.len() < 255 { field.value.push(c); } },
            }
            self.form_status();
            return true;
        }
        if matches!(sex_app::nav(key), Some(Nav::Up | Nav::Down | Nav::Home | Nav::End | Nav::Left | Nav::Right)) { return false; }
        let browsing = matches!(self.loom.places.view, Some(View::Browse { .. }));
        if key.control && key.shift && key.code == crate::loom::KEY_X && browsing { self.places_download(window); return true; }
        if key.control && !key.shift && key.code == crate::KEY_V && browsing { self.places_upload(window); return true; }
        if key.control && !key.shift && matches!(key.code, crate::KEY_H | crate::KEY_D | crate::KEY_T | crate::KEY_G) { self.places_leave(window); return true; }
        let Some(e) = self.entries.get(self.cursor) else {
            if key.code == ESC || key.code == BACKSPACE { self.places_leave(window); }
            return true;
        };
        let (name, dir) = (String::from(shown(&e.name)), e.dir);
        match (key.code, browsing) {
            (ESC, _) => self.places_leave(window),
            (BACKSPACE, false) => self.places_leave(window),
            (BACKSPACE, true) => {
                let Some(View::Browse { dir, .. }) = self.loom.places.view.as_mut() else { return true };
                if dir.is_empty() { set_window(window); self.loom.places.view = Some(View::List); set_window_none(); self.places_list(); return true; }
                let here = match dir.rfind('/') { Some(i) => { let h = dir[i + 1..].to_string(); dir.truncate(i); h } None => core::mem::take(dir) };
                set_window(window);
                self.places_list();
                self.select(here.as_bytes());
            }
            (ENTER, false) => match ADD_ROWS.iter().find(|(l, _)| *l == name) {
                Some((_, proto)) => { self.loom.places.form = Some(Form::new(*proto)); self.form_status(); }
                None => if let Some(i) = self.loom.places.saved.iter().position(|s| s.label() == name) { self.places_connect(window, i); },
            },
            (ENTER, true) => {
                if !dir { self.status = format!("{}: {} on the server. Ctrl+Shift+X copies it down.", name, crate::human(e.size)); return true; }
                let Some(View::Browse { dir, .. }) = self.loom.places.view.as_mut() else { return true };
                *dir = rjoin(dir, &name);
                set_window(window);
                self.places_list();
            }
            _ if !browsing && matches!(sex_app::nav(key), Some(Nav::Delete)) => {
                if let Some(i) = self.loom.places.saved.iter().position(|s| s.label() == name) {
                    self.loom.places.saved.remove(i);
                    sex_pdx::serial_record!("[files.app.places.forgotten]");
                    self.places_list();
                    self.status = "Forgot that place and its password.".into();
                }
            }
            _ => self.status = if browsing { "On a network place: Ctrl+Shift+X copies the selected item down, Ctrl+V uploads what Ctrl+C picked, Backspace goes up, Esc leaves.".into() }
                else { "Places: Enter opens one or adds one, Del forgets one, Esc goes back.".into() },
        }
        true
    }

    fn places_connect(&mut self, window: &mut Window, i: usize) {
        let label = self.loom.places.saved[i].label();
        self.status = format!("Connecting to {}...", label);
        self.render(window);
        set_window(window);
        let r = self.connect(window, i);
        match r {
            Ok(conn) => {
                sex_pdx::serial_record!("[files.app.places.connected] place={} proto={}", i, self.loom.places.saved[i].proto.name());
                self.loom.places.view = Some(View::Browse { place: i, conn, dir: String::new() });
                self.status = format!("{}: Ctrl+Shift+X copies down, Ctrl+V uploads, Backspace goes up, Esc leaves.", label);
                self.places_list();
            }
            Err(why) => {
                sex_pdx::serial_record!("[files.app.places.failed] kind=connect place={} why={}", i, why);
                self.status = format!("Can't open {}: {}.", label, why);
                self.loom.places.view = Some(View::List);
            }
        }
        set_window_none();
    }
    fn connect(&mut self, window: &mut Window, i: usize) -> Result<Box<dyn Place>, String> {
        let s = &self.loom.places.saved[i];
        // The network is given at launch (`run +net files`); ask the service first so the
        // answer is plain.
        if let Err(e) = net::direct::up() { return Err(String::from(shown(net::message(e))).replace("This program was not given the network. Start it with run +net.", "Files was started without the network: start it with run +net files")); }
        let authority = if (s.tls && s.port == 443) || (!s.tls && s.port == 80) { s.host.clone() } else { format!("{}:{}", s.host, s.port) };
        match s.proto {
            Proto::Sftp => {
                let rng = random().ok_or("this computer has no hardware random generator to make SSH keys with")?;
                let mut auth = Vec::new();
                if !s.key_file.is_empty() {
                    let mut buf = vec![0u8; 16 * 1024];
                    let n = files::read(window, &at(s.key_file.as_bytes()), &mut buf).map_err(|e| format!("can't read the key file: {}", shown(files::message(e))))?;
                    let text = core::str::from_utf8(&buf[..n]).map_err(|_| "the key file isn't text")?;
                    let key = sex_ssh::keys::Ed25519::from_openssh(text).map_err(|_| "the key file isn't an unencrypted OpenSSH ed25519 key")?;
                    for b in buf.iter_mut() { *b = 0; }
                    auth.push(sex_ssh::Auth::Ed25519(key));
                }
                if !s.secret.0.is_empty() { auth.push(sex_ssh::Auth::Password(s.secret.0.clone())); }
                let conn = NetStream::open(&s.host, s.port, false).map_err(remote_err)?;
                let client = sex_ssh::Client::new(&s.user, auth, Some(sex_ssh::Run::Subsystem("sftp".into())), rng);
                let (known, who) = (self.loom.places.known.clone(), format!("{}:{}", s.host, s.port));
                let trust = Box::new(move |alg: &str, _blob: &[u8], fp: &str| {
                    let mut k = known.borrow_mut();
                    match k.iter().find(|(h, _)| *h == who) {
                        Some((_, seen)) => seen == fp,
                        None => { sex_pdx::serial_record!("[files.app.places.hostkey] host={} algorithm={} fingerprint={} first=1", who, alg, fp); k.push((who.clone(), fp.into())); true }
                    }
                });
                let chan = loom_remote::sftp::SshChannel::open(client, conn, trust).map_err(|e| match e { loom_remote::Error::Denied => "the server refused the login, or its host key changed".into(), e => remote_err(e) })?;
                Ok(Box::new(loom_remote::sftp::Sftp::new(chan, &s.path).map_err(remote_err)?))
            }
            Proto::WebDav => {
                let http = Http1::new(NetConnect { host: s.host.clone(), port: s.port, tls: s.tls }, &authority);
                let auth = if s.user.is_empty() { loom_remote::webdav::Auth::None } else { loom_remote::webdav::Auth::Basic { user: s.user.clone(), password: s.secret.0.clone() } };
                let mut dav = loom_remote::webdav::WebDav::new(http, if s.tls { "https" } else { "http" }, &s.path, auth);
                dav.stat("").map_err(remote_err)?;
                Ok(Box::new(dav))
            }
            Proto::S3 => {
                let http = Http1::new(NetConnect { host: s.host.clone(), port: s.port, tls: s.tls }, &authority);
                let path = s.path.trim_matches('/');
                let (bucket, prefix) = match path.split_once('/') { Some((b, p)) => (b, p), None => (path, "") };
                let mut cfg = loom_remote::s3::Config::new(bucket, &s.region);
                if !prefix.is_empty() { cfg.prefix = format!("{}/", prefix.trim_end_matches('/')); }
                let creds = loom_remote::sigv4::Credentials::new(&s.user, &s.secret.0);
                let clock = Box::new(|| sex_pdx::civil_time_ms().map(|ms| ms / 1000).unwrap_or(0));
                let mut s3 = loom_remote::s3::S3::new(http, cfg, creds, clock);
                s3.list("").map_err(remote_err)?;
                Ok(Box::new(s3))
            }
        }
    }

    /// Ctrl+Shift+X on a place: the selected file or folder comes down into the folder
    /// Files was in, under a hidden `.NAME.part` until whole.
    fn places_download(&mut self, window: &mut Window) {
        let Some(e) = self.entries.get(self.cursor) else { return };
        let name = String::from(shown(&e.name));
        let Some(View::Browse { place, mut conn, dir }) = self.loom.places.view.take() else { return };
        let wp: *mut Window = window;
        set_window(window);
        let top = rjoin(&dir, &name);
        // Everything to fetch: (remote path, relative path, folder, size).
        let mut items: Vec<(String, String, bool, u64)> = Vec::new();
        let mut todo = vec![(top.clone(), name.clone())];
        let mut failed = None;
        while let Some((path, rel)) = todo.pop() {
            match conn.stat(&path) {
                Ok(st) if st.kind == Kind::Dir => {
                    items.push((path.clone(), rel.clone(), true, 0));
                    match conn.list(&path) { Ok(kids) => for k in kids { todo.push((rjoin(&path, &k.name), format!("{}/{}", rel, k.name))); }, Err(e) => { failed = Some(remote_err(e)); break; } }
                }
                Ok(st) => items.push((path, rel, false, st.size)),
                Err(e) => { failed = Some(remote_err(e)); break; }
            }
            if items.len() > 100_000 { failed = Some("too many files".into()); break; }
        }
        items.sort_by(|a, b| a.1.cmp(&b.1));
        let home = self.loom.places.home.clone();
        let to = self.free_name(unsafe { &mut *wp }, &home, name.as_bytes(), "copy");
        let mut part_name = b".".to_vec();
        part_name.extend_from_slice(base(&to));
        part_name.extend_from_slice(b".part");
        let part = join(&home, &part_name);
        let _ = self.remove_all(unsafe { &mut *wp }, &part);
        let total: u64 = items.iter().map(|i| i.3).sum();
        sex_pdx::serial_record!("[files.app.places.download.start] place={} member={} to={} items={} bytes={}", place, top, shown(&to), items.len(), total);
        let mut ticker = Ticker::new("Downloading", base(&to));
        let (mut done, mut files_n) = (0u64, 0u64);
        let mut made = BTreeSet::new();
        let mut end = match failed { Some(why) => End::Failed(why), None => End::Done };
        if let End::Done = end {
            'all: for (n, (path, rel, is_dir, size)) in items.iter().enumerate() {
                let sub = if n == 0 { None } else { Some(&rel[name.len() + 1..]) };
                let dest = match sub { None => part.clone(), Some(s) => join(&part, s.as_bytes()) };
                if *is_dir {
                    let r = match sub { None => direct::mkdir(&at(&dest)).map_err(|_| ()), Some(s) => mkdirs(&part, s, &mut made).map_err(|_| ()) };
                    if r.is_err() { end = End::Failed("couldn't make a folder".into()); break; }
                    continue;
                }
                if direct::create(&at(&dest)).is_err() && direct::truncate(&at(&dest), 0).is_err() { end = End::Failed("couldn't make a file".into()); break; }
                files_n += 1;
                let mut off = 0u64;
                while off < *size {
                    let want = (*size - off).min(256 * 1024);
                    let data = match conn.read(path, off, want) { Ok(d) if !d.is_empty() => d, Ok(_) => { end = End::Failed(format!("{} ended early", rel)); break 'all; } Err(e) => { end = End::Failed(remote_err(e)); break 'all; } };
                    match direct::write_at(&at(&dest), off, &data) { Ok(w) if w == data.len() => {} _ => { end = End::Failed("couldn't write (is the disk full?)".into()); break 'all; } }
                    off += data.len() as u64;
                    done += data.len() as u64;
                    if let Flow::Cancel = ticker.tick(self, unsafe { &mut *wp }, Progress { done, total }) { end = End::Stopped; break 'all; }
                }
            }
        }
        let window = unsafe { &mut *wp };
        if let End::Done = end { if let Err(e) = files::rename(window, &at(&part), &at(&to)) { end = End::Failed(shown(files::message(e)).into()); } }
        let _ = files::sync(window);
        match end {
            End::Done => {
                sex_pdx::serial_record!("[files.app.places.downloaded] place={} member={} to={} files={} bytes={}", place, top, shown(&to), files_n, done);
                self.status = format!("Copied down {} ({} files, {}) into {}. Ctrl+Z takes it back.", name, files_n, crate::human(done), shown(&home));
                self.undo.push(Undo::Copied { to });
            }
            End::Stopped => { let _ = self.remove_all(window, &part); self.status = "Stopped downloading: nothing half-copied was left.".into(); sex_pdx::serial_record!("[files.app.places.cancelled] kind=download place={}", place); }
            End::Failed(why) => {
                let _ = self.remove_all(window, &part);
                sex_pdx::serial_record!("[files.app.places.failed] kind=download place={} member={} why={}", place, top, why);
                self.status = format!("Couldn't copy {} down: {}. Nothing half-copied was left.", name, why);
            }
        }
        ticker.finish(window);
        let cursor = self.cursor;
        let status = core::mem::take(&mut self.status);
        self.loom.places.view = Some(View::Browse { place, conn, dir });
        self.places_list();
        self.cursor = cursor.min(self.entries.len().saturating_sub(1));
        self.status = status;
        set_window_none();
    }

    /// Ctrl+V on a place: what Ctrl+C picked goes up into the folder being looked at.
    fn places_upload(&mut self, window: &mut Window) {
        let Some((from, _)) = self.clip.clone() else { self.status = "Nothing to upload: Ctrl+C a file or folder first, then come back and Ctrl+V.".into(); return };
        let Some(View::Browse { place, mut conn, dir }) = self.loom.places.view.take() else { return };
        let wp: *mut Window = window;
        set_window(window);
        let name = String::from(shown(base(&from)));
        // Everything to send: (local path, remote path, folder, size).
        let mut items: Vec<(Vec<u8>, String, bool, u64)> = Vec::new();
        let mut todo = vec![(from.clone(), rjoin(&dir, &name))];
        while let Some((local, remote)) = todo.pop() {
            let Ok(st) = direct::stat(&at(&local)) else { continue };
            items.push((local.clone(), remote.clone(), st.dir, st.size));
            if st.dir { for k in direct::list(&at(&local)).unwrap_or_default() { todo.push((join(&local, &k.name), rjoin(&remote, shown(&k.name)))); } }
        }
        items.sort_by(|a, b| a.1.cmp(&b.1));
        let total: u64 = items.iter().map(|i| i.3).sum();
        sex_pdx::serial_record!("[files.app.places.upload.start] place={} from={} to={} items={} bytes={}", place, shown(&from), rjoin(&dir, &name), items.len(), total);
        let mut ticker = Ticker::new("Uploading", name.as_bytes());
        let (mut done, mut files_n, mut stopped) = (0u64, 0u64, false);
        let mut end = if items.is_empty() { End::Failed("it couldn't be read".into()) } else { End::Done };
        for (local, remote, is_dir, size) in &items {
            if !matches!(end, End::Done) { break; }
            if *is_dir {
                match conn.mkdir(remote) { Ok(()) | Err(loom_remote::Error::Exists) => {} Err(e) => end = End::Failed(remote_err(e)) }
                continue;
            }
            files_n += 1;
            let r = {
                let me: *mut App = self;
                let mut tick = |n: u64| { done += n; match ticker.tick(unsafe { &mut *me }, unsafe { &mut *wp }, Progress { done, total }) { Flow::Cancel => { stopped = true; false } _ => true } };
                let mut src = Upload { path: local.clone(), off: 0, size: *size, tick: &mut tick };
                conn.write(remote, &mut src, Some(*size))
            };
            match r { Ok(_) => {} Err(_) if stopped => end = End::Stopped, Err(e) => end = End::Failed(remote_err(e)) }
        }
        let window = unsafe { &mut *wp };
        match end {
            End::Done => {
                sex_pdx::serial_record!("[files.app.places.uploaded] place={} from={} to={} files={} bytes={}", place, shown(&from), rjoin(&dir, &name), files_n, done);
                self.status = format!("Uploaded {} ({} files, {}).", name, files_n, crate::human(done));
            }
            End::Stopped => { sex_pdx::serial_record!("[files.app.places.cancelled] kind=upload place={}", place); self.status = "Stopped uploading: the file that was going up was not finished on the server.".into(); }
            End::Failed(why) => {
                sex_pdx::serial_record!("[files.app.places.failed] kind=upload place={} from={} why={}", place, shown(&from), why);
                self.status = format!("Couldn't upload {}: {}.", name, why);
            }
        }
        ticker.finish(window);
        let status = core::mem::take(&mut self.status);
        self.loom.places.view = Some(View::Browse { place, conn, dir });
        self.places_list();
        self.select(name.as_bytes());
        self.status = status;
        set_window_none();
    }
}

fn set_window(window: &mut Window) { unsafe { WINDOW = window; } }
/// Between operations nothing may use the window through the pointer; a connection
/// dropped then just lets its flow go when Files ends.
fn set_window_none() {}
