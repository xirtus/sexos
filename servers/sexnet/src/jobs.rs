// SPDX-License-Identifier: GPL-3.0-only
//! Private bounded state. The caller authenticates the exact live BOOT identity
//! before every operation/advance. Neither a ticket nor a generation grants access.
use sex_pdx::network_job::{self as abi, Request, State, MAX_BYTES, TURN};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity { pub pd: u32, pub task: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link { Opening, Open, PeerFin, Closed }
pub trait Wire {
    fn begin(&mut self, ticket: u32) -> Result<(), u32>;
    fn receive(&mut self, out: &mut [u8]) -> Result<usize, u32>;
    fn send(&mut self, data: &[u8]) -> Result<usize, u32>;
    fn queued(&self) -> usize;
    fn link(&self) -> Link;
    fn finish_send(&mut self);
    fn abort(&mut self);
}
/// Observation only: `sent` counts bytes queued into the wire, not delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State, pub ticket: u32, pub staged: usize,
    pub sent: usize, pub received: usize,
}
#[derive(Clone, Copy)]
struct Receipt { owner: Identity, request: Request, word: u64 }
pub struct Machine {
    owner: Option<Identity>, ticket: u32, next_ticket: u32, seq: u32,
    state: State, request: [u8; MAX_BYTES], response: [u8; MAX_BYTES],
    request_len: usize, staged: usize, sent: usize, response_len: usize,
    deadline: u64, last_now: u64, finished_send: bool,
    pending: [Option<Receipt>; 6], reply_cursor: usize,
    // One replay fence for each identity in the finite six-app BOOT roster.
    // Entries survive Release and are reclaimed only by exact retirement.
    cached: [Option<Receipt>; 6],
}
impl Machine {
    pub const fn new() -> Self {
        Self { owner: None, ticket: 0, next_ticket: 1, seq: 0, state: State::Idle,
            request: [0; MAX_BYTES], response: [0; MAX_BYTES], request_len: 0,
            staged: 0, sent: 0, response_len: 0, deadline: 0, last_now: 0,
            finished_send: false, pending: [None; 6], reply_cursor: 0, cached: [None; 6] }
    }
    pub fn owner(&self) -> Option<Identity> { self.owner }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { state: self.state, ticket: self.ticket, staged: self.staged,
            sent: self.sent, received: self.response_len }
    }
    pub fn can_transmit(&self, ticket: u32) -> bool {
        self.owner.is_some() && self.ticket == ticket
            && matches!(self.state, State::Connecting | State::Sending | State::Receiving | State::Complete)
    }
    fn pending_slot(&self) -> Option<usize> {
        (0..self.pending.len()).map(|offset| (self.reply_cursor + offset) % self.pending.len())
            .find(|slot| self.pending[*slot].is_some())
    }
    pub fn pending_reply(&self) -> Option<(Identity, u64)> {
        self.pending_slot().and_then(|slot| self.pending[slot]).map(|r| (r.owner, r.word))
    }
    pub fn reply_accepted(&mut self) {
        if let Some(slot) = self.pending_slot() {
            self.pending[slot] = None;
            self.reply_cursor = (slot + 1) % self.pending.len();
        }
    }
    /// Full keeps the selected receipt intact while giving other owners a turn.
    pub fn reply_deferred(&mut self) {
        if let Some(slot) = self.pending_slot() {
            self.reply_cursor = (slot + 1) % self.pending.len();
        }
    }
    fn scrub(&mut self, wire: &mut impl Wire) {
        self.request.fill(0); self.response.fill(0);
        self.request_len = 0; self.staged = 0; self.sent = 0; self.response_len = 0;
        self.finished_send = false; self.deadline = 0; self.last_now = 0;
        wire.abort();
    }
    fn fail(&mut self, wire: &mut impl Wire) { self.scrub(wire); self.state = State::Failed; }
    pub fn retire(&mut self, owner: Identity, wire: &mut impl Wire) -> bool {
        let held = self.owner == Some(owner);
        if held { self.scrub(wire); self.owner = None; self.ticket = 0; self.seq = 0; self.state = State::Idle; }
        for slot in 0..self.cached.len() {
            if self.cached[slot].map(|r| r.owner) == Some(owner) {
                self.cached[slot] = None;
                self.pending[slot] = None;
            }
        }
        held
    }
    /// False means no reply capacity; no bytes, wire state, or receipts change.
    pub fn handle(&mut self, owner: Identity, r: Request, now: u64, wire: &mut impl Wire) -> bool {
        // Reserve a replay-fence slot before applying any operation. There is
        // no eviction: another live identity must never erase an old sequence.
        let slot = match self.cached.iter().position(|c| c.map(|c| c.owner) == Some(owner))
            .or_else(|| self.cached.iter().position(Option::is_none)) {
                Some(slot) => slot, None => return false,
            };
        // Only this owner's outstanding reply applies backpressure to it.
        if let Some(p) = self.pending[slot] { return p.request == r; }
        if let Some(c) = self.cached[slot] {
            if c.request == r { self.pending[slot] = Some(c); return true; }
            if r.seq <= c.request.seq { return self.respond(slot, owner, r, Err(abi::STALE)); }
        }
        // The held watermark additionally fences the current job's sequence.
        if self.owner == Some(owner) && r.seq <= self.seq {
            return self.respond(slot, owner, r, Err(abi::STALE));
        }
        let result = self.apply(owner, r, now, wire);
        if self.owner == Some(owner) && r.seq <= abi::MAX_SEQ { self.seq = self.seq.max(r.seq); }
        self.respond(slot, owner, r, result)
    }
    fn respond(&mut self, slot: usize, owner: Identity, r: Request, value: Result<u32, u32>) -> bool {
        let word = match value { Ok(v) => abi::reply(r.seq, v), Err(e) => abi::error(r.seq, e) };
        let receipt = Receipt { owner, request: r, word };
        self.pending[slot] = Some(receipt);
        // An older/changed-sequence rejection must not regress the replay fence.
        if !self.cached[slot].map(|c| r.seq <= c.request.seq).unwrap_or(false) {
            self.cached[slot] = Some(receipt);
        }
        true
    }
    fn apply(&mut self, owner: Identity, r: Request, now: u64, wire: &mut impl Wire) -> Result<u32, u32> {
        if owner.pd == 0 || owner.task == 0 || !r.valid() { return Err(abi::INVALID); }
        if r.kind == abi::BEGIN {
            if self.owner.is_some() { return Err(abi::BUSY); }
            if self.next_ticket > abi::MAX_TICKET { return Err(abi::EXHAUSTED); }
            self.ticket = self.next_ticket;
            self.next_ticket = self.next_ticket.checked_add(1).ok_or(abi::EXHAUSTED)?;
            self.owner = Some(owner); self.seq = r.seq; self.state = State::Staging;
            self.request_len = r.length as usize;
            return Ok(self.ticket);
        }
        if self.owner != Some(owner) { return Err(abi::DENIED); }
        if r.ticket != self.ticket { return Err(abi::STALE); }
        match r.kind {
            abi::WRITE => {
                if self.state != State::Staging { return Err(abi::BUSY); }
                let end = self.staged.checked_add(r.length as usize).ok_or(abi::INVALID)?;
                if r.offset as usize != self.staged || end > self.request_len { return Err(abi::INVALID); }
                self.request[self.staged..end].copy_from_slice(&r.a.to_le_bytes()[..r.length as usize]);
                self.staged = end; Ok(end as u32)
            }
            abi::START => {
                if self.state != State::Staging || self.staged != self.request_len { return Err(abi::BUSY); }
                let deadline = match now.checked_add(5_000) { Some(d) => d,
                    None => { self.fail(wire); return Err(abi::TIMEOUT); } };
                // The empty pending slot was reserved by handle's initial check.
                self.deadline = deadline; self.last_now = now; self.state = State::Connecting;
                if let Err(e) = wire.begin(self.ticket) { self.fail(wire); return Err(e); }
                Ok(0)
            }
            abi::STATUS => Ok(abi::status(self.state, self.response_len)),
            abi::READ => {
                if self.state != State::Complete { return Err(abi::BUSY); }
                let start = r.offset as usize; let end = start + r.length as usize;
                if end > self.response_len { return Err(abi::INVALID); }
                let mut bytes = [0u8; 4]; bytes[..r.length as usize].copy_from_slice(&self.response[start..end]);
                Ok(u32::from_le_bytes(bytes))
            }
            abi::CANCEL => { self.scrub(wire); self.state = State::Cancelled; Ok(0) }
            abi::RELEASE => {
                self.scrub(wire); self.owner = None; self.ticket = 0; self.seq = 0; self.state = State::Idle; Ok(0)
            }
            _ => Err(abi::INVALID),
        }
    }
    /// At most TURN received bytes, or (only when receive yields zero) TURN sent.
    pub fn advance(&mut self, now: u64, wire: &mut impl Wire) {
        if !matches!(self.state, State::Connecting | State::Sending | State::Receiving) { return; }
        if now < self.last_now || now >= self.deadline { self.fail(wire); return; }
        self.last_now = now;
        if wire.link() == Link::Closed { self.fail(wire); return; }
        let room = (MAX_BYTES - self.response_len).min(TURN);
        let received = if room != 0 {
            wire.receive(&mut self.response[self.response_len..self.response_len + room])
        } else {
            // A full response still needs one bounded probe to distinguish EOF
            // from overflow; the extra byte is never retained or exposed.
            wire.receive(&mut [0u8; 1])
        };
        let n = match received { Ok(n) if n <= room && room != 0 => n,
            Ok(0) => 0, _ => { self.fail(wire); return; } };
        self.response_len += n;
        if wire.link() == Link::Closed { self.fail(wire); return; }
        if wire.link() == Link::PeerFin {
            if self.sent != self.request_len || wire.queued() != 0 { self.fail(wire); return; }
            if !self.finished_send { wire.finish_send(); self.finished_send = true; }
            if n == 0 { self.state = State::Complete; }
            return;
        }
        if n == 0 && wire.link() == Link::Open && self.sent < self.request_len {
            self.state = State::Sending;
            let end = (self.sent + TURN).min(self.request_len);
            match wire.send(&self.request[self.sent..end]) {
                Ok(sent) if sent <= end - self.sent => self.sent += sent,
                _ => { self.fail(wire); return; }
            }
        }
        if self.sent == self.request_len && wire.queued() == 0 && !self.finished_send {
            wire.finish_send(); self.finished_send = true; self.state = State::Receiving;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;
    const A: Identity = Identity { pd: 7, task: 70 };
    const B: Identity = Identity { pd: 7, task: 71 };
    struct TestWire { link: Link, incoming: Vec<u8>, sent: Vec<u8>, receive_at: usize,
        send_limit: usize, receive_limit: usize, queued: usize, begins: usize,
        aborts: usize, finishes: usize, begin_error: Option<u32> }
    impl TestWire {
        fn new() -> Self { Self { link: Link::Open, incoming: Vec::new(), sent: Vec::new(),
            receive_at: 0, send_limit: TURN, receive_limit: TURN, queued: 0,
            begins: 0, aborts: 0, finishes: 0, begin_error: None } }
    }
    impl Wire for TestWire {
        fn begin(&mut self, _: u32) -> Result<(), u32> { self.begins += 1; self.begin_error.map_or(Ok(()), Err) }
        fn receive(&mut self, out: &mut [u8]) -> Result<usize, u32> {
            assert!(out.len() <= TURN);
            let n = out.len().min(self.receive_limit).min(self.incoming.len() - self.receive_at);
            out[..n].copy_from_slice(&self.incoming[self.receive_at..self.receive_at+n]);
            self.receive_at += n; Ok(n)
        }
        fn send(&mut self, data: &[u8]) -> Result<usize, u32> {
            assert!(data.len() <= TURN); let n = data.len().min(self.send_limit);
            self.sent.extend_from_slice(&data[..n]); Ok(n)
        }
        fn queued(&self) -> usize { self.queued }
        fn link(&self) -> Link { self.link }
        fn finish_send(&mut self) { self.finishes += 1; }
        fn abort(&mut self) { self.aborts += 1; }
    }
    fn req(seq: u32, kind: u8, ticket: u32) -> Request {
        Request { seq, kind, ticket, offset: 0, length: 0, a: 0 }
    }
    fn call(m: &mut Machine, w: &mut TestWire, who: Identity, r: Request) -> Result<u32,u32> {
        assert!(m.handle(who, r, 10, w));
        let (owner, word) = m.pending_reply().unwrap(); assert_eq!(owner, who);
        let result = abi::decode_reply(word, r.seq); m.reply_accepted(); result
    }
    fn stage(m: &mut Machine, w: &mut TestWire, bytes: usize) -> u32 {
        let mut begin = req(1, abi::BEGIN, 0); begin.length = bytes as u16;
        let ticket = call(m,w,A,begin).unwrap();
        for offset in (0..bytes).step_by(8) {
            let mut r = req(2 + (offset / 8) as u32, abi::WRITE, ticket);
            r.offset = offset as u16; r.length = (bytes-offset).min(8) as u16;
            r.a = u64::MAX >> ((8-r.length) * 8);
            assert_eq!(call(m,w,A,r), Ok((offset+r.length as usize) as u32));
        }
        ticket
    }
    fn start(m: &mut Machine, w: &mut TestWire, bytes: usize) -> u32 {
        let ticket = stage(m,w,bytes);
        assert_eq!(call(m,w,A,req(1000,abi::START,ticket)),Ok(0)); ticket
    }
    fn scrubbed(m: &Machine) { assert!(m.request.iter().all(|b| *b == 0)); assert!(m.response.iter().all(|b| *b == 0)); }
    #[test] fn abi_registers_and_canonical_bytes() {
        let mut r = req(abi::MAX_SEQ,abi::WRITE,abi::MAX_TICKET); r.offset=4095; r.length=1; r.a=0xFF;
        assert_eq!(Request::decode(r.encode()),Ok(r)); assert!(r.valid());
        r.a=0x100; assert!(!r.valid()); r.length=8; assert!(r.valid());
        let mut args=r.encode(); args[1]|=1<<32; assert_eq!(Request::decode(args),Err(abi::INVALID));
        args=r.encode(); args[0]&=!0xFF_FFFF; assert_eq!(Request::decode(args),Err(abi::INVALID));
        assert_eq!(abi::decode_reply(abi::reply(2,0xFFFF_FFFF),2),Ok(0xFFFF_FFFF));
        assert_eq!(abi::decode_reply(abi::reply(2,5),3),Err(abi::INVALID));
        assert_eq!(abi::decode_status(abi::status(State::Complete,4096)),Some((State::Complete,4096)));
    }
    #[test] fn identity_ticket_and_sequence_are_isolated() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=stage(&mut m,&mut w,1);
        assert_eq!(call(&mut m,&mut w,B,req(5,abi::CANCEL,t)),Err(abi::DENIED));
        assert_eq!(call(&mut m,&mut w,A,req(5,abi::CANCEL,t+1)),Err(abi::STALE));
        assert_eq!(w.aborts,0);
        assert_eq!(call(&mut m,&mut w,A,req(4,abi::CANCEL,t)),Err(abi::STALE));
        assert_eq!(call(&mut m,&mut w,A,req(6,abi::STATUS,t)),Ok(abi::status(State::Staging,0)));
        let mut changed=req(6,abi::CANCEL,t);
        assert_eq!(call(&mut m,&mut w,A,changed),Err(abi::STALE));
        changed.seq=7; assert_eq!(call(&mut m,&mut w,A,changed),Ok(0)); assert_eq!(w.aborts,1);
    }
    #[test] fn pending_full_has_no_effect_and_duplicate_start_does_not_reconnect() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=stage(&mut m,&mut w,8);
        let r=req(3,abi::START,t); assert!(m.handle(A,r,10,&mut w));
        let receipt=m.pending_reply(); assert_eq!(w.begins,1);
        assert!(m.handle(A,r,10,&mut w)); assert!(!m.handle(A,req(4,abi::CANCEL,t),10,&mut w));
        assert!(m.handle(B,r,10,&mut w)); assert_eq!(w.aborts,0);
        // Prior accepted A writes left the fair cursor at B's newly filled slot.
        assert_eq!(m.pending_reply(),Some((B,abi::error(r.seq,abi::DENIED))));
        m.reply_deferred(); assert_eq!(m.pending_reply(),receipt);
        m.reply_accepted();
        assert_eq!(m.pending_reply(),Some((B,abi::error(r.seq,abi::DENIED)))); m.reply_accepted();
        assert!(m.handle(A,r,11,&mut w)); assert_eq!(w.begins,1); assert_eq!(m.pending_reply(),receipt);
    }
    #[test] fn bounded_partial_io_and_explicit_fin_completion() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=start(&mut m,&mut w,600);
        w.send_limit=99; m.advance(11,&mut w); assert_eq!(w.sent.len(),99);
        for now in 12..20 { m.advance(now,&mut w); }
        assert_eq!(w.sent.len(),600); assert_eq!(w.finishes,1); assert_eq!(m.state,State::Receiving);
        w.incoming=vec![0x41;600]; w.receive_limit=101;
        m.advance(20,&mut w); assert_eq!(m.response_len,101);
        for now in 21..27 { m.advance(now,&mut w); }
        assert_eq!(m.response_len,600); assert_ne!(m.state,State::Complete);
        w.link=Link::PeerFin; m.advance(28,&mut w); assert_eq!(m.state,State::Complete);
        let mut r=req(1001,abi::READ,t); r.offset=598; r.length=2;
        assert_eq!(call(&mut m,&mut w,A,r),Ok(0x4141)); assert!(m.can_transmit(t));
        r.seq+=1; r.offset=599; assert_eq!(call(&mut m,&mut w,A,r),Err(abi::INVALID));
    }
    #[test] fn receive_has_priority_and_queue_must_drain() {
        let mut m=Machine::new(); let mut w=TestWire::new(); start(&mut m,&mut w,8);
        w.incoming=vec![1;300]; m.advance(11,&mut w); assert_eq!(m.response_len,256); assert!(w.sent.is_empty());
        m.advance(12,&mut w); assert_eq!(m.response_len,300); assert!(w.sent.is_empty());
        w.queued=8; m.advance(13,&mut w); assert_eq!(w.sent.len(),8); assert_eq!(w.finishes,0);
        w.queued=0; m.advance(14,&mut w); assert_eq!(w.finishes,1);
    }
    #[test] fn early_fin_rst_timeout_and_clock_rollback_fail_scrubbed() {
        for mode in 0..4 {
            let mut m=Machine::new(); let mut w=TestWire::new(); let t=start(&mut m,&mut w,8);
            m.response[4000]=7;
            let now=match mode { 0=>{w.link=Link::PeerFin;11},1=>{w.link=Link::Closed;11},2=>5010,_=>9 };
            m.advance(now,&mut w); assert_eq!(m.state,State::Failed); assert!(!m.can_transmit(t)); scrubbed(&m);
            let begins=w.begins; m.advance(6000,&mut w); assert_eq!(w.begins,begins);
        }
    }
    #[test] fn fin_requires_receive_drained_and_all_bytes_sent() {
        let mut m=Machine::new(); let mut w=TestWire::new(); start(&mut m,&mut w,1);
        m.advance(11,&mut w); w.incoming=vec![2;300]; w.link=Link::PeerFin;
        m.advance(12,&mut w); assert_eq!(m.state,State::Receiving); assert_eq!(m.response_len,256);
        m.advance(13,&mut w); assert_eq!(m.response_len,300); assert_ne!(m.state,State::Complete);
        m.advance(14,&mut w); assert_eq!(m.state,State::Complete);
    }
    #[test] fn response_overflow_and_begin_failure_scrub() {
        let mut m=Machine::new(); let mut w=TestWire::new(); start(&mut m,&mut w,1); m.advance(11,&mut w);
        w.incoming=vec![5;MAX_BYTES+1]; for now in 12..30 { m.advance(now,&mut w); }
        assert_eq!(m.state,State::Failed); scrubbed(&m);
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=stage(&mut m,&mut w,1); w.begin_error=Some(abi::OFFLINE);
        assert_eq!(call(&mut m,&mut w,A,req(3,abi::START,t)),Err(abi::OFFLINE)); assert_eq!(m.state,State::Failed); scrubbed(&m);
    }
    #[test] fn cancel_release_retire_scrub_full_arrays_and_drop_dead_receipts() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=start(&mut m,&mut w,8);
        m.request[MAX_BYTES-1]=9; m.response[MAX_BYTES-1]=8;
        assert_eq!(call(&mut m,&mut w,A,req(1001,abi::CANCEL,t)),Ok(0)); scrubbed(&m);
        assert_eq!(m.owner(),Some(A)); assert!(!m.can_transmit(t)); assert_eq!(m.state,State::Cancelled);
        assert_eq!(call(&mut m,&mut w,A,req(1002,abi::RELEASE,t)),Ok(0)); assert_eq!(m.owner(),None);
        let mut begin=req(1,abi::BEGIN,0); begin.length=1; assert!(m.handle(B,begin,20,&mut w));
        assert_eq!(m.ticket,t+1); scrubbed(&m); assert!(!m.retire(A,&mut w)); assert_eq!(m.owner(),Some(B));
        m.request[MAX_BYTES-1]=1; m.response[MAX_BYTES-1]=2;
        assert!(m.retire(B,&mut w)); scrubbed(&m); assert_eq!(m.pending_reply(),None); assert!(m.cached.iter().all(Option::is_none));
        assert!(!m.can_transmit(t+1));
    }
    #[test] fn opening_blocks_send_and_deadline_overflow_scrubs() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=stage(&mut m,&mut w,1);
        assert!(m.handle(A,req(3,abi::START,t),u64::MAX,&mut w));
        assert_eq!(abi::decode_reply(m.pending_reply().unwrap().1,3),Err(abi::TIMEOUT));
        assert_eq!(w.begins,0); assert_eq!(m.state,State::Failed); scrubbed(&m);
        let mut m=Machine::new(); let mut w=TestWire::new(); start(&mut m,&mut w,1);
        w.link=Link::Opening; m.advance(11,&mut w); assert!(w.sent.is_empty());
        assert_eq!(m.snapshot(),Snapshot { state:State::Connecting,ticket:1,staged:1,sent:0,received:0 });
    }
    #[test] fn rejected_older_sequence_cannot_lower_release_fence() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=stage(&mut m,&mut w,1);
        assert_eq!(call(&mut m,&mut w,A,req(10,abi::RELEASE,t)),Ok(0));
        assert_eq!(call(&mut m,&mut w,A,req(3,abi::CANCEL,t)),Err(abi::STALE));
        let mut begin=req(4,abi::BEGIN,0); begin.length=1;
        assert_eq!(call(&mut m,&mut w,A,begin),Err(abi::STALE)); assert_eq!(m.owner(),None);
    }
    #[test] fn finite_tickets_never_wrap_or_reuse() {
        let mut m=Machine::new(); let mut w=TestWire::new(); m.next_ticket=abi::MAX_TICKET;
        let mut r=req(1,abi::BEGIN,0); r.length=1;
        assert_eq!(call(&mut m,&mut w,A,r),Ok(abi::MAX_TICKET));
        assert_eq!(call(&mut m,&mut w,A,req(2,abi::RELEASE,abi::MAX_TICKET)),Ok(0));
        r.seq=3; assert_eq!(call(&mut m,&mut w,A,r),Err(abi::EXHAUSTED)); assert_eq!(m.owner(),None);
        assert_eq!(48_000+abi::MAX_TICKET,65_535);
    }
    #[test] fn release_receipt_replays_without_releasing_next_owner() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let t=stage(&mut m,&mut w,1);
        let release=req(3,abi::RELEASE,t); assert_eq!(call(&mut m,&mut w,A,release),Ok(0));
        assert_eq!(call(&mut m,&mut w,A,release),Ok(0)); assert_eq!(w.aborts,1);
        let mut r=req(1,abi::BEGIN,0); r.length=1; assert_eq!(call(&mut m,&mut w,B,r),Ok(t+1));
        assert_eq!(call(&mut m,&mut w,A,release),Ok(0)); assert_eq!(m.owner(),Some(B)); assert_eq!(w.aborts,1);
    }
    #[test] fn foreign_receipts_cannot_erase_released_owner_sequence_fence() {
        let mut m=Machine::new(); let mut w=TestWire::new();
        let mut begin=req(1,abi::BEGIN,0); begin.length=1;
        let t=call(&mut m,&mut w,A,begin).unwrap();
        assert_eq!(call(&mut m,&mut w,A,req(2,abi::RELEASE,t)),Ok(0));
        assert_eq!(call(&mut m,&mut w,B,req(1,abi::STATUS,t)),Err(abi::DENIED));
        assert_eq!(call(&mut m,&mut w,A,begin),Err(abi::STALE));
        assert_eq!(m.owner(),None); assert_eq!(m.next_ticket,t+1); assert_eq!(w.aborts,1);
        assert_eq!(call(&mut m,&mut w,B,begin),Err(abi::STALE));
        begin.seq=3;
        assert_eq!(call(&mut m,&mut w,B,begin),Ok(t+1));
        begin.seq=1;
        assert_eq!(call(&mut m,&mut w,A,begin),Err(abi::STALE));
        assert_eq!(m.owner(),Some(B)); assert_eq!(m.next_ticket,t+2);
    }
    #[test] fn replay_cache_exhaustion_has_no_effect_and_only_exact_retire_reclaims() {
        let mut m=Machine::new(); let mut w=TestWire::new();
        for n in 0..6 {
            let owner=Identity { pd:100+n,task:1000+n };
            assert_eq!(call(&mut m,&mut w,owner,req(1,abi::STATUS,1)),Err(abi::DENIED));
        }
        let snapshot=m.snapshot(); let next_ticket=m.next_ticket;
        let mut begin=req(1,abi::BEGIN,0); begin.length=1;
        assert!(!m.handle(A,begin,10,&mut w));
        assert_eq!(m.snapshot(),snapshot); assert_eq!(m.next_ticket,next_ticket);
        assert_eq!(m.pending_reply(),None); assert_eq!(w.begins,0); assert_eq!(w.aborts,0);
        assert!(!m.retire(Identity { pd:100,task:999 },&mut w));
        assert!(!m.handle(A,begin,10,&mut w));
        // Existing owners retain replay capacity even when every slot is held.
        let prior=Identity { pd:100,task:1000 };
        assert_eq!(call(&mut m,&mut w,prior,req(1,abi::STATUS,1)),Err(abi::DENIED));
        assert!(!m.retire(prior,&mut w));
        assert_eq!(call(&mut m,&mut w,A,begin),Ok(next_ticket));
        assert_eq!(m.cached.iter().filter(|c| c.is_some()).count(),6);
        assert_eq!(w.aborts,0);
    }

    #[test] fn blocked_owner_does_not_drop_or_starve_other_owners_busy_receipt() {
        let mut m=Machine::new(); let mut w=TestWire::new();
        let mut begin=req(1,abi::BEGIN,0); begin.length=1;
        assert!(m.handle(A,begin,10,&mut w));
        let receipt_a=m.pending_reply().unwrap(); assert_eq!(receipt_a,(A,abi::reply(1,1)));
        assert!(m.handle(B,begin,10,&mut w));
        let receipt_b=(B,abi::error(1,abi::BUSY));
        assert_eq!(m.pending_reply(),Some(receipt_a));
        assert!(m.handle(A,begin,10,&mut w)); assert!(m.handle(B,begin,10,&mut w));
        assert!(!m.handle(A,req(2,abi::CANCEL,1),10,&mut w));
        assert!(!m.handle(B,req(2,abi::STATUS,1),10,&mut w));
        assert_eq!(m.pending_reply(),Some(receipt_a));
        m.reply_deferred(); assert_eq!(m.pending_reply(),Some(receipt_b));
        m.reply_accepted(); assert_eq!(m.pending_reply(),Some(receipt_a));
        assert_eq!(m.owner(),Some(A)); assert_eq!(m.next_ticket,2); assert_eq!(w.aborts,0);
        m.reply_accepted(); assert_eq!(m.pending_reply(),None);
        // Replaying B's exact BUSY receipt remains independent of A's receipt.
        assert!(m.handle(B,begin,11,&mut w)); assert_eq!(m.pending_reply(),Some(receipt_b));
        m.reply_accepted();
        assert!(m.handle(A,begin,11,&mut w)); assert_eq!(m.pending_reply(),Some(receipt_a));
    }
    #[test] fn retire_removes_only_exact_owners_pending_and_cached_receipts() {
        let mut m=Machine::new(); let mut w=TestWire::new();
        let mut begin=req(1,abi::BEGIN,0); begin.length=1;
        assert!(m.handle(A,begin,10,&mut w)); assert!(m.handle(B,begin,10,&mut w));
        let receipt_b=(B,abi::error(1,abi::BUSY));
        assert!(!m.retire(Identity { pd:A.pd,task:A.task+100 },&mut w));
        assert_eq!(m.pending_reply(),Some((A,abi::reply(1,1))));
        assert!(m.retire(A,&mut w)); assert_eq!(m.pending_reply(),Some(receipt_b));
        assert_eq!(m.owner(),None); assert_eq!(w.aborts,1); scrubbed(&m);
        assert_eq!(m.cached.iter().filter(|r| r.is_some()).count(),1);
        assert!(m.handle(B,begin,10,&mut w)); assert_eq!(m.pending_reply(),Some(receipt_b));
        m.reply_accepted();
        assert!(m.handle(B,begin,11,&mut w)); assert_eq!(m.pending_reply(),Some(receipt_b));
        assert!(!m.retire(B,&mut w)); assert_eq!(m.pending_reply(),None);
        assert!(m.cached.iter().all(Option::is_none));
    }

}
