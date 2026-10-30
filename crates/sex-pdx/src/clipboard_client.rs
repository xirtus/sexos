// SPDX-License-Identifier: GPL-3.0-only
//! One bounded text handoff over an existing BOOT-held Shell route.
//!
//! This client owns scratch bytes, not clipboard authority. The caller sends
//! packets only through its validated Participant route and supplies actual IPC
//! sender PD/task metadata to `receive`. A successful copy is not permission to
//! delete a selection; callers must compare their immutable selection snapshot.
use crate::{
    clipboard::{
        self as abi, Action, Packet, Participant, Phase, Request, Response, Status, Subject,
        MAX_BYTES, SHELL,
    },
    text_selection::Source,
};

pub const TOTAL_TURNS: u64 = 65536;
pub const TOTAL_TICKS: u64 = 15000;
pub const STALL_TURNS: u64 = 4096;
pub const STALL_TICKS: u64 = 2000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Accepted,
    Full,
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Idle,
    Pending,
    Ready,
    Copied,
    Cancelled,
    Empty,
    Unsupported,
    Busy,
    Denied,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Export {
        ticket: u32,
        cut: bool,
    },
    Paste {
        ticket: u32,
    },
    Published {
        ticket: u32,
        cut: bool,
        len: usize,
        generation: u32,
        hash: u64,
    },
    Ready {
        ticket: u32,
        len: usize,
        generation: u32,
        hash: u64,
    },
    Stopped {
        ticket: u32,
        view: View,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Idle,
    Export,
    Paste,
    Begin,
    BeginAck,
    Write,
    Publish,
    PublishAck,
    Read,
    ReadAck,
    Get,
    Data,
    Finish,
    FinishAck,
    Ready,
    Cancel,
    CancelAck,
    Uncertain,
    Closed,
}
impl Step {
    fn active(self) -> bool {
        !matches!(self, Self::Idle | Self::Closed)
    }
}

pub struct Client {
    participant: Option<Participant>,
    step: Step,
    view: View,
    cancel_view: View,
    ticket: u32,
    latest: u32,
    action: Action,
    source: Source,
    len: usize,
    offset: usize,
    generation: u32,
    digest: u64,
    payload: [u8; MAX_BYTES],
    packet: Packet,
    packet_pending: bool,
    admitted: bool,
    event: Option<Event>,
    turn: u64,
    start_turn: u64,
    progress_turn: u64,
    clock_turn: u64,
    start_ticks: u64,
    progress_ticks: u64,
    last_ticks: u64,
}
impl Client {
    /// Construct directly in static storage; no hidden allocation or payload copy.
    pub const fn new() -> Self {
        Self {
            participant: None,
            step: Step::Idle,
            view: View::Idle,
            cancel_view: View::Cancelled,
            ticket: 0,
            latest: 0,
            action: Action::Copy,
            source: Source {
                generation: 0,
                revision: 0,
            },
            len: 0,
            offset: 0,
            generation: 0,
            digest: 0,
            payload: [0; MAX_BYTES],
            packet: Packet {
                opcode: 0,
                args: [0; 3],
            },
            packet_pending: false,
            admitted: false,
            event: None,
            turn: 0,
            start_turn: 0,
            progress_turn: 0,
            clock_turn: 0,
            start_ticks: 0,
            progress_ticks: 0,
            last_ticks: 0,
        }
    }
    /// Bind once. Participant must originate from a validated held catalog.
    pub fn bind(&mut self, p: Participant) -> bool {
        if self.participant.is_some()
            || self.step == Step::Closed
            || !p.subject.valid()
            || p.route == 0
            || p.surface == 0
        {
            return false;
        }
        self.participant = Some(p);
        true
    }
    pub fn participant(&self) -> Option<Participant> {
        self.participant
    }
    pub fn view(&self) -> View {
        self.view
    }
    pub fn pending(&self) -> bool {
        self.step.active()
    }
    pub fn ticket(&self) -> Option<u32> {
        self.step.active().then_some(self.ticket)
    }
    pub fn take_event(&mut self) -> Option<Event> {
        self.event.take()
    }
    /// No read payload is exposed before the exact positive FINISH receipt.
    pub fn bytes(&self) -> Option<&[u8]> {
        (self.step == Step::Ready).then_some(&self.payload[..self.len])
    }
    fn erase_payload(&mut self) {
        for byte in &mut self.payload {
            unsafe { core::ptr::write_volatile(byte, 0) }
        }
    }
    fn erase_packet(&mut self) {
        self.packet.erase();
        self.packet_pending = false;
    }
    fn progress(&mut self, now: u64) {
        self.progress_turn = self.turn;
        self.progress_ticks = now;
    }
    fn start(&mut self, ticket: u32, action: Action, now: u64) {
        self.ticket = ticket;
        self.action = action;
        self.len = 0;
        self.offset = 0;
        self.generation = 0;
        self.digest = 0;
        self.admitted = false;
        self.start_turn = self.turn;
        self.progress_turn = self.turn;
        self.clock_turn = self.turn;
        self.start_ticks = now;
        self.progress_ticks = now;
        self.last_ticks = now;
        self.view = View::Pending;
    }
    fn finish(&mut self, view: View) {
        self.erase_payload();
        self.erase_packet();
        self.event = None;
        self.step = Step::Idle;
        self.view = view;
        self.len = 0;
        self.offset = 0;
        self.admitted = false;
    }
    fn stop(&mut self, view: View) {
        let ticket = self.ticket;
        self.finish(view);
        self.event = Some(Event::Stopped { ticket, view });
    }
    fn uncertain(&mut self) {
        self.erase_payload();
        self.erase_packet();
        self.event = Some(Event::Stopped {
            ticket: self.ticket,
            view: View::Uncertain,
        });
        self.len = 0;
        self.offset = 0;
        self.step = Step::Uncertain;
        self.view = View::Uncertain;
    }
    fn clock(&mut self, now: u64) -> bool {
        if !self.step.active() || self.step == Step::Ready || self.step == Step::Uncertain {
            return true;
        }
        if now < self.last_ticks {
            self.uncertain();
            return false;
        }
        if now > self.last_ticks {
            self.clock_turn = self.turn;
            self.last_ticks = now;
        }
        if self.turn.saturating_sub(self.start_turn) >= TOTAL_TURNS
            || now.saturating_sub(self.start_ticks) >= TOTAL_TICKS
            || self.turn.saturating_sub(self.progress_turn) >= STALL_TURNS
            || now.saturating_sub(self.progress_ticks) >= STALL_TICKS
            || self.turn.saturating_sub(self.clock_turn) >= STALL_TURNS
        {
            self.uncertain();
            return false;
        }
        true
    }
    pub fn publish(&mut self, ticket: u32, source: Source, bytes: &[u8], now: u64) -> bool {
        if self.step != Step::Export || ticket != self.ticket || !self.clock(now) {
            return false;
        }
        if !source.valid() || bytes.is_empty() || bytes.len() > MAX_BYTES || !abi::text(bytes) {
            self.decline(
                ticket,
                if bytes.is_empty() {
                    View::Empty
                } else {
                    View::Unsupported
                },
                now,
            );
            return false;
        }
        self.payload[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        self.source = source;
        self.digest = abi::hash(bytes);
        self.step = Step::Begin;
        self.progress(now);
        true
    }
    pub fn read(&mut self, ticket: u32, now: u64) -> bool {
        if self.step != Step::Paste || ticket != self.ticket || !self.clock(now) {
            return false;
        }
        self.step = Step::Read;
        self.progress(now);
        true
    }
    /// Erase a completed handoff after the caller applies it atomically or declines it.
    pub fn settle(&mut self, success: bool) -> bool {
        if self.step != Step::Ready {
            return false;
        }
        self.finish(if success {
            View::Ready
        } else {
            View::Cancelled
        });
        true
    }
    pub fn finish_read(&mut self) -> bool {
        self.settle(true)
    }
    /// Cancellation is a new operation, never rollback or replay of a publication.
    pub fn cancel(&mut self, now: u64) {
        if matches!(self.step, Step::Closed | Step::Idle) {
            return;
        }
        if self.step == Step::Ready {
            self.stop(View::Cancelled);
            return;
        }
        if matches!(self.step, Step::Cancel | Step::CancelAck) {
            return;
        }
        self.erase_payload();
        self.erase_packet();
        self.event = None;
        self.len = 0;
        self.offset = 0;
        // Even an unadmitted intent is retired at the broker before another intent.
        self.step = Step::Cancel;
        self.cancel_view = View::Cancelled;
        self.view = View::Pending;
        self.start_turn = self.turn;
        self.progress_turn = self.turn;
        self.clock_turn = self.turn;
        self.start_ticks = now;
        self.progress_ticks = now;
        self.last_ticks = now;
    }
    /// Decline a Shell intent without publishing any bytes. Retire the issued
    /// ticket before accepting another intent; the requested status stays visible.
    pub fn decline(&mut self, ticket: u32, view: View, now: u64) -> bool {
        if ticket != self.ticket
            || !matches!(self.step, Step::Export | Step::Paste)
            || !matches!(
                view,
                View::Empty | View::Unsupported | View::Cancelled | View::Denied
            )
        {
            return false;
        }
        self.cancel(now);
        self.cancel_view = view;
        self.view = view;
        true
    }
    /// Permanent local fence. The caller's lifecycle close retires broker custody.
    pub fn close(&mut self) {
        self.finish(View::Cancelled);
        self.step = Step::Closed;
    }
    fn request(&self) -> Option<Request> {
        Some(match self.step {
            Step::Begin => Request::Begin {
                ticket: self.ticket,
                len: self.len,
                source: self.source,
            },
            Step::Write => {
                let count = (self.len - self.offset).min(16);
                let mut bytes = [0; 16];
                bytes[..count].copy_from_slice(&self.payload[self.offset..self.offset + count]);
                Request::Write {
                    ticket: self.ticket,
                    offset: self.offset,
                    count,
                    bytes,
                }
            }
            Step::Publish => Request::Publish {
                ticket: self.ticket,
                len: self.len,
                hash: self.digest,
            },
            Step::Read => Request::Read {
                ticket: self.ticket,
            },
            Step::Get => Request::Get {
                ticket: self.ticket,
                offset: self.offset,
            },
            Step::Finish => Request::Finish {
                ticket: self.ticket,
                generation: self.generation,
                hash: self.digest,
            },
            Step::Cancel => Request::Cancel {
                ticket: self.ticket,
            },
            _ => return None,
        })
    }
    /// At most four transport attempts, including Full, per cooperative turn.
    pub fn poll(&mut self, now: u64, mut send: impl FnMut(Packet) -> Admission) -> usize {
        let Some(turn) = self.turn.checked_add(1) else {
            if self.step.active() {
                self.uncertain()
            }
            return 0;
        };
        self.turn = turn;
        if !self.clock(now) {
            return 0;
        }
        let mut attempts = 0;
        while attempts < 4 {
            if !self.packet_pending {
                let Some(request) = self.request() else { break };
                self.packet = request.wire();
                self.packet_pending = true;
            }
            attempts += 1;
            match send(self.packet) {
                Admission::Full => break,
                Admission::Rejected => {
                    if self.admitted {
                        self.uncertain()
                    } else {
                        self.stop(View::Denied)
                    }
                    break;
                }
                Admission::Accepted => {
                    self.erase_packet();
                    self.admitted = true;
                    self.progress(now);
                    self.step = match self.step {
                        Step::Begin => Step::BeginAck,
                        Step::Write => {
                            self.offset += (self.len - self.offset).min(16);
                            if self.offset == self.len {
                                Step::Publish
                            } else {
                                Step::Write
                            }
                        }
                        Step::Publish => Step::PublishAck,
                        Step::Read => Step::ReadAck,
                        Step::Get => Step::Data,
                        Step::Finish => Step::FinishAck,
                        Step::Cancel => Step::CancelAck,
                        _ => {
                            self.uncertain();
                            break;
                        }
                    };
                }
            }
        }
        attempts
    }
    pub fn receive(&mut self, sender: Subject, opcode: u64, args: [u64; 3], now: u64) -> bool {
        if sender != SHELL || self.participant.is_none() || self.step == Step::Closed {
            return false;
        }
        if self.step == Step::Ready && opcode != abi::INTENT {
            return false;
        }
        let Some(response) = Response::decode(opcode, args) else {
            if abi::response_opcode(opcode) && args[0] as u32 == self.ticket && self.step.active() {
                self.uncertain();
                return true;
            }
            return false;
        };
        if let Response::Intent { ticket, action } = response {
            if ticket <= self.latest {
                return false;
            }
            self.latest = ticket;
            if self.step.active() || self.event.is_some() {
                return false;
            }
            self.start(ticket, action, now);
            if !self.participant.unwrap().allows(action) {
                self.step = Step::Export;
                self.decline(ticket, View::Unsupported, now);
                return true;
            }
            match action {
                Action::Copy | Action::Cut => {
                    self.step = Step::Export;
                    self.event = Some(Event::Export {
                        ticket,
                        cut: action == Action::Cut,
                    })
                }
                Action::Paste => {
                    self.step = Step::Paste;
                    self.event = Some(Event::Paste { ticket })
                }
            }
            return true;
        }
        if !self.step.active() || response.ticket() != self.ticket || self.step == Step::Uncertain {
            return false;
        }
        if !self.clock(now) {
            return true;
        }
        // Once FINISH succeeded the bytes have been handed off. A later producer
        // retirement cannot revoke that already completed transfer.
        if self.step == Step::Ready {
            return false;
        }
        if let Response::Meta {
            phase: Phase::Finish,
            status: Status::Invalidated | Status::Uncertain,
            ..
        } = response
        {
            if self.action == Action::Paste && !matches!(self.step, Step::Cancel | Step::CancelAck)
            {
                self.uncertain();
                return true;
            }
        }
        match response {
            Response::Meta {
                phase,
                status,
                len,
                generation,
                hash,
                ..
            } => {
                let expected = match self.step {
                    Step::BeginAck => Phase::Begin,
                    Step::PublishAck => Phase::Publish,
                    Step::ReadAck => Phase::Read,
                    Step::FinishAck => Phase::Finish,
                    Step::CancelAck => Phase::Cancel,
                    _ => return false,
                };
                // A receipt for a previous phase may be duplicated by the peer;
                // it cannot advance a new operation or erase cancellation custody.
                if phase != expected {
                    return false;
                }
                if status != Status::Ok {
                    self.stop(match status {
                        Status::Busy => View::Busy,
                        Status::Empty => View::Empty,
                        Status::Denied => View::Denied,
                        Status::Invalidated | Status::Uncertain => {
                            self.uncertain();
                            return true;
                        }
                        _ => View::Unsupported,
                    });
                    return true;
                }
                let valid = match self.step {
                    Step::BeginAck => len == self.len && generation == 0 && hash == 0,
                    Step::PublishAck => len == self.len && generation != 0 && hash == self.digest,
                    Step::ReadAck => len > 0 && len <= MAX_BYTES && generation != 0,
                    Step::FinishAck => {
                        len == self.len && generation == self.generation && hash == self.digest
                    }
                    Step::CancelAck => len == 0 && generation == 0 && hash == 0,
                    _ => false,
                };
                if !valid {
                    self.uncertain();
                    return true;
                }
                self.progress(now);
                match self.step {
                    Step::BeginAck => {
                        self.offset = 0;
                        self.step = Step::Write
                    }
                    Step::PublishAck => {
                        let event = Event::Published {
                            ticket: self.ticket,
                            cut: self.action == Action::Cut,
                            len: self.len,
                            generation,
                            hash,
                        };
                        self.finish(View::Copied);
                        self.event = Some(event)
                    }
                    Step::ReadAck => {
                        self.len = len;
                        self.generation = generation;
                        self.digest = hash;
                        self.offset = 0;
                        self.step = Step::Get
                    }
                    Step::FinishAck => {
                        self.step = Step::Ready;
                        self.view = View::Ready;
                        self.event = Some(Event::Ready {
                            ticket: self.ticket,
                            len: self.len,
                            generation,
                            hash,
                        })
                    }
                    Step::CancelAck => self.stop(self.cancel_view),
                    _ => {}
                }
            }
            Response::Data {
                offset,
                count,
                bytes,
                ..
            } => {
                if self.step != Step::Data
                    || offset != self.offset
                    || count != (self.len - self.offset).min(16)
                    || !abi::text(&bytes[..count])
                {
                    self.uncertain();
                    return true;
                }
                self.payload[offset..offset + count].copy_from_slice(&bytes[..count]);
                self.offset += count;
                self.progress(now);
                if self.offset == self.len {
                    if abi::hash(&self.payload[..self.len]) != self.digest {
                        self.uncertain()
                    } else {
                        self.step = Step::Finish
                    }
                } else {
                    self.step = Step::Get
                }
            }
            Response::Intent { .. } => unreachable!(),
        }
        true
    }
}

#[cfg(test)]
impl Client {
    pub(crate) fn storage_erased(&self) -> bool {
        self.payload.iter().all(|&b| b == 0)
    }
    pub(crate) fn packet_erased(&self) -> bool {
        self.packet.opcode == 0 && self.packet.args == [0; 3] && !self.packet_pending
    }
}
