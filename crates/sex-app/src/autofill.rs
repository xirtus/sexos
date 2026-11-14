// SPDX-License-Identifier: GPL-3.0-only
//! Live, explicit-consent Chastity client/provider transport through Shell's
//! existing BOOT route. One autofill role/session per program main task.
//! Programs poll this alongside their Window; while active use short waits
//! (1 tick), not Window::next(), which waits indefinitely for a UI event.
use crate::{mailbox, Window};
use alloc::vec::Vec;
use sex_pdx::{autofill as w, PdxMessage};
use zeroize::{Zeroize, Zeroizing};

fn send(p: &w::Packet) -> bool {
    sex_pdx::pdx_call(
        sex_pdx::SLOT_SHELL,
        p.opcode,
        p.args[0],
        p.args[1],
        p.args[2],
    )
    .0 == 0
}
pub(crate) fn erase_message(m: &mut PdxMessage) {
    unsafe {
        for v in [&mut m.type_id, &mut m.arg0, &mut m.arg1, &mut m.arg2] {
            core::ptr::write_volatile(v, 0);
        }
    }
}
fn shell(m: &PdxMessage) -> bool {
    m.caller_pd == w::Subject::SHELL.pd
        && m.caller_task == w::Subject::SHELL.task
        && w::response(m.type_id)
}
fn incoming() -> Option<w::Packet> {
    if let Some(mut m) = mailbox::take(shell) {
        let p = w::Packet::new(m.type_id, m.arg0, m.arg1, m.arg2);
        erase_message(&mut m);
        return Some(p);
    }
    for _ in 0..64 {
        let mut m = sex_pdx::pdx_try_listen_raw(0)?;
        if shell(&m) {
            let p = w::Packet::new(m.type_id, m.arg0, m.arg1, m.arg2);
            erase_message(&mut m);
            return Some(p);
        }
        if w::response(m.type_id) || w::request(m.type_id) {
            erase_message(&mut m);
        } else {
            mailbox::put(m);
        }
    }
    None
}
fn current(window: &Window) -> w::Subject {
    w::Subject {
        pd: window.target,
        task: window.task,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Busy,
    Denied,
    Locked,
    Stale,
    Declined,
    Unavailable,
    Transport,
}
fn refusal(s: w::Status) -> Error {
    match s {
        w::Status::Ok => Error::Invalid,
        w::Status::Busy => Error::Busy,
        w::Status::Denied => Error::Denied,
        w::Status::Invalid => Error::Invalid,
        w::Status::Locked => Error::Locked,
        w::Status::Stale => Error::Stale,
        w::Status::Declined => Error::Declined,
        w::Status::Unavailable => Error::Unavailable,
    }
}
/// No Debug/Clone/Copy. Secret storage is allocated once and erased on drop.
pub struct Credentials {
    bytes: Zeroizing<Vec<u8>>,
    username: usize,
    password: usize,
}
impl Credentials {
    pub fn with<R>(&self, f: impl FnOnce(&str, &str) -> R) -> R {
        let user =
            core::str::from_utf8(&self.bytes[..self.username]).expect("validated credential UTF8");
        let pass = core::str::from_utf8(&self.bytes[self.username..self.username + self.password])
            .expect("validated credential UTF8");
        f(user, pass)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClientPhase {
    Begin,
    Ready,
    Origin,
    Consent,
    Data,
    Cancelling,
    Done,
}
pub struct Client {
    subject: w::Subject,
    sequence: u32,
    token: u32,
    epoch: u64,
    origin: Zeroizing<Vec<u8>>,
    origin_at: usize,
    waiting: bool,
    phase: ClientPhase,
    bytes: Zeroizing<Vec<u8>>,
    data_at: usize,
    lengths: (usize, usize),
    deadline: u64,
    last: u64,
    failed: Option<Error>,
    cancel_at: u64,
}
static SEQUENCE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
impl Client {
    pub fn request(window: &Window, origin: &str, now: u64) -> Result<Self, Error> {
        if !w::valid_origin(origin.as_bytes()) {
            return Err(Error::Invalid);
        }
        let seq = SEQUENCE
            .try_update(
                core::sync::atomic::Ordering::Relaxed,
                core::sync::atomic::Ordering::Relaxed,
                |v| v.checked_add(1),
            )
            .map_err(|_| Error::Invalid)?
            + 1;
        Ok(Self {
            subject: current(window),
            sequence: seq,
            token: 0,
            epoch: 0,
            origin: Zeroizing::new(origin.as_bytes().to_vec()),
            origin_at: 0,
            waiting: false,
            phase: ClientPhase::Begin,
            bytes: Zeroizing::new(alloc::vec![0;w::MAX_DATA]),
            data_at: 0,
            lengths: (0, 0),
            deadline: now.saturating_add(w::CONSENT_TICKS),
            last: now,
            failed: None,
            cancel_at: 0,
        })
    }
    pub fn active(&self) -> bool {
        self.phase != ClientPhase::Done
    }
    /// Wipe immediately and retain this object/poll until the broker confirms
    /// cancellation. A close/retired run is also revoked by native lifecycle.
    pub fn cancel(&mut self) {
        if self.phase == ClientPhase::Done {
            return;
        }
        self.bytes.as_mut_slice().zeroize();
        self.origin.as_mut_slice().zeroize();
        self.phase = ClientPhase::Cancelling;
        self.failed = None;
        self.cancel_at = 0;
    }
    fn fail(&mut self, error: Error) {
        self.bytes.as_mut_slice().zeroize();
        self.origin.as_mut_slice().zeroize();
        self.phase = ClientPhase::Done;
        self.failed = Some(error);
    }
    pub fn poll(&mut self, now: u64) -> Option<Result<Credentials, Error>> {
        if let Some(e) = self.failed.take() {
            return Some(Err(e));
        }
        if !self.active() {
            return None;
        }
        if now < self.last || now >= self.deadline {
            self.cancel();
            self.failed = None;
            return Some(Err(Error::Stale));
        }
        self.last = now;
        for _ in 0..64 {
            let Some(p) = incoming() else {
                break;
            };
            let a = p.args[0];
            let b = p.args[1];
            let c = if p.opcode == w::DATA || p.opcode == w::ORIGIN {
                0
            } else {
                p.args[2]
            };
            if p.opcode == w::CANCELLED
                && self.phase == ClientPhase::Cancelling
                && w::Subject::decode(c) == Some(self.subject)
            {
                let (id, epoch) = if self.token == 0 {
                    (self.sequence, 0)
                } else {
                    (self.token, self.epoch)
                };
                if a == id as u64 && b == epoch {
                    self.fail(Error::Declined);
                    break;
                }
            } else if p.opcode == w::STATUS {
                let id = if self.token == 0 {
                    self.sequence
                } else {
                    self.token
                };
                if a == id as u64 && (self.epoch == 0 || self.epoch == b) {
                    if let Some(status) = w::Status::decode(c) {
                        self.fail(refusal(status));
                        break;
                    }
                }
            } else if p.opcode == w::READY
                && self.phase == ClientPhase::Ready
                && a as u32 == self.sequence
                && (a >> 32) != 0
                && b != 0
                && w::Subject::decode(c) == Some(self.subject)
            {
                self.token = (a >> 32) as u32;
                self.epoch = b;
                self.phase = ClientPhase::Origin;
                self.waiting = false;
            } else if p.opcode == w::ACK
                && a == self.token as u64
                && b == self.epoch
                && self.phase == ClientPhase::Origin
                && self.waiting
            {
                let next = (self.origin_at + w::CHUNK).min(self.origin.len());
                if c == next as u64 {
                    self.origin_at = next;
                    self.waiting = false;
                    if next == self.origin.len() {
                        self.phase = ClientPhase::Consent;
                    }
                }
            } else if p.opcode == w::APPROVED
                && a == self.token as u64
                && b == self.epoch
                && self.phase == ClientPhase::Consent
            {
                if let Some(lengths) = w::lengths(c) {
                    self.lengths = lengths;
                    self.phase = ClientPhase::Data;
                    self.deadline = now.saturating_add(w::TRANSFER_TICKS);
                }
            } else if p.opcode == w::DATA && self.phase == ClientPhase::Data {
                if let Some((token, epoch, offset, count)) = w::chunk_meta(&p) {
                    if token == self.token
                        && epoch == self.epoch
                        && offset == self.data_at
                        && offset + count <= self.lengths.0 + self.lengths.1
                    {
                        w::chunk_copy(&p, &mut self.bytes);
                        self.data_at += count;
                    } else {
                        self.fail(Error::Invalid);
                        break;
                    }
                } else {
                    self.fail(Error::Invalid);
                    break;
                }
            } else if p.opcode == w::FINISH
                && a == self.token as u64
                && b == self.epoch
                && c == 0
                && self.phase == ClientPhase::Data
                && self.data_at == self.lengths.0 + self.lengths.1
            {
                if core::str::from_utf8(&self.bytes[..self.lengths.0]).is_err()
                    || core::str::from_utf8(&self.bytes[self.lengths.0..self.data_at]).is_err()
                {
                    self.fail(Error::Invalid);
                    break;
                }
                self.phase = ClientPhase::Done;
                self.origin.as_mut_slice().zeroize();
                let bytes = core::mem::replace(&mut self.bytes, Zeroizing::new(Vec::new()));
                return Some(Ok(Credentials {
                    bytes,
                    username: self.lengths.0,
                    password: self.lengths.1,
                }));
            }
        }
        if let Some(e) = self.failed.take() {
            return Some(Err(e));
        }
        if self.phase == ClientPhase::Cancelling {
            if now >= self.cancel_at {
                let packet = if self.token == 0 {
                    w::Packet::new(w::CANCEL, 0, 0, self.sequence as u64)
                } else {
                    w::Packet::new(w::CANCEL, self.token as u64, self.epoch, 0)
                };
                if send(&packet) {
                    self.cancel_at = now.saturating_add(100);
                }
            }
        } else if self.phase == ClientPhase::Begin {
            if send(&w::Packet::new(
                w::BEGIN,
                self.sequence as u64,
                self.origin.len() as u64,
                0,
            )) {
                self.phase = ClientPhase::Ready;
            }
        } else if self.phase == ClientPhase::Origin && !self.waiting {
            let end = (self.origin_at + w::CHUNK).min(self.origin.len());
            if let Some(p) = w::chunk(
                w::ORIGIN,
                self.token,
                self.epoch,
                self.origin_at,
                &self.origin[self.origin_at..end],
            ) {
                if send(&p) {
                    self.waiting = true;
                }
            }
        }
        None
    }
}
struct Request {
    token: u32,
    consumer: w::Subject,
    name: [u8; 16],
    names: u8,
    origin: Zeroizing<Vec<u8>>,
    origin_len: usize,
    origin_at: usize,
    deadline: u64,
}
/// Read-only request data, all supplied by Shell's authenticated broker. Name is
/// descriptive; the exact kernel run identity is always shown alongside it.
pub struct Offer<'a> {
    pub token: u32,
    pub consumer: w::Subject,
    pub name: &'a [u8; 16],
    pub origin: &'a str,
}
pub struct Provider {
    subject: w::Subject,
    epoch: u64,
    registered: bool,
    register_at: u64,
    request: Option<Request>,
    bytes: Zeroizing<Vec<u8>>,
    send_len: usize,
    send_at: usize,
    lengths: u64,
    header: bool,
    header_acked: bool,
    waiting: bool,
    finishing: bool,
    closing: bool,
    closed: bool,
    revoke_at: u64,
    last: u64,
}
impl Provider {
    pub fn new(window: &Window, epoch: u64) -> Result<Self, Error> {
        if epoch == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            subject: current(window),
            epoch,
            registered: false,
            register_at: 0,
            request: None,
            bytes: Zeroizing::new(alloc::vec![0;w::MAX_DATA]),
            send_len: 0,
            send_at: 0,
            lengths: 0,
            header: false,
            header_acked: false,
            waiting: false,
            finishing: false,
            closing: false,
            closed: false,
            revoke_at: 0,
            last: 0,
        })
    }
    pub fn busy(&self) -> bool {
        self.request.is_some()
    }
    pub fn registered(&self) -> bool {
        self.registered
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn offer(&self) -> Option<Offer<'_>> {
        let r = self.request.as_ref()?;
        if r.origin_at != r.origin_len || r.names != 3 || self.send_len != 0 {
            return None;
        }
        let origin = core::str::from_utf8(&r.origin[..r.origin_len]).ok()?;
        if !w::valid_origin(origin.as_bytes()) {
            return None;
        }
        Some(Offer {
            token: r.token,
            consumer: r.consumer,
            name: &r.name,
            origin,
        })
    }
    fn clear(&mut self) {
        self.bytes.as_mut_slice().zeroize();
        self.request = None;
        self.send_len = 0;
        self.send_at = 0;
        self.lengths = 0;
        self.header = false;
        self.header_acked = false;
        self.waiting = false;
        self.finishing = false;
    }
    pub fn deny(&mut self) {
        if let Some(r) = &self.request {
            send(&w::Packet::new(w::DENY, r.token as u64, self.epoch, 0));
        }
        self.clear();
    }
    /// Local secrets are wiped now; keep polling this provider until revoked()
    /// before reporting completed Lock or opening a new provider epoch.
    pub fn close(&mut self) {
        self.clear();
        self.registered = false;
        self.closing = true;
        self.closed = false;
        self.revoke_at = 0;
    }
    pub fn revoked(&self) -> bool {
        self.closed
    }
    pub fn closing(&self) -> bool {
        self.closing
    }
    pub fn allow(&mut self, username: &str, password: &str, now: u64) -> Result<(), Error> {
        if self.offer().is_none()
            || username.len() > w::MAX_CREDENTIAL
            || password.is_empty()
            || password.len() > w::MAX_CREDENTIAL
        {
            return Err(Error::Invalid);
        }
        self.bytes.as_mut_slice().zeroize();
        self.bytes[..username.len()].copy_from_slice(username.as_bytes());
        self.bytes[username.len()..username.len() + password.len()]
            .copy_from_slice(password.as_bytes());
        self.send_len = username.len() + password.len();
        self.lengths = username.len() as u64 | (password.len() as u64) << 16;
        self.request.as_mut().unwrap().deadline = now.saturating_add(w::TRANSFER_TICKS);
        Ok(())
    }
    pub fn poll(&mut self, now: u64) {
        if self.closed {
            return;
        }
        if now < self.last {
            self.close();
        }
        self.last = now;
        if self.request.as_ref().is_some_and(|r| now >= r.deadline) {
            self.deny();
        }
        for _ in 0..64 {
            let Some(p) = incoming() else {
                break;
            };
            let a = p.args[0];
            let b = p.args[1];
            let c = if p.opcode == w::DATA || p.opcode == w::ORIGIN {
                0
            } else {
                p.args[2]
            };
            if self.closing {
                if p.opcode == w::REVOKED && a == self.epoch && b == self.subject.word() && c == 0 {
                    self.closing = false;
                    self.closed = true;
                }
                continue;
            }
            if p.opcode == w::REGISTERED && a == self.epoch && b == self.subject.word() && c == 0 {
                self.registered = true;
            } else if p.opcode == w::OFFER
                && self.registered
                && c == self.epoch
                && self.request.is_none()
            {
                let token = a as u32;
                let len = (a >> 32) as usize;
                if token != 0 && (1..=w::MAX_ORIGIN).contains(&len) {
                    if let Some(consumer) = w::Subject::decode(b) {
                        self.request = Some(Request {
                            token,
                            consumer,
                            name: [0; 16],
                            names: 0,
                            origin: Zeroizing::new(alloc::vec![0;w::MAX_ORIGIN]),
                            origin_len: len,
                            origin_at: 0,
                            deadline: now.saturating_add(w::CONSENT_TICKS),
                        });
                    }
                }
            } else if p.opcode == w::CONSUMER && b == self.epoch {
                if let Some(r) = &mut self.request {
                    let part = (a >> 32) as usize;
                    if a as u32 == r.token && part < 2 {
                        r.name[part * 8..part * 8 + 8].copy_from_slice(&c.to_le_bytes());
                        r.names |= 1 << part;
                    }
                }
            } else if p.opcode == w::ORIGIN {
                if let Some((token, epoch, offset, count)) = w::chunk_meta(&p) {
                    if let Some(r) = &mut self.request {
                        if token == r.token
                            && epoch == self.epoch
                            && offset == r.origin_at
                            && offset + count <= r.origin_len
                        {
                            w::chunk_copy(&p, &mut r.origin);
                            r.origin_at += count;
                        }
                    }
                }
            } else if p.opcode == w::CANCEL
                && b == self.epoch
                && self.request.as_ref().is_some_and(|r| r.token as u64 == a)
            {
                self.clear();
            } else if p.opcode == w::ACK
                && b == self.epoch
                && self.waiting
                && self.request.as_ref().is_some_and(|r| r.token as u64 == a)
            {
                if c == u64::MAX && self.finishing {
                    self.clear();
                } else if !self.finishing {
                    let expected = if !self.header_acked {
                        0
                    } else {
                        (self.send_at + w::CHUNK).min(self.send_len)
                    };
                    if c == expected as u64 {
                        self.header_acked = true;
                        self.send_at = expected;
                        self.waiting = false;
                    }
                }
            }
        }
        if self.closed {
            return;
        }
        if self.closing {
            if now >= self.revoke_at && send(&w::Packet::new(w::UNREGISTER, self.epoch, 0, 0)) {
                self.revoke_at = now.saturating_add(100);
            }
            return;
        }
        if !self.registered && now >= self.register_at {
            if send(&w::Packet::new(w::REGISTER, self.epoch, 0, 0)) {
                self.register_at = now.saturating_add(1000);
            }
        }
        let Some(r) = &self.request else {
            return;
        };
        if self.send_len == 0 || self.waiting {
            return;
        }
        let packet = if !self.header {
            w::Packet::new(w::ALLOW, r.token as u64, self.epoch, self.lengths)
        } else if self.send_at == self.send_len {
            w::Packet::new(w::FINISH, r.token as u64, self.epoch, 0)
        } else {
            let end = (self.send_at + w::CHUNK).min(self.send_len);
            w::chunk(
                w::DATA,
                r.token,
                self.epoch,
                self.send_at,
                &self.bytes[self.send_at..end],
            )
            .unwrap()
        };
        if send(&packet) {
            if !self.header {
                self.header = true;
            } else if self.send_at == self.send_len {
                self.finishing = true;
            }
            self.waiting = true;
        }
    }
}
