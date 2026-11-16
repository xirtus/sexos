// SPDX-License-Identifier: GPL-3.0-only
//! SFTP version 3 (draft-ietf-secsh-filexfer-02, what OpenSSH speaks) over a byte
//! stream: an SSH `sftp` subsystem ([`SshChannel`]) or a server's pipes.
//! Reads and writes keep several requests in flight. A write goes to a temporary
//! name in the same folder, then replaces the target with OpenSSH's
//! `posix-rename@openssh.com` when the server has it (atomic), else remove + rename.
use crate::{components, split_last, temp_name, Abilities, Entry, Error, Kind, Place, Result, Source, Stream};
use alloc::{boxed::Box, collections::BTreeMap, format, string::{String, ToString}, vec, vec::Vec};
use sex_ssh::wire::{Reader, Writer};

const INIT: u8 = 1; const VERSION: u8 = 2; const OPEN: u8 = 3; const CLOSE: u8 = 4; const READ: u8 = 5; const WRITE: u8 = 6;
const LSTAT: u8 = 7; const OPENDIR: u8 = 11; const READDIR: u8 = 12; const REMOVE: u8 = 13; const MKDIR: u8 = 14;
const RMDIR: u8 = 15; const STAT: u8 = 17; const RENAME: u8 = 18; const EXTENDED: u8 = 200;
const STATUS: u8 = 101; const HANDLE: u8 = 102; const DATA: u8 = 103; const NAME: u8 = 104; const ATTRS: u8 = 105;

const FX_OK: u32 = 0; const FX_EOF: u32 = 1; const FX_NO_SUCH_FILE: u32 = 2; const FX_PERMISSION_DENIED: u32 = 3;
const FX_OP_UNSUPPORTED: u32 = 8;
const READ_ACCESS: u32 = 1; const WRITE_ACCESS: u32 = 2; const CREAT: u32 = 8; const TRUNC: u32 = 0x10; const EXCL: u32 = 0x20;

/// Bytes per READ/WRITE request (OpenSSH's sftp uses 32 KiB) and how many fly at once.
const CHUNK: usize = 32 << 10;
const IN_FLIGHT: usize = 16;
/// A packet bigger than this is a broken server.
const PACKET_MAX: usize = 256 << 10;

/// File attributes as SFTP v3 sends them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attrs { pub size: Option<u64>, pub perms: Option<u32>, pub mtime: Option<u32> }
impl Attrs {
    fn read(r: &mut Reader) -> Result<Attrs> {
        let flags = r.u32().map_err(proto)?;
        let mut a = Attrs::default();
        if flags & 1 != 0 { let hi = r.u32().map_err(proto)? as u64; a.size = Some(hi << 32 | r.u32().map_err(proto)? as u64); }
        if flags & 2 != 0 { r.u32().map_err(proto)?; r.u32().map_err(proto)?; }
        if flags & 4 != 0 { a.perms = Some(r.u32().map_err(proto)?); }
        if flags & 8 != 0 { r.u32().map_err(proto)?; a.mtime = Some(r.u32().map_err(proto)?); }
        if flags & 0x8000_0000 != 0 { for _ in 0..r.u32().map_err(proto)? { r.string().map_err(proto)?; r.string().map_err(proto)?; } }
        Ok(a)
    }
    pub fn kind(&self) -> Kind {
        match self.perms.map(|p| p & 0o170000) { Some(0o040000) => Kind::Dir, Some(0o100000) | None => Kind::File, Some(0o120000) => Kind::Link, _ => Kind::Other }
    }
    fn entry(&self, name: String) -> Entry {
        let kind = self.kind();
        Entry { name, kind, size: if kind == Kind::Dir { 0 } else { self.size.unwrap_or(0) }, mtime: self.mtime.map(u64::from) }
    }
}
fn proto(_: sex_ssh::Error) -> Error { Error::Protocol("short SFTP packet") }

/// A reply: its type and the bytes after the request id.
struct Reply { kind: u8, body: Vec<u8> }

pub struct Sftp<S: Stream> {
    io: S,
    buf: Vec<u8>,
    next_id: u32,
    early: BTreeMap<u32, Reply>,
    extensions: Vec<(String, Vec<u8>)>,
    /// The folder place paths start from: "" (the login's home), "/" or an absolute path.
    root: String,
    temp_counter: u32,
}

impl<S: Stream> Sftp<S> {
    /// Says hello (version 3). `root` is the folder the place shows ("" for the
    /// login's home folder, or an absolute path like "/srv/share").
    pub fn new(io: S, root: &str) -> Result<Self> {
        let mut s = Self { io, buf: Vec::new(), next_id: 1, early: BTreeMap::new(), extensions: Vec::new(), root: if root == "/" { root.into() } else { root.trim_end_matches('/').into() }, temp_counter: 0 };
        let mut w = Writer::new();
        w.byte(INIT).u32(3);
        s.packet(&w.finish())?;
        let (kind, body) = s.read_packet()?;
        if kind != VERSION { return Err(Error::Protocol("no SFTP VERSION")); }
        let mut r = Reader::new(&body);
        let v = r.u32().map_err(proto)?;
        if v < 3 { return Err(Error::Protocol("SFTP version below 3")); }
        while !r.rest().is_empty() {
            let n = r.string().map_err(proto)?;
            let d = r.string().map_err(proto)?;
            s.extensions.push((String::from_utf8_lossy(n).into_owned(), d.to_vec()));
        }
        // Mix in something per-connection so two writers rarely pick the same temp name.
        s.temp_counter = s.extensions.len() as u32 ^ (&s as *const _ as usize as u32).rotate_left(7);
        Ok(s)
    }
    pub fn has_extension(&self, name: &str) -> bool { self.extensions.iter().any(|(n, _)| n == name) }
    pub fn into_inner(self) -> S { self.io }

    fn packet(&mut self, payload: &[u8]) -> Result<()> {
        let mut m = Vec::with_capacity(payload.len() + 4);
        m.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        m.extend_from_slice(payload);
        self.io.write_all(&m)
    }
    fn read_packet(&mut self) -> Result<(u8, Vec<u8>)> {
        let mut tmp = vec![0u8; 64 << 10];
        loop {
            if self.buf.len() >= 4 {
                let n = u32::from_be_bytes(self.buf[..4].try_into().unwrap()) as usize;
                if n == 0 || n > PACKET_MAX { return Err(Error::Protocol("bad SFTP packet length")); }
                if self.buf.len() >= 4 + n {
                    let kind = self.buf[4];
                    let body = self.buf[5..4 + n].to_vec();
                    self.buf.drain(..4 + n);
                    return Ok((kind, body));
                }
            }
            let k = self.io.read(&mut tmp)?;
            if k == 0 { return Err(Error::Transport("SFTP server closed".into())); }
            self.buf.extend_from_slice(&tmp[..k]);
        }
    }
    /// Sends a request (type, then id, then `args`); gives its id.
    fn request(&mut self, kind: u8, args: impl FnOnce(&mut Writer)) -> Result<u32> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mut w = Writer::new();
        w.byte(kind).u32(id);
        args(&mut w);
        self.packet(&w.finish())?;
        Ok(id)
    }
    fn wait(&mut self, id: u32) -> Result<Reply> {
        if let Some(r) = self.early.remove(&id) { return Ok(r); }
        loop {
            let (kind, body) = self.read_packet()?;
            if body.len() < 4 { return Err(Error::Protocol("SFTP reply without id")); }
            let got = u32::from_be_bytes(body[..4].try_into().unwrap());
            let reply = Reply { kind, body: body[4..].to_vec() };
            if got == id { return Ok(reply); }
            self.early.insert(got, reply);
        }
    }
    fn call(&mut self, kind: u8, args: impl FnOnce(&mut Writer)) -> Result<Reply> { let id = self.request(kind, args)?; self.wait(id) }

    fn path(&self, path: &str) -> Result<String> {
        let c = components(path)?;
        if c.is_empty() { return Ok(if self.root.is_empty() { ".".into() } else { self.root.clone() }); }
        let mut p = self.root.clone();
        if !p.is_empty() && !p.ends_with('/') { p.push('/'); }
        p.push_str(&c.join("/"));
        Ok(p)
    }
    fn join(&self, parent: &[&str], name: &str) -> Result<String> { let mut v = parent.to_vec(); v.push(name); self.path(&v.join("/")) }

    fn handle(&mut self, kind: u8, args: impl FnOnce(&mut Writer)) -> Result<Vec<u8>> {
        let r = self.call(kind, args)?;
        match r.kind { HANDLE => Ok(Reader::new(&r.body).string().map_err(proto)?.to_vec()), STATUS => Err(status(&r.body).unwrap_err()), _ => Err(Error::Protocol("expected HANDLE")) }
    }
    fn close(&mut self, h: &[u8]) -> Result<()> { let r = self.call(CLOSE, |w| { w.string(h); })?; expect_ok(&r) }
    fn attrs(&mut self, kind: u8, p: &str) -> Result<Attrs> {
        let r = self.call(kind, |w| { w.string(p.as_bytes()); })?;
        match r.kind { ATTRS => Attrs::read(&mut Reader::new(&r.body)), STATUS => Err(status(&r.body).unwrap_err()), _ => Err(Error::Protocol("expected ATTRS")) }
    }
    fn simple(&mut self, kind: u8, args: impl FnOnce(&mut Writer)) -> Result<()> { let r = self.call(kind, args)?; expect_ok(&r) }
    fn exists(&mut self, p: &str) -> Result<bool> {
        match self.attrs(LSTAT, p) { Ok(_) => Ok(true), Err(Error::NotFound) => Ok(false), Err(e) => Err(e) }
    }

    fn list_path(&mut self, p: &str) -> Result<Vec<Entry>> {
        let h = self.handle(OPENDIR, |w| { w.string(p.as_bytes()); })?;
        let mut out = Vec::new();
        let res = loop {
            let r = match self.call(READDIR, |w| { w.string(&h); }) { Ok(r) => r, Err(e) => break Err(e) };
            match r.kind {
                NAME => {
                    let mut rd = Reader::new(&r.body);
                    let parsed: Result<()> = (|| {
                        for _ in 0..rd.u32().map_err(proto)? {
                            let name = String::from_utf8_lossy(rd.string().map_err(proto)?).into_owned();
                            rd.string().map_err(proto)?;
                            let a = Attrs::read(&mut rd)?;
                            if name != "." && name != ".." { out.push(a.entry(name)); }
                        }
                        Ok(())
                    })();
                    if let Err(e) = parsed { break Err(e); }
                }
                STATUS => match status(&r.body) { Err(Error::Remote(FX_EOF, _)) | Ok(()) => break Ok(()), Err(e) => break Err(e) },
                _ => break Err(Error::Protocol("expected NAME")),
            }
        };
        let closed = self.close(&h);
        res?;
        closed?;
        crate::sort_entries(&mut out);
        Ok(out)
    }

    fn read_handle(&mut self, h: &[u8], offset: u64, len: u64) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut next = offset;
        let end = offset.saturating_add(len);
        let mut flying: alloc::collections::VecDeque<(u32, u64, u32)> = Default::default();
        let mut eof = false;
        let mut err = None;
        loop {
            while !eof && err.is_none() && flying.len() < IN_FLIGHT && next < end {
                let n = (end - next).min(CHUNK as u64) as u32;
                match self.request(READ, |w| { w.string(h).u32((next >> 32) as u32).u32(next as u32).u32(n); }) {
                    Ok(id) => flying.push_back((id, next, n)),
                    Err(e) => { err = Some(e); break; }
                }
                next += n as u64;
            }
            let Some((id, at, n)) = flying.pop_front() else { break };
            let r = match self.wait(id) { Ok(r) => r, Err(e) => return Err(e) };
            if eof || err.is_some() { continue; }
            match r.kind {
                DATA => {
                    let d = Reader::new(&r.body).string().map_err(proto)?.to_vec();
                    if at != offset + out.len() as u64 { err = Some(Error::Protocol("SFTP read out of order")); continue; }
                    out.extend_from_slice(&d);
                    if (d.len() as u32) < n {
                        // A short read: fetch the rest of this piece before the ones in flight.
                        let mut pos = at + d.len() as u64;
                        let stop = at + n as u64;
                        while pos < stop {
                            let k = (stop - pos) as u32;
                            let r = self.call(READ, |w| { w.string(h).u32((pos >> 32) as u32).u32(pos as u32).u32(k); })?;
                            match r.kind {
                                DATA => { let d = Reader::new(&r.body).string().map_err(proto)?.to_vec(); if d.is_empty() { eof = true; break; } pos += d.len() as u64; out.extend_from_slice(&d); }
                                STATUS => { match status(&r.body) { Err(Error::Remote(FX_EOF, _)) => { eof = true; break; } Err(e) => return Err(e), Ok(()) => return Err(Error::Protocol("READ said OK")) } }
                                _ => return Err(Error::Protocol("expected DATA")),
                            }
                        }
                    }
                }
                STATUS => match status(&r.body) { Err(Error::Remote(FX_EOF, _)) => eof = true, Err(e) => err = Some(e), Ok(()) => err = Some(Error::Protocol("READ said OK")) },
                _ => err = Some(Error::Protocol("expected DATA")),
            }
        }
        match err { Some(e) => Err(e), None => Ok(out) }
    }

    fn write_handle(&mut self, h: &[u8], src: &mut dyn Source) -> Result<u64> {
        let mut flying = alloc::collections::VecDeque::new();
        let mut at = 0u64;
        let mut buf = vec![0u8; CHUNK];
        let mut err = None;
        loop {
            if err.is_none() && flying.len() < IN_FLIGHT {
                match crate::read_full(src, &mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        let d = &buf[..n];
                        match self.request(WRITE, |w| { w.string(h).u32((at >> 32) as u32).u32(at as u32).string(d); }) { Ok(id) => flying.push_back(id), Err(e) => return Err(e) }
                        at += n as u64;
                        continue;
                    }
                    Err(e) => err = Some(e),
                }
            }
            let Some(id) = flying.pop_front() else { break };
            let r = self.wait(id)?;
            if let Err(e) = expect_ok(&r) { err.get_or_insert(e); }
        }
        match err { Some(e) => Err(e), None => Ok(at) }
    }
    /// Replaces `to` with `from`.
    fn replace(&mut self, from: &str, to: &str) -> Result<()> {
        if self.has_extension("posix-rename@openssh.com") {
            return self.simple(EXTENDED, |w| { w.string(b"posix-rename@openssh.com").string(from.as_bytes()).string(to.as_bytes()); });
        }
        if self.exists(to)? { self.simple(REMOVE, |w| { w.string(to.as_bytes()); })?; }
        self.simple(RENAME, |w| { w.string(from.as_bytes()).string(to.as_bytes()); })
    }
}

fn status(body: &[u8]) -> Result<()> {
    let mut r = Reader::new(body);
    let code = r.u32().map_err(proto)?;
    let msg = r.string().map(|m| String::from_utf8_lossy(m).into_owned()).unwrap_or_default();
    match code {
        FX_OK => Ok(()),
        FX_NO_SUCH_FILE => Err(Error::NotFound),
        FX_PERMISSION_DENIED => Err(Error::Denied),
        FX_OP_UNSUPPORTED => Err(Error::Unsupported),
        c => Err(Error::Remote(c, msg)),
    }
}
fn expect_ok(r: &Reply) -> Result<()> { if r.kind == STATUS { status(&r.body) } else { Err(Error::Protocol("expected STATUS")) } }

impl<S: Stream> Place for Sftp<S> {
    fn abilities(&self) -> Abilities { Abilities { atomic_write: self.has_extension("posix-rename@openssh.com"), atomic_dir_rename: true } }
    fn list(&mut self, dir: &str) -> Result<Vec<Entry>> { let p = self.path(dir)?; self.list_path(&p) }
    fn stat(&mut self, path: &str) -> Result<Entry> {
        let p = self.path(path)?;
        let a = self.attrs(LSTAT, &p)?;
        let a = if a.kind() == Kind::Link { self.attrs(STAT, &p).unwrap_or(a) } else { a };
        Ok(a.entry(components(path)?.last().unwrap_or(&"").to_string()))
    }
    fn read(&mut self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let p = self.path(path)?;
        let h = self.handle(OPEN, |w| { w.string(p.as_bytes()).u32(READ_ACCESS).u32(0); })?;
        let r = self.read_handle(&h, offset, len);
        let c = self.close(&h);
        let r = r?;
        c?;
        Ok(r)
    }
    fn write(&mut self, path: &str, src: &mut dyn Source, _size: Option<u64>) -> Result<u64> {
        let (parent, name) = split_last(path)?;
        let target = self.join(&parent, name)?;
        if self.attrs(LSTAT, &target).is_ok_and(|a| a.kind() == Kind::Dir) { return Err(Error::Exists); }
        // A fresh temporary name (EXCL, so never someone else's file).
        let (tmp, h) = 'open: {
            for _ in 0..8 {
                self.temp_counter = self.temp_counter.wrapping_mul(1664525).wrapping_add(1013904223);
                let tmp = self.join(&parent, &temp_name(name, self.temp_counter))?;
                match self.handle(OPEN, |w| { w.string(tmp.as_bytes()).u32(WRITE_ACCESS | CREAT | EXCL | TRUNC).u32(4).u32(0o644); }) {
                    Ok(h) => break 'open (tmp, h),
                    Err(Error::Remote(..)) if self.exists(&tmp).unwrap_or(false) => continue,
                    Err(e) => return Err(e),
                }
            }
            return Err(Error::Exists);
        };
        let written = self.write_handle(&h, src);
        let closed = self.close(&h);
        let done = written.and_then(|n| closed.map(|_| n)).and_then(|n| self.replace(&tmp, &target).map(|_| n));
        if done.is_err() { let _ = self.simple(REMOVE, |w| { w.string(tmp.as_bytes()); }); }
        done
    }
    fn mkdir(&mut self, path: &str) -> Result<()> {
        let p = self.path(path)?;
        if components(path)?.is_empty() { return Err(Error::BadPath); }
        match self.simple(MKDIR, |w| { w.string(p.as_bytes()).u32(0); }) {
            Err(Error::Remote(..)) if self.exists(&p)? => Err(Error::Exists),
            r => r,
        }
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if components(from)?.is_empty() || components(to)?.is_empty() { return Err(Error::BadPath); }
        let (f, t) = (self.path(from)?, self.path(to)?);
        if self.exists(&t)? { return Err(Error::Exists); }
        self.simple(RENAME, |w| { w.string(f.as_bytes()).string(t.as_bytes()); })
    }
    fn delete(&mut self, path: &str) -> Result<()> {
        if components(path)?.is_empty() { return Err(Error::BadPath); }
        let p = self.path(path)?;
        if self.attrs(LSTAT, &p)?.kind() == Kind::Dir {
            if !self.list_path(&p)?.is_empty() { return Err(Error::NotEmpty); }
            self.simple(RMDIR, |w| { w.string(p.as_bytes()); })
        } else {
            self.simple(REMOVE, |w| { w.string(p.as_bytes()); })
        }
    }
}

/// An SSH session channel as a [`Stream`]: drives sex-ssh's client over a
/// connection `T` to the server. Start the client with `Run::Subsystem("sftp")`.
pub struct SshChannel<T: Stream> {
    ssh: sex_ssh::Client,
    conn: T,
    data: Vec<u8>,
    open: bool,
    closed: bool,
    /// Decides whether to trust the server's host key (algorithm, key blob, SHA256 fingerprint).
    trust: Box<dyn FnMut(&str, &[u8], &str) -> bool>,
}

impl<T: Stream> SshChannel<T> {
    /// Logs in and waits until the subsystem's channel is open.
    pub fn open(ssh: sex_ssh::Client, conn: T, trust: Box<dyn FnMut(&str, &[u8], &str) -> bool>) -> Result<Self> {
        let mut c = Self { ssh, conn, data: Vec::new(), open: false, closed: false, trust };
        while !c.open {
            if c.closed { return Err(Error::Transport("SSH session closed before it opened".into())); }
            c.pump()?;
        }
        Ok(c)
    }
    fn pump(&mut self) -> Result<()> {
        let o = self.ssh.output();
        if !o.is_empty() { self.conn.write_all(&o)?; }
        let mut buf = vec![0u8; 64 << 10];
        let n = self.conn.read(&mut buf)?;
        if n == 0 { self.closed = true; return Ok(()); }
        self.ssh.receive(&buf[..n]).map_err(ssh_error)?;
        while let Some(e) = self.ssh.event() {
            match e {
                sex_ssh::Event::HostKey { algorithm, blob, fingerprint } => {
                    if (self.trust)(&algorithm, &blob, &fingerprint) { self.ssh.accept_host_key().map_err(ssh_error)?; } else {
                        let _ = self.ssh.reject_host_key();
                        let o = self.ssh.output();
                        let _ = self.conn.write_all(&o);
                        return Err(Error::Denied);
                    }
                }
                sex_ssh::Event::SessionOpen => self.open = true,
                sex_ssh::Event::Data { bytes, stderr: false } => self.data.extend_from_slice(&bytes),
                sex_ssh::Event::Closed => self.closed = true,
                _ => {}
            }
        }
        let o = self.ssh.output();
        if !o.is_empty() { self.conn.write_all(&o)?; }
        Ok(())
    }
    /// Ends the session politely.
    pub fn close(mut self) -> T { self.ssh.eof(); let o = self.ssh.output(); let _ = self.conn.write_all(&o); self.conn }
}
fn ssh_error(e: sex_ssh::Error) -> Error {
    match e { sex_ssh::Error::AuthFailed => Error::Denied, sex_ssh::Error::HostKeyRejected => Error::Denied, e => Error::Transport(format!("ssh: {e:?}")) }
}

impl<T: Stream> Stream for SshChannel<T> {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        if self.closed { return Err(Error::Transport("SSH session closed".into())); }
        self.ssh.send(bytes);
        let o = self.ssh.output();
        if !o.is_empty() { self.conn.write_all(&o)?; }
        Ok(())
    }
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        while self.data.is_empty() && !self.closed { self.pump()?; }
        let n = buf.len().min(self.data.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data.drain(..n);
        Ok(n)
    }
}
