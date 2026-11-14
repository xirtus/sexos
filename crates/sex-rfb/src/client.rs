// SPDX-License-Identifier: GPL-3.0-only
//! The viewer's side: handshake, then framebuffer updates in Raw, CopyRect, RRE,
//! Hextile and ZRLE, the DesktopSize and Cursor pseudo-encodings, bells and cut text.
//!
//! ```text
//! let mut c = Client::new(password);
//! loop { c.feed(&bytes_from_server); while let Some(e) = c.next_event()? { ... }
//!        send(&c.take_output()); }
//! ```
use crate::{des, encoding, security, zrle, Error, Framebuffer, PixelFormat, Reader, Rect, MAX_SIDE};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

/// What the client asks the server for, best first.
pub const ENCODINGS: [i32; 7] = [encoding::COPY_RECT, encoding::ZRLE, encoding::HEXTILE, encoding::RRE, encoding::RAW, encoding::CURSOR, encoding::DESKTOP_SIZE];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// ServerInit arrived: [`Client::name`] and [`Client::fb`] are set.
    Connected,
    /// A whole FramebufferUpdate was applied; the area it changed (empty when none).
    Updated { area: Option<Rect>, rects: u16 },
    /// The server's screen changed size (DesktopSize); the framebuffer was remade.
    Resized { width: usize, height: usize },
    Bell,
    /// The server's clipboard text, Latin-1.
    CutText(Vec<u8>),
    /// The server sent a new pointer shape ([`Client::cursor`]).
    Cursor,
}

/// The remote pointer's shape: `0xAARRGGBB`, alpha 0 or 255.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor { pub hot_x: u16, pub hot_y: u16, pub width: usize, pub height: usize, pub pixels: Vec<u32> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State { Version, Security, Security33, Challenge, Result, ServerInit, Normal }

pub struct Client {
    state: State,
    /// The protocol minor version agreed: 3, 7 or 8.
    pub minor: u8,
    password: Option<Vec<u8>>,
    input: Vec<u8>,
    output: Vec<u8>,
    events: VecDeque<Event>,
    pub fb: Framebuffer,
    pub name: Vec<u8>,
    /// The pixel format the server offered (the client then asks for RGB32).
    pub server_format: Option<PixelFormat>,
    pub cursor: Option<Cursor>,
    /// The security type used.
    pub security: u8,
    encodings: Vec<i32>,
    rects_left: u16,
    rects: u16,
    damage: Option<Rect>,
    zlib: Option<Box<InflateState>>,
    zbuf: Vec<u8>,
    /// Whole FramebufferUpdates applied so far.
    pub updates: u64,
}

impl Client {
    /// A client that will answer VNC Authentication with `password` if the server asks.
    pub fn new(password: Option<&[u8]>) -> Client { Client::with_encodings(password, &ENCODINGS) }

    pub fn with_encodings(password: Option<&[u8]>, encodings: &[i32]) -> Client {
        Client {
            state: State::Version, minor: 0, password: password.map(|p| p.to_vec()), input: Vec::new(), output: Vec::new(),
            events: VecDeque::new(), fb: Framebuffer::default(), name: Vec::new(), server_format: None, cursor: None, security: 0,
            encodings: encodings.to_vec(), rects_left: 0, rects: 0, damage: None, zlib: None, zbuf: Vec::new(), updates: 0,
        }
    }

    /// True once ServerInit has arrived.
    pub fn connected(&self) -> bool { self.state == State::Normal }

    /// Bytes that came from the server.
    pub fn feed(&mut self, bytes: &[u8]) { self.input.extend_from_slice(bytes); }

    /// The bytes to send to the server, taken.
    pub fn take_output(&mut self) -> Vec<u8> { core::mem::take(&mut self.output) }
    pub fn output(&self) -> &[u8] { &self.output }

    /// The next thing that happened, or `None` when more bytes must come first.
    pub fn next_event(&mut self) -> Result<Option<Event>, Error> {
        loop {
            if let Some(e) = self.events.pop_front() { return Ok(Some(e)); }
            let input = core::mem::take(&mut self.input);
            let mut r = Reader::new(&input);
            let step = self.step(&mut r);
            let used = r.at;
            self.input = input;
            match step {
                None => return Ok(None),
                Some(Err(e)) => return Err(e),
                Some(Ok(())) => { self.input.drain(..used); }
            }
        }
    }

    /// Asks for the whole screen (`incremental` false) or what changed since the last.
    pub fn request_update(&mut self, incremental: bool) {
        let (w, h) = (self.fb.width as u16, self.fb.height as u16);
        self.output.push(3);
        self.output.push(incremental as u8);
        for v in [0, 0, w, h] { self.output.extend_from_slice(&v.to_be_bytes()); }
    }

    /// A key went down or up.
    pub fn key(&mut self, keysym: u32, down: bool) {
        self.output.extend_from_slice(&[4, down as u8, 0, 0]);
        self.output.extend_from_slice(&keysym.to_be_bytes());
    }

    /// Presses and releases a key, with Control held around it when `control`.
    pub fn tap(&mut self, keysym: u32, control: bool) {
        if control { self.key(crate::keysym::CONTROL_L, true); }
        self.key(keysym, true);
        self.key(keysym, false);
        if control { self.key(crate::keysym::CONTROL_L, false); }
    }

    /// The pointer is at `(x, y)` with `buttons` held: bit 0 left, 1 middle, 2 right,
    /// 3 and 4 the wheel up and down.
    pub fn pointer(&mut self, buttons: u8, x: u16, y: u16) {
        self.output.extend_from_slice(&[5, buttons]);
        self.output.extend_from_slice(&x.to_be_bytes());
        self.output.extend_from_slice(&y.to_be_bytes());
    }

    /// This side's clipboard text, Latin-1.
    pub fn cut_text(&mut self, text: &[u8]) {
        self.output.extend_from_slice(&[6, 0, 0, 0]);
        self.output.extend_from_slice(&(text.len() as u32).to_be_bytes());
        self.output.extend_from_slice(text);
    }

    fn reason(r: &mut Reader) -> Option<Vec<u8>> {
        let n = r.u32()? as usize;
        Some(r.bytes(n.min(4096))?.to_vec())
    }

    fn client_init(&mut self) {
        self.output.push(1); // shared: other viewers stay connected
        self.state = State::ServerInit;
    }

    fn after_security(&mut self, chosen: u8) {
        self.security = chosen;
        match chosen {
            security::VNC_AUTH => self.state = State::Challenge,
            _ if self.minor >= 8 => self.state = State::Result,
            _ => self.client_init(),
        }
    }

    /// One message or handshake step. `None`: wait for more bytes (nothing consumed).
    fn step(&mut self, r: &mut Reader) -> Option<Result<(), Error>> {
        match self.state {
            State::Version => {
                let v = r.bytes(12)?;
                if &v[..4] != b"RFB " || v[7] != b'.' || v[11] != b'\n' { return Some(Err(Error::Protocol("version"))); }
                let num = |b: &[u8]| core::str::from_utf8(b).ok().and_then(|s| s.parse::<u32>().ok());
                let (Some(3), Some(minor)) = (num(&v[4..7]), num(&v[8..11])) else { return Some(Err(Error::Protocol("version"))) };
                self.minor = match minor { 8.. => 8, 7 => 7, _ => 3 };
                self.output.extend_from_slice(b"RFB 003.00");
                self.output.extend_from_slice(&[b'0' + self.minor, b'\n']);
                self.state = if self.minor >= 7 { State::Security } else { State::Security33 };
            }
            State::Security => {
                let n = r.u8()? as usize;
                if n == 0 { return Some(Err(Error::Refused(Self::reason(r)?))); }
                let types = r.bytes(n)?;
                let chosen = if self.password.is_some() && types.contains(&security::VNC_AUTH) {
                    security::VNC_AUTH
                } else if types.contains(&security::NONE) {
                    security::NONE
                } else if types.contains(&security::VNC_AUTH) {
                    return Some(Err(Error::NeedPassword));
                } else {
                    return Some(Err(Error::NoSecurity));
                };
                self.output.push(chosen);
                self.after_security(chosen);
            }
            State::Security33 => match r.u32()? {
                0 => return Some(Err(Error::Refused(Self::reason(r)?))),
                1 => self.after_security(security::NONE),
                2 if self.password.is_some() => self.after_security(security::VNC_AUTH),
                2 => return Some(Err(Error::NeedPassword)),
                _ => return Some(Err(Error::NoSecurity)),
            },
            State::Challenge => {
                let challenge: [u8; 16] = r.bytes(16)?.try_into().unwrap();
                let answer = des::vnc_response(self.password.as_deref().unwrap_or(b""), &challenge);
                self.output.extend_from_slice(&answer);
                self.state = State::Result;
            }
            State::Result => {
                if r.u32()? == 0 {
                    self.client_init();
                } else {
                    let why = if self.minor >= 8 { Self::reason(r)? } else { Vec::new() };
                    return Some(Err(Error::AuthFailed(why)));
                }
            }
            State::ServerInit => {
                let (w, h) = (r.u16()? as usize, r.u16()? as usize);
                let format = PixelFormat::from_bytes(r.bytes(16)?.try_into().unwrap());
                let n = r.u32()? as usize;
                if n > 1 << 16 { return Some(Err(Error::Protocol("desktop name length"))); }
                let name = r.bytes(n)?.to_vec();
                if w > MAX_SIDE || h > MAX_SIDE { return Some(Err(Error::TooLarge)); }
                self.fb = Framebuffer::new(w, h);
                self.name = name;
                self.server_format = Some(format);
                self.output.extend_from_slice(&[0, 0, 0, 0]);
                self.output.extend_from_slice(&PixelFormat::RGB32.to_bytes());
                self.output.extend_from_slice(&[2, 0]);
                self.output.extend_from_slice(&(self.encodings.len() as u16).to_be_bytes());
                for e in &self.encodings { self.output.extend_from_slice(&e.to_be_bytes()); }
                self.request_update(false);
                self.state = State::Normal;
                self.events.push_back(Event::Connected);
            }
            State::Normal if self.rects_left > 0 => return self.rect(r),
            State::Normal => match r.u8()? {
                0 => {
                    r.u8()?;
                    let n = r.u16()?;
                    if n == 0 {
                        self.updates += 1;
                        self.events.push_back(Event::Updated { area: None, rects: 0 });
                    }
                    (self.rects_left, self.rects, self.damage) = (n, n, None);
                }
                1 => {
                    r.bytes(3)?;
                    let n = r.u16()? as usize;
                    r.bytes(6 * n)?;
                }
                2 => self.events.push_back(Event::Bell),
                3 => {
                    r.bytes(3)?;
                    let n = r.u32()? as usize;
                    if n > 1 << 24 { return Some(Err(Error::Protocol("cut text length"))); }
                    let text = r.bytes(n)?.to_vec();
                    self.events.push_back(Event::CutText(text));
                }
                _ => return Some(Err(Error::Protocol("server message type"))),
            },
        }
        Some(Ok(()))
    }

    fn rect(&mut self, r: &mut Reader) -> Option<Result<(), Error>> {
        let (x, y, w, h) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
        let enc = r.u32()? as i32;
        let rect = Rect { x, y, w, h };
        let (ux, uy, uw, uh) = (x as usize, y as usize, w as usize, h as usize);
        let inside = self.fb.contains(ux, uy, uw, uh);
        let outside = Some(Err(Error::Protocol("rectangle outside the framebuffer")));
        let mut changed = true;
        match enc {
            encoding::RAW => {
                let px = r.bytes(uw * uh * 4)?;
                if !inside { return outside; }
                for row in 0..uh {
                    let line = &mut self.fb.pixels[(uy + row) * self.fb.width + ux..][..uw];
                    for (d, s) in line.iter_mut().zip(px[row * uw * 4..].chunks_exact(4)) {
                        *d = u32::from_le_bytes(s.try_into().unwrap()) & 0x00FF_FFFF;
                    }
                }
            }
            encoding::COPY_RECT => {
                let (sx, sy) = (r.u16()? as usize, r.u16()? as usize);
                if !inside || !self.fb.contains(sx, sy, uw, uh) { return outside; }
                self.fb.copy(sx, sy, ux, uy, uw, uh);
            }
            encoding::RRE => {
                let n = r.u32()? as usize;
                let bg = r.pixel()?;
                let subs = r.bytes(n.checked_mul(12)?)?;
                if !inside { return outside; }
                self.fb.fill(ux, uy, uw, uh, bg);
                for s in subs.chunks_exact(12) {
                    let c = u32::from_le_bytes(s[..4].try_into().unwrap()) & 0x00FF_FFFF;
                    let v = |i: usize| u16::from_be_bytes([s[4 + 2 * i], s[5 + 2 * i]]) as usize;
                    let (sx, sy, sw, sh) = (v(0), v(1), v(2), v(3));
                    if sx + sw > uw || sy + sh > uh { return Some(Err(Error::Protocol("RRE subrectangle"))); }
                    self.fb.fill(ux + sx, uy + sy, sw, sh, c);
                }
            }
            encoding::HEXTILE => {
                if !inside { return outside; }
                // Measure first, so a half-arrived rectangle paints nothing.
                let mut probe = Reader { buf: r.buf, at: r.at };
                if let Err(e) = hextile(&mut probe, None, ux, uy, uw, uh)? { return Some(Err(e)); }
                hextile(r, Some(&mut self.fb), ux, uy, uw, uh)?.ok()?;
            }
            encoding::ZRLE => {
                let n = r.u32()? as usize;
                let data = r.bytes(n)?;
                if !inside { return outside; }
                let state = self.zlib.get_or_insert_with(|| InflateState::new_boxed(DataFormat::Zlib));
                if let Err(e) = inflate_all(state, data, &mut self.zbuf) { return Some(Err(e)); }
                if let Err(e) = zrle::decode(&self.zbuf, &mut self.fb, ux, uy, uw, uh) { return Some(Err(e)); }
            }
            encoding::CURSOR => {
                let px = r.bytes(uw * uh * 4)?;
                let mask = r.bytes(uw.div_ceil(8) * uh)?;
                let mut pixels = Vec::with_capacity(uw * uh);
                for row in 0..uh {
                    for col in 0..uw {
                        let i = row * uw + col;
                        let c = u32::from_le_bytes(px[i * 4..i * 4 + 4].try_into().unwrap()) & 0x00FF_FFFF;
                        let on = mask[row * uw.div_ceil(8) + col / 8] & (0x80 >> (col % 8)) != 0;
                        pixels.push(if on { c | 0xFF00_0000 } else { 0 });
                    }
                }
                self.cursor = Some(Cursor { hot_x: x, hot_y: y, width: uw, height: uh, pixels });
                self.events.push_back(Event::Cursor);
                changed = false;
            }
            encoding::DESKTOP_SIZE => {
                if uw > MAX_SIDE || uh > MAX_SIDE { return Some(Err(Error::TooLarge)); }
                self.fb = Framebuffer::new(uw, uh);
                self.events.push_back(Event::Resized { width: uw, height: uh });
                let full = Rect { x: 0, y: 0, w, h };
                self.damage = Some(full);
                changed = false;
            }
            other => return Some(Err(Error::Encoding(other))),
        }
        if changed && w > 0 && h > 0 { self.damage = Some(self.damage.map_or(rect, |d| d.union(rect))); }
        self.rects_left -= 1;
        if self.rects_left == 0 {
            self.updates += 1;
            self.events.push_back(Event::Updated { area: self.damage.take(), rects: self.rects });
        }
        Some(Ok(()))
    }
}

/// Walks a Hextile rectangle, painting it when `fb` is given. `None`: incomplete.
fn hextile(r: &mut Reader, mut fb: Option<&mut Framebuffer>, x: usize, y: usize, w: usize, h: usize) -> Option<Result<(), Error>> {
    let (mut bg, mut fg) = (0u32, 0u32);
    for ty in (y..y + h).step_by(16) {
        for tx in (x..x + w).step_by(16) {
            let (tw, th) = (16.min(x + w - tx), 16.min(y + h - ty));
            let sub = r.u8()?;
            if sub & 1 != 0 {
                let px = r.bytes(tw * th * 4)?;
                if let Some(fb) = fb.as_deref_mut() {
                    for (i, s) in px.chunks_exact(4).enumerate() {
                        fb.pixels[(ty + i / tw) * fb.width + tx + i % tw] = u32::from_le_bytes(s.try_into().unwrap()) & 0x00FF_FFFF;
                    }
                }
                continue;
            }
            if sub & 2 != 0 { bg = r.pixel()?; }
            if sub & 4 != 0 { fg = r.pixel()?; }
            if let Some(fb) = fb.as_deref_mut() { fb.fill(tx, ty, tw, th, bg); }
            if sub & 8 != 0 {
                for _ in 0..r.u8()? {
                    let c = if sub & 16 != 0 { r.pixel()? } else { fg };
                    let (xy, wh) = (r.u8()? as usize, r.u8()? as usize);
                    let (sx, sy, sw, sh) = (xy >> 4, xy & 15, (wh >> 4) + 1, (wh & 15) + 1);
                    if sx + sw > tw || sy + sh > th { return Some(Err(Error::Protocol("Hextile subrectangle"))); }
                    if let Some(fb) = fb.as_deref_mut() { fb.fill(tx + sx, ty + sy, sw, sh, c); }
                }
            }
        }
    }
    Some(Ok(()))
}

/// Inflates all of `input` (one rectangle's share of the connection's zlib stream).
fn inflate_all(state: &mut InflateState, mut input: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
    const STEP: usize = 64 * 1024;
    const LIMIT: usize = MAX_SIDE * MAX_SIDE * 4 + MAX_SIDE * MAX_SIDE / 64;
    let bad = Error::Protocol("ZRLE zlib stream");
    out.clear();
    loop {
        let at = out.len();
        if at > LIMIT { return Err(bad); }
        out.resize(at + STEP, 0);
        let r = inflate(state, input, &mut out[at..], MZFlush::None);
        input = &input[r.bytes_consumed..];
        out.truncate(at + r.bytes_written);
        match r.status {
            Ok(MZStatus::StreamEnd) => return Ok(()),
            Ok(_) | Err(MZError::Buf) => {}
            Err(_) => return Err(bad),
        }
        if input.is_empty() && r.bytes_written < STEP { return Ok(()); }
        if r.bytes_consumed == 0 && r.bytes_written == 0 { return Err(bad); }
    }
}
