// SPDX-License-Identifier: GPL-3.0-only
//! Private bounded state. The caller authenticates the exact live BOOT identity
//! before every operation/advance. Neither a ticket nor a generation grants access.
use sex_pdx::network_job::{self as abi, Request, State, MAX_BYTES, TURN};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity { pub pd: u32, pub task: u32 }
/// Copied DNS observation admitted by the parent under its fixed endpoint policy.
/// Freshness is checked at BEGIN; this data cannot grant a route or redirect it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub owner: Identity, pub generation: u32, pub address: [u8; 4],
    pub observed_tick: u64, pub expires_tick: u64,
}
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
/// Finite private byte custody only, never a delivery or durability receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSnapshot { pub produced: usize, pub consumed: usize, pub buffered: usize }
#[derive(Clone, Copy)]
struct Receipt { owner: Identity, request: Request, word: u64 }
pub struct Machine {
    owner: Option<Identity>, ticket: u32, next_ticket: u32, seq: u32,
    state: State, request: [u8; MAX_BYTES], response: [u8; MAX_BYTES],
    request_len: usize, staged: usize, sent: usize, response_len: usize,
    deadline: u64, last_now: u64, finished_send: bool,
    streaming: bool, duplex: bool, finish_requested: bool, consumed: usize, idle_deadline: u64,
    pending: [Option<Receipt>; 6], reply_cursor: usize,
    // One replay fence for each identity in the finite six-app BOOT roster.
    // Entries survive Release and are reclaimed only by exact retirement.
    cached: [Option<Receipt>; 6],
    resolution: Option<Resolution>,
}
impl Machine {
    pub const fn new() -> Self {
        Self { owner: None, ticket: 0, next_ticket: 1, seq: 0, state: State::Idle,
            request: [0; MAX_BYTES], response: [0; MAX_BYTES], request_len: 0,
            staged: 0, sent: 0, response_len: 0, deadline: 0, last_now: 0,
            finished_send: false, streaming: false, duplex: false, finish_requested: false, consumed: 0, idle_deadline: 0,
            pending: [None; 6], reply_cursor: 0, cached: [None; 6], resolution: None }
    }
    pub fn is_duplex(&self) -> bool { self.duplex }
    pub fn finish_requested(&self) -> bool { self.finish_requested }
    pub fn owner(&self) -> Option<Identity> { self.owner }
    pub fn resolution(&self) -> Option<Resolution> { self.resolution }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { state: self.state, ticket: self.ticket, staged: self.staged,
            sent: self.sent, received: self.response_len }
    }
    pub fn stream_snapshot(&self) -> Option<StreamSnapshot> {
        self.streaming.then_some(StreamSnapshot { produced: self.response_len,
            consumed: self.consumed, buffered: self.response_len - self.consumed })
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
        self.streaming = false; self.duplex = false; self.finish_requested = false; self.consumed = 0; self.idle_deadline = 0;
        self.resolution = None;
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
        self.handle_exclusive(owner,r,now,false,wire)
    }
    /// Another held protocol job forbids new BEGIN effects through this route.
    pub fn handle_exclusive(&mut self, owner: Identity, r: Request, now: u64, other_held: bool, wire: &mut impl Wire) -> bool {
        self.handle_resolved(owner, r, now, other_held, Err(abi::DENIED), wire)
    }
    /// Admission denials use the same retained sequence fence as successes.
    /// Duplicate requests replay their original receipt without re-admission.
    pub fn handle_resolved(&mut self, owner: Identity, r: Request, now: u64, other_held: bool,
        admission: Result<Resolution, u32>, wire: &mut impl Wire) -> bool {
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
        let result = if other_held && matches!(r.kind, abi::BEGIN | abi::BEGIN_RESOLVED | abi::BEGIN_STREAM_RESOLVED | abi::BEGIN_DUPLEX_RESOLVED) { Err(abi::BUSY) }
            else { self.apply(owner, r, now, admission, wire) };
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
    fn apply(&mut self, owner: Identity, r: Request, now: u64, admission: Result<Resolution,u32>, wire: &mut impl Wire) -> Result<u32, u32> {
        if owner.pd == 0 || owner.task == 0 || !r.valid() { return Err(abi::INVALID); }
        if matches!(r.kind, abi::BEGIN | abi::BEGIN_RESOLVED | abi::BEGIN_STREAM_RESOLVED | abi::BEGIN_DUPLEX_RESOLVED) {
            if self.owner.is_some() { return Err(abi::BUSY); }
            if self.next_ticket > abi::MAX_TICKET { return Err(abi::EXHAUSTED); }
            let resolution = if matches!(r.kind, abi::BEGIN_RESOLVED | abi::BEGIN_STREAM_RESOLVED | abi::BEGIN_DUPLEX_RESOLVED) {
                let record = admission?;
                if record.owner != owner || record.generation as u64 != r.a
                    || now < record.observed_tick || now >= record.expires_tick { return Err(abi::STALE); }
                Some(record)
            } else { None };
            self.ticket = self.next_ticket;
            self.next_ticket = self.next_ticket.checked_add(1).ok_or(abi::EXHAUSTED)?;
            self.owner = Some(owner); self.seq = r.seq; self.state = State::Staging;
            self.request_len = r.length as usize;
            self.duplex = r.kind == abi::BEGIN_DUPLEX_RESOLVED;
            self.streaming = r.kind == abi::BEGIN_STREAM_RESOLVED || self.duplex;
            self.resolution = resolution;
            return Ok(self.ticket);
        }
        if self.owner != Some(owner) { return Err(abi::DENIED); }
        if r.ticket != self.ticket { return Err(abi::STALE); }
        // A fresh authenticated operation cannot extend an expired stream by
        // draining old data. Cleanup remains possible after any clock failure.
        if self.streaming && matches!(self.state, State::Connecting | State::Sending | State::Receiving)
            && !matches!(r.kind, abi::CANCEL | abi::RELEASE) {
            if self.expired(now) { self.fail(wire); return Err(abi::TIMEOUT); }
            self.last_now = now;
        }
        match r.kind {
            abi::WRITE => {
                if self.duplex && self.state != State::Staging {
                    if !matches!(self.state,State::Connecting|State::Sending|State::Receiving)||self.finish_requested{return Err(abi::BUSY);}
                    let end=self.staged.checked_add(r.length as usize).ok_or(abi::INVALID)?;
                    if r.offset as usize!=self.staged||end>abi::MAX_DUPLEX_BYTES{return Err(abi::INVALID);}
                    if end-self.sent>MAX_BYTES{return Err(abi::BUSY);}
                    for (i,byte) in r.a.to_le_bytes()[..r.length as usize].iter().enumerate(){self.request[(self.staged+i)%MAX_BYTES]=*byte;}
                    self.staged=end;return Ok(end as u32);
                }
                if self.state != State::Staging { return Err(abi::BUSY); }
                let end = self.staged.checked_add(r.length as usize).ok_or(abi::INVALID)?;
                if r.offset as usize != self.staged || end > self.request_len { return Err(abi::INVALID); }
                self.request[self.staged..end].copy_from_slice(&r.a.to_le_bytes()[..r.length as usize]);
                self.staged = end; Ok(end as u32)
            }
            abi::START => {
                if self.state != State::Staging || self.staged != self.request_len { return Err(abi::BUSY); }
                let duration = if self.streaming { abi::STREAM_TOTAL_TICKS } else { 5_000 };
                let deadline = match now.checked_add(duration) { Some(d) => d,
                    None => { self.fail(wire); return Err(abi::TIMEOUT); } };
                // The empty pending slot was reserved by handle's initial check.
                self.deadline = deadline; self.last_now = now; self.state = State::Connecting;
                if self.streaming { self.stream_progress(now); }
                if let Err(e) = wire.begin(self.ticket) { self.fail(wire); return Err(e); }
                Ok(0)
            }
            abi::STATUS => Ok(abi::status(self.state, self.response_len)),
            abi::READ => {
                if self.streaming {
                    if !(matches!(self.state, State::Receiving | State::Complete)||self.duplex&&matches!(self.state,State::Connecting|State::Sending)) { return Err(abi::BUSY); }
                    if r.offset as usize != self.consumed { return Err(abi::INVALID); }
                    let end = self.consumed.checked_add(r.length as usize).ok_or(abi::INVALID)?;
                    if end > self.response_len { return Err(abi::BUSY); }
                    // handle reserved this owner's pending receipt before apply.
                    // Its copied word survives Full and exact duplicate replay,
                    // even after these ring cells are scrubbed/reused.
                    let mut bytes = [0u8; 4];
                    for (index, byte) in bytes[..r.length as usize].iter_mut().enumerate() {
                        let cell = (self.consumed + index) % abi::STREAM_RING_BYTES;
                        *byte = self.response[cell]; self.response[cell] = 0;
                    }
                    self.consumed = end;
                    self.stream_progress(now);
                    return Ok(u32::from_le_bytes(bytes));
                }
                if self.state != State::Complete { return Err(abi::BUSY); }
                let start = r.offset as usize; let end = start + r.length as usize;
                if end > self.response_len { return Err(abi::INVALID); }
                let mut bytes = [0u8; 4]; bytes[..r.length as usize].copy_from_slice(&self.response[start..end]);
                Ok(u32::from_le_bytes(bytes))
            }
            abi::FINISH => {
                if !self.duplex||!matches!(self.state,State::Connecting|State::Sending|State::Receiving|State::Complete){return Err(abi::BUSY);}
                self.finish_requested=true;Ok(0)
            }
            abi::CANCEL => { self.scrub(wire); self.state = State::Cancelled; Ok(0) }
            abi::RELEASE => {
                self.scrub(wire); self.owner = None; self.ticket = 0; self.seq = 0; self.state = State::Idle; Ok(0)
            }
            _ => Err(abi::INVALID),
        }
    }
    fn expired(&self, now: u64) -> bool {
        now < self.last_now || now >= self.deadline || self.streaming && now >= self.idle_deadline
    }
    /// Only actual send/receive/consumption refreshes idle time. STATUS, rejected
    /// operations, cached duplicates and Full admission never count as progress.
    /// The absolute total bound never moves; Complete holds data until cleanup.
    fn stream_progress(&mut self, now: u64) {
        self.idle_deadline = now.checked_add(abi::STREAM_IDLE_TICKS).unwrap_or(self.deadline).min(self.deadline);
    }
    /// A full ring is backpressure, not an overflow or an observed EOF. Only the
    /// independent cumulative wire limit permits a one-byte overflow/EOF probe.
    fn stream_receive(&mut self, wire: &mut impl Wire) -> Result<(usize, bool), ()> {
        self.stream_receive_bounded(wire,TURN)
    }
    fn stream_receive_bounded(&mut self, wire:&mut impl Wire,budget:usize)->Result<(usize,bool),()> {
        let maximum=if self.duplex{abi::MAX_DUPLEX_BYTES}else{abi::MAX_STREAM_BYTES};
        if self.response_len == maximum {
            return match wire.receive(&mut [0u8; 1]) { Ok(0) => Ok((0, true)), _ => Err(()) };
        }
        let occupied = self.response_len - self.consumed;
        let at = self.response_len % abi::STREAM_RING_BYTES;
        let room = (abi::STREAM_RING_BYTES - occupied).min(abi::STREAM_RING_BYTES - at)
            .min(budget).min(maximum - self.response_len);
        if room == 0 { return Ok((0, false)); }
        match wire.receive(&mut self.response[at..at+room]) {
            Ok(n) if n <= room => { self.response_len += n; Ok((n, n == 0)) },
            _ => Err(()),
        }
    }
    /// Duplex turns share TURN bytes across both directions. Receive gets at
    /// most half; available TX always gets the remaining budget even under a
    /// continuous RX stream. A full RX ring never masquerades as an EOF.
    fn advance_duplex(&mut self,now:u64,wire:&mut impl Wire){
        let (received,drained)=match self.stream_receive_bounded(wire,TURN/2){Ok(v)=>v,Err(())=>{self.fail(wire);return;}};
        if received!=0{self.stream_progress(now);}
        let link=wire.link();if link==Link::Closed{self.fail(wire);return;}
        if matches!(link,Link::Open|Link::PeerFin)&&self.sent<self.staged {
            let at=self.sent%MAX_BYTES;let count=(self.staged-self.sent).min(MAX_BYTES-at).min(TURN-received);
            match wire.send(&self.request[at..at+count]){
                Ok(n)if n<=count=>{self.request[at..at+n].fill(0);self.sent+=n;if n!=0{self.stream_progress(now);}},
                _=>{self.fail(wire);return;}
            }
        }
        if self.finish_requested&&self.sent==self.staged&&wire.queued()==0&&!self.finished_send{
            wire.finish_send();self.finished_send=true;
        }
        if self.finished_send&&wire.link()==Link::PeerFin&&drained&&self.consumed==self.response_len&&wire.queued()==0{self.state=State::Complete;}
        else if wire.link()!=Link::Opening{self.state=if self.sent<self.staged||wire.queued()!=0{State::Sending}else{State::Receiving};}
    }
    /// Legacy modes retain receive priority and implicit one-request FIN. Only
    /// explicit duplex polls both directions and requires caller FINISH.
    pub fn advance(&mut self, now: u64, wire: &mut impl Wire) {
        if !matches!(self.state, State::Connecting | State::Sending | State::Receiving) { return; }
        if self.expired(now) { self.fail(wire); return; }
        self.last_now = now;
        if wire.link() == Link::Closed { self.fail(wire); return; }
        if self.duplex{self.advance_duplex(now,wire);return;}
        let (n, drained) = if self.streaming {
            match self.stream_receive(wire) { Ok(result) => result, Err(()) => { self.fail(wire); return; } }
        } else {
            let room = (MAX_BYTES - self.response_len).min(TURN);
            let received = if room != 0 {
                wire.receive(&mut self.response[self.response_len..self.response_len + room])
            } else {
                // Legacy whole-response mode retains its exact overflow probe.
                wire.receive(&mut [0u8; 1])
            };
            let n = match received { Ok(n) if n <= room && room != 0 => n,
                Ok(0) => 0, _ => { self.fail(wire); return; } };
            self.response_len += n; (n, n == 0)
        };
        if self.streaming && n != 0 { self.stream_progress(now); }
        if wire.link() == Link::Closed { self.fail(wire); return; }
        if wire.link() == Link::PeerFin {
            if self.sent != self.request_len || wire.queued() != 0 { self.fail(wire); return; }
            if !self.finished_send { wire.finish_send(); self.finished_send = true; }
            // A valid FIN may arrive while request bytes finish leaving the
            // protocol queue. The response must become drainable even if the
            // preceding state was Sending; a full ring is not observed EOF.
            self.state = if drained { State::Complete } else { State::Receiving };
            return;
        }
        if n == 0 && wire.link() == Link::Open && self.sent < self.request_len {
            self.state = State::Sending;
            let end = (self.sent + TURN).min(self.request_len);
            match wire.send(&self.request[self.sent..end]) {
                Ok(sent) if sent <= end - self.sent => {
                    self.sent += sent;
                    if self.streaming && sent != 0 { self.stream_progress(now); }
                },
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
        aborts: usize, finishes: usize, receive_calls: usize, begin_error: Option<u32> }
    impl TestWire {
        fn new() -> Self { Self { link: Link::Open, incoming: Vec::new(), sent: Vec::new(),
            receive_at: 0, send_limit: TURN, receive_limit: TURN, queued: 0,
            begins: 0, aborts: 0, finishes: 0, receive_calls: 0, begin_error: None } }
    }
    impl Wire for TestWire {
        fn begin(&mut self, _: u32) -> Result<(), u32> { self.begins += 1; self.begin_error.map_or(Ok(()), Err) }
        fn receive(&mut self, out: &mut [u8]) -> Result<usize, u32> {
            assert!(out.len() <= TURN);
            self.receive_calls += 1;
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

    #[test] fn held_other_protocol_blocks_begin_with_retained_replay_receipt() {
        let mut m=Machine::new(); let mut w=TestWire::new(); let mut begin=req(1,abi::BEGIN,0);begin.length=1;
        assert!(m.handle_exclusive(A,begin,10,true,&mut w));
        assert_eq!(m.pending_reply(),Some((A,abi::error(1,abi::BUSY))));m.reply_accepted();
        assert!(m.handle_exclusive(A,begin,11,false,&mut w));
        assert_eq!(m.pending_reply(),Some((A,abi::error(1,abi::BUSY))));m.reply_accepted();
        assert_eq!(m.owner(),None);assert_eq!(m.next_ticket,1);assert_eq!(w.begins,0);
        begin.seq=2;assert!(m.handle_exclusive(A,begin,12,false,&mut w));
        assert_eq!(m.pending_reply(),Some((A,abi::reply(2,1))));assert_eq!(m.owner(),Some(A));
    }

    fn resolved_begin(seq:u32,generation:u32)->Request {
        let mut r=req(seq,abi::BEGIN_RESOLVED,0);r.length=86;r.a=generation as u64;r
    }
    fn observation(owner:Identity,generation:u32)->Resolution {
        Resolution{owner,generation,address:[10,0,2,2],observed_tick:10,expires_tick:1010}
    }
    fn resolved_call(m:&mut Machine,w:&mut TestWire,r:Request,now:u64,admission:Result<Resolution,u32>)->Result<u32,u32>{
        assert!(m.handle_resolved(A,r,now,false,admission,w));
        let (owner,word)=m.pending_reply().unwrap();assert_eq!(owner,A);m.reply_accepted();abi::decode_reply(word,r.seq)
    }
    #[test] fn resolved_registers_reject_noncanonical_selectors() {
        let mut r=resolved_begin(1,1);assert!(r.valid());assert_eq!(Request::decode(r.encode()),Ok(r));
        for selector in [0,abi::MAX_TICKET as u64+1,1u64<<32]{r.a=selector;assert!(!r.valid());}
        r.a=1;r.ticket=1;assert!(!r.valid());r.ticket=0;r.offset=1;assert!(!r.valid());
        r.offset=0;r.length=0;assert!(!r.valid());r.length=4097;assert!(!r.valid());
    }
    #[test] fn resolved_denial_is_retained_across_renewal_and_full() {
        let mut m=Machine::new();let mut w=TestWire::new();let r=resolved_begin(1,3);
        assert!(m.handle_resolved(A,r,10,false,Err(abi::DENIED),&mut w));
        let receipt=m.pending_reply();assert_eq!(receipt,Some((A,abi::error(1,abi::DENIED))));
        assert!(!m.handle_resolved(A,resolved_begin(2,3),11,false,Ok(observation(A,3)),&mut w));
        assert!(m.handle_resolved(A,r,11,false,Ok(observation(A,3)),&mut w));assert_eq!(m.pending_reply(),receipt);
        m.reply_accepted();assert_eq!(resolved_call(&mut m,&mut w,r,11,Ok(observation(A,3))),Err(abi::DENIED));
        assert_eq!(m.next_ticket,1);assert_eq!(m.owner(),None);assert_eq!(w.begins,0);
        assert_eq!(resolved_call(&mut m,&mut w,resolved_begin(2,3),11,Ok(observation(A,3))),Ok(1));
    }
    #[test] fn resolved_acceptance_replays_after_expiry_but_changed_request_cannot() {
        let mut m=Machine::new();let mut w=TestWire::new();let r=resolved_begin(5,2);let record=observation(A,2);
        assert_eq!(resolved_call(&mut m,&mut w,r,11,Ok(record)),Ok(1));assert_eq!(m.resolution(),Some(record));
        assert_eq!(resolved_call(&mut m,&mut w,r,2000,Err(abi::STALE)),Ok(1));
        assert_eq!(resolved_call(&mut m,&mut w,resolved_begin(5,3),2000,Ok(observation(A,3))),Err(abi::STALE));
        assert_eq!(m.next_ticket,2);assert_eq!(m.resolution(),Some(record));assert_eq!(w.begins,0);
        assert_eq!(call(&mut m,&mut w,A,req(6,abi::RELEASE,1)),Ok(0));assert_eq!(m.resolution(),None);
    }
    #[test] fn resolved_owner_generation_clock_and_policy_denials_allocate_nothing() {
        for mode in 0..5{
            let mut m=Machine::new();let mut w=TestWire::new();let mut record=observation(A,1);let mut now=11;
            let admission=match mode{0=>{record.owner=B;Ok(record)},1=>{record.generation=2;Ok(record)},2=>{now=9;Ok(record)},3=>{now=1010;Ok(record)},_=>Err(abi::DENIED)};
            assert!(resolved_call(&mut m,&mut w,resolved_begin(1,1),now,admission).is_err());
            assert_eq!(m.owner(),None);assert_eq!(m.resolution(),None);assert_eq!(m.next_ticket,1);assert_eq!(w.begins,0);
        }
    }
    #[test] fn resolved_other_protocol_busy_is_fenced_and_raw_begin_clears_provenance() {
        let mut m=Machine::new();let mut w=TestWire::new();let r=resolved_begin(1,1);
        assert!(m.handle_resolved(A,r,11,true,Ok(observation(A,1)),&mut w));m.reply_accepted();
        assert_eq!(resolved_call(&mut m,&mut w,r,11,Ok(observation(A,1))),Err(abi::BUSY));
        assert_eq!(resolved_call(&mut m,&mut w,resolved_begin(2,1),11,Ok(observation(A,1))),Ok(1));
        assert!(m.retire(A,&mut w));assert_eq!(m.resolution(),None);
        let mut raw=req(1,abi::BEGIN,0);raw.length=1;assert_eq!(call(&mut m,&mut w,B,raw),Ok(2));assert_eq!(m.resolution(),None);
    }

    fn stream_begin(seq:u32)->Request {
        let mut r=resolved_begin(seq,1);r.kind=abi::BEGIN_STREAM_RESOLVED;r.length=1;r
    }
    fn call_at(m:&mut Machine,w:&mut TestWire,owner:Identity,r:Request,now:u64)->Result<u32,u32> {
        assert!(m.handle(owner,r,now,w));let(who,word)=m.pending_reply().unwrap();assert_eq!(who,owner);
        m.reply_accepted();abi::decode_reply(word,r.seq)
    }
    fn streaming()->(Machine,TestWire,u32) {
        let mut m=Machine::new();let mut w=TestWire::new();
        let t=resolved_call(&mut m,&mut w,stream_begin(1),10,Ok(observation(A,1))).unwrap();
        let mut write=req(2,abi::WRITE,t);write.length=1;write.a=b'X' as u64;
        assert_eq!(call_at(&mut m,&mut w,A,write,10),Ok(1));
        assert_eq!(call_at(&mut m,&mut w,A,req(3,abi::START,t),10),Ok(0));
        m.advance(11,&mut w);assert_eq!(m.state,State::Receiving);assert_eq!(w.sent,b"X");(m,w,t)
    }
    fn read(seq:u32,t:u32,offset:usize,length:usize)->Request {
        let mut r=req(seq,abi::READ,t);r.offset=offset as u16;r.length=length as u16;r
    }
    #[test] fn stream_kind_selector_bounds_admission_and_other_protocol_denials_are_fenced() {
        let r=stream_begin(1);assert!(r.valid());assert_eq!(Request::decode(r.encode()),Ok(r));
        for a in [0,abi::MAX_TICKET as u64+1,1u64<<32]{let mut bad=r;bad.a=a;assert!(!bad.valid());}
        for length in [0,4097,u16::MAX]{let mut bad=r;bad.length=length;assert!(!bad.valid());}
        let mut args=r.encode();args[0]=(args[0]&!(255<<24))|(12<<24);assert_eq!(Request::decode(args),Err(abi::INVALID));
        let mut m=Machine::new();let mut w=TestWire::new();assert_eq!(call_at(&mut m,&mut w,A,r,10),Err(abi::DENIED));
        assert_eq!(resolved_call(&mut m,&mut w,r,11,Ok(observation(A,1))),Err(abi::DENIED));assert_eq!(m.owner(),None);assert_eq!(m.next_ticket,1);assert!(!m.streaming);
        assert!(m.handle_resolved(A,stream_begin(2),11,true,Ok(observation(A,1)),&mut w));m.reply_accepted();
        assert_eq!(resolved_call(&mut m,&mut w,stream_begin(2),11,Ok(observation(A,1))),Err(abi::BUSY));assert_eq!(m.next_ticket,1);assert_eq!(w.begins,0);
    }
    #[test] fn stream_full_ring_pauses_without_probe_or_false_fin_and_reuses_scrubbed_cells() {
        let(mut m,mut w,t)=streaming();w.incoming=(0..2048).map(|n|(n%251)as u8).collect();
        for now in 12..16{m.advance(now,&mut w);}assert_eq!((m.response_len,m.consumed,w.receive_at),(1024,0,1024));let calls=w.receive_calls;
        w.link=Link::PeerFin;for now in 16..20{m.advance(now,&mut w);}assert_eq!(w.receive_calls,calls);assert_eq!(m.state,State::Receiving);assert_eq!(w.receive_at,1024);
        assert_eq!(call_at(&mut m,&mut w,A,read(4,t,0,4),19),Ok(0x03020100));assert_eq!(&m.response[..4],&[0;4]);assert_eq!(m.snapshot().received,1024);
        assert_eq!(call_at(&mut m,&mut w,A,req(5,abi::STATUS,t),19),Ok(abi::status(State::Receiving,1024)));
        m.advance(20,&mut w);assert_eq!((m.response_len,m.consumed,w.receive_at),(1028,4,1028));assert_eq!(&m.response[..4],&w.incoming[1024..1028]);assert!(m.response[abi::STREAM_RING_BYTES..].iter().all(|b|*b==0));
    }
    #[test] fn stream_read_full_and_exact_replay_preserve_copied_bytes_after_ring_reuse() {
        let(mut m,mut w,t)=streaming();w.incoming=(0..2048).map(|n|(n%251)as u8).collect();for now in 12..16{m.advance(now,&mut w);}
        let r=read(4,t,0,4);assert!(m.handle(A,r,16,&mut w));let receipt=m.pending_reply();assert_eq!(receipt,Some((A,abi::reply(4,0x03020100))));assert_eq!(m.consumed,4);
        assert!(!m.handle(A,read(5,t,4,4),17,&mut w));assert!(!m.handle(A,req(5,abi::CANCEL,t),17,&mut w));assert_eq!(w.aborts,0);
        m.advance(17,&mut w);assert_eq!(m.response_len,1028);assert_eq!(&m.response[..4],&w.incoming[1024..1028]);let idle=m.idle_deadline;
        assert!(m.handle(A,r,20_000,&mut w));assert_eq!(m.pending_reply(),receipt);assert_eq!(m.consumed,4);assert_eq!(m.idle_deadline,idle);
        m.reply_accepted();assert_eq!(call_at(&mut m,&mut w,A,r,20_000),Ok(0x03020100));assert_eq!(m.consumed,4);assert_eq!(m.idle_deadline,idle);
        let expected=u32::from_le_bytes(w.incoming[4..8].try_into().unwrap());assert_eq!(call_at(&mut m,&mut w,A,read(5,t,4,4),18),Ok(expected));assert_eq!(m.consumed,8);assert_eq!(w.begins,1);
    }
    #[test] fn stream_foreign_stale_and_gapped_cursors_cannot_consume_or_refresh_idle() {
        let(mut m,mut w,t)=streaming();w.incoming=b"abcdefgh".to_vec();m.advance(12,&mut w);let idle=m.idle_deadline;
        assert_eq!(call_at(&mut m,&mut w,B,read(1,t,0,4),13),Err(abi::DENIED));
        assert_eq!(call_at(&mut m,&mut w,A,read(4,t+1,0,4),13),Err(abi::STALE));
        assert_eq!(call_at(&mut m,&mut w,A,read(5,t,1,4),13),Err(abi::INVALID));assert_eq!(m.consumed,0);assert_eq!(m.idle_deadline,idle);
        assert_eq!(call_at(&mut m,&mut w,A,read(6,t,0,4),13),Ok(u32::from_le_bytes(*b"abcd")));
        assert_eq!(call_at(&mut m,&mut w,A,read(7,t,0,4),13),Err(abi::INVALID));assert_eq!(m.consumed,4);
        assert_eq!(call_at(&mut m,&mut w,A,read(8,t,8,4),13),Err(abi::INVALID));assert_eq!(m.consumed,4);
        assert_eq!(call_at(&mut m,&mut w,A,read(9,t,4,4),13),Ok(u32::from_le_bytes(*b"efgh")));let idle=m.idle_deadline;
        assert_eq!(call_at(&mut m,&mut w,A,read(10,t,8,1),14),Err(abi::BUSY));assert_eq!((m.consumed,m.response_len),(8,8));assert_eq!(m.idle_deadline,idle);assert_eq!(w.aborts,0);
    }
    #[test] fn stream_partial_receives_wrap_and_status_reports_cumulative_length() {
        let(mut m,mut w,t)=streaming();let wanted:Vec<u8>=(0..MAX_BYTES+777).map(|n|(n*17+19)as u8).collect();w.incoming=wanted.clone();w.receive_limit=7;
        let mut actual=Vec::new();let mut seq=4;let mut now=11;
        for _ in 0..5000{now+=1;m.advance(now,&mut w);assert_ne!(m.state,State::Failed);let available=m.response_len-m.consumed;
            if available!=0{let n=available.min(4);let offset=actual.len();let word=call_at(&mut m,&mut w,A,read(seq,t,offset,n),now).unwrap();seq+=1;actual.extend_from_slice(&word.to_le_bytes()[..n]);}
            if actual.len()==wanted.len(){break;}}
        assert_eq!(actual,wanted);assert_eq!(m.response_len,wanted.len());assert_eq!(m.consumed,wanted.len());assert_ne!(m.state,State::Complete);
        assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::STATUS,t),now),Ok(abi::status(State::Receiving,wanted.len())));seq+=1;
        w.link=Link::PeerFin;now+=1;m.advance(now,&mut w);assert_eq!(m.state,State::Complete);assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::STATUS,t),now),Ok(abi::status(State::Complete,wanted.len())));
    }
    #[test] fn stream_wire_cap_needs_real_fin_and_extra_byte_fails_without_partial_completion() {
        for extra in [0,1]{let(mut m,mut w,t)=streaming();let wanted:Vec<u8>=(0..abi::MAX_STREAM_BYTES+extra).map(|n|(n*13)as u8).collect();w.incoming=wanted.clone();let mut seq=4;let mut now=11;let mut actual=Vec::new();
            for _ in 0..5000{now+=1;m.advance(now,&mut w);if m.state==State::Failed{break;}
                if m.response_len>m.consumed{let n=(m.response_len-m.consumed).min(4);let offset=actual.len();let word=call_at(&mut m,&mut w,A,read(seq,t,offset,n),now).unwrap();seq+=1;actual.extend_from_slice(&word.to_le_bytes()[..n]);}
                if actual.len()==abi::MAX_STREAM_BYTES{break;}}
            if extra!=0{assert_eq!(m.state,State::Failed);scrubbed(&m);assert_eq!(w.receive_at,abi::MAX_STREAM_BYTES+1);assert!(!m.can_transmit(t));}
            else{assert_eq!(actual,wanted);assert_eq!(m.response_len,abi::MAX_STREAM_BYTES);assert_eq!(m.state,State::Receiving);now+=1;m.advance(now,&mut w);assert_eq!(m.state,State::Receiving);
                w.link=Link::PeerFin;now+=1;m.advance(now,&mut w);assert_eq!(m.state,State::Complete);assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::RELEASE,t),now),Ok(0));scrubbed(&m);}}
    }
    #[test] fn stream_complete_can_hold_unread_data_until_release() {
        let(mut m,mut w,t)=streaming();w.incoming=b"complete then read".to_vec();w.link=Link::PeerFin;m.advance(12,&mut w);assert_eq!(m.state,State::Receiving);m.advance(13,&mut w);assert_eq!(m.state,State::Complete);
        let produced=m.response_len;let mut actual=Vec::new();let mut seq=4;while actual.len()<produced{let offset=actual.len();let n=(produced-offset).min(4);let word=call_at(&mut m,&mut w,A,read(seq,t,offset,n),40_000).unwrap();seq+=1;actual.extend_from_slice(&word.to_le_bytes()[..n]);}
        assert_eq!(actual,w.incoming);assert_eq!((m.response_len,m.consumed),(produced,produced));assert!(m.response.iter().all(|b|*b==0));
    }
    #[test] fn stream_idle_refreshes_only_true_movement_and_expired_read_cannot_revive() {
        let(mut m,mut w,t)=streaming();assert_eq!((m.deadline,m.idle_deadline),(30_010,5_011));w.incoming=b"data".to_vec();m.advance(1000,&mut w);assert_eq!(m.idle_deadline,6000);
        let r=read(4,t,0,4);assert_eq!(call_at(&mut m,&mut w,A,r,2000),Ok(u32::from_le_bytes(*b"data")));assert_eq!(m.idle_deadline,7000);
        assert_eq!(call_at(&mut m,&mut w,A,r,6999),Ok(u32::from_le_bytes(*b"data")));assert_eq!(m.idle_deadline,7000);
        assert_eq!(call_at(&mut m,&mut w,A,req(5,abi::STATUS,t),6999),Ok(abi::status(State::Receiving,4)));assert_eq!(m.idle_deadline,7000);m.advance(7000,&mut w);assert_eq!(m.state,State::Failed);scrubbed(&m);
        let(mut m,mut w,t)=streaming();w.incoming=b"data".to_vec();m.advance(12,&mut w);assert_eq!(call_at(&mut m,&mut w,A,read(4,t,0,4),5012),Err(abi::TIMEOUT));assert_eq!(m.state,State::Failed);scrubbed(&m);
    }
    #[test] fn stream_total_deadline_clock_rollback_and_start_overflow_fail_scrubbed() {
        let(mut m,mut w,t)=streaming();w.incoming=vec![1;32];w.receive_limit=1;
        for now in [4000,8000,12000,16000,20000,24000,28000]{m.advance(now,&mut w);assert_eq!(m.state,State::Receiving);}
        assert_eq!((m.deadline,m.idle_deadline),(30_010,30_010));m.advance(30_010,&mut w);assert_eq!(m.state,State::Failed);assert!(!m.can_transmit(t));scrubbed(&m);
        let(mut m,mut w,_)=streaming();m.advance(12,&mut w);m.advance(11,&mut w);assert_eq!(m.state,State::Failed);scrubbed(&m);
        let mut m=Machine::new();let mut w=TestWire::new();let t=resolved_call(&mut m,&mut w,stream_begin(1),10,Ok(observation(A,1))).unwrap();let mut write=req(2,abi::WRITE,t);write.length=1;write.a=1;assert_eq!(call_at(&mut m,&mut w,A,write,10),Ok(1));
        assert_eq!(call_at(&mut m,&mut w,A,req(3,abi::START,t),u64::MAX-100),Err(abi::TIMEOUT));assert_eq!(w.begins,0);assert_eq!(m.state,State::Failed);scrubbed(&m);
    }
    #[test] fn stream_cancel_release_retire_scrub_whole_buffers_and_exact_pending_receipts() {
        for cleanup in [abi::CANCEL,abi::RELEASE,0]{let(mut m,mut w,t)=streaming();w.incoming=vec![0x61;1024];m.advance(12,&mut w);m.request[MAX_BYTES-1]=9;m.response[MAX_BYTES-1]=8;
            if cleanup==0{assert!(m.handle(A,read(4,t,0,4),12,&mut w));assert!(!m.retire(B,&mut w));assert_eq!(m.consumed,4);assert!(m.retire(A,&mut w));assert_eq!(m.pending_reply(),None);assert!(m.cached.iter().all(Option::is_none));assert_eq!(m.owner(),None);}
            else{assert_eq!(call_at(&mut m,&mut w,A,req(4,cleanup,t),12),Ok(0));if cleanup==abi::CANCEL{assert_eq!(m.state,State::Cancelled);assert_eq!(m.owner(),Some(A));assert_eq!(call_at(&mut m,&mut w,A,req(5,abi::RELEASE,t),13),Ok(0));}assert_eq!(m.owner(),None);}
            scrubbed(&m);assert_eq!((m.response_len,m.consumed),(0,0));assert!(!m.streaming);assert_eq!(m.idle_deadline,0);assert!(!m.can_transmit(t));assert_eq!(w.begins,1);}
    }

    #[test] fn stream_fin_while_sending_exposes_receiving_and_drains_more_than_ring_to_completion() {
        let mut m=Machine::new();let mut w=TestWire::new();assert_eq!(m.stream_snapshot(),None);
        let t=resolved_call(&mut m,&mut w,stream_begin(1),10,Ok(observation(A,1))).unwrap();
        let mut write=req(2,abi::WRITE,t);write.length=1;write.a=b'X' as u64;
        assert_eq!(call_at(&mut m,&mut w,A,write,10),Ok(1));assert_eq!(call_at(&mut m,&mut w,A,req(3,abi::START,t),10),Ok(0));
        // Actual send is accepted into a still-pending protocol queue, so the
        // machine legitimately remains Sending with write-side close deferred.
        w.queued=1;m.advance(11,&mut w);assert_eq!(w.sent,b"X");assert_eq!(m.state,State::Sending);assert!(!m.finished_send);
        let wanted:Vec<u8>=(0..2049).map(|n|(n*19+7)as u8).collect();w.incoming=wanted.clone();w.queued=0;w.link=Link::PeerFin;
        m.advance(12,&mut w);assert_eq!(m.state,State::Receiving);assert!(m.finished_send);assert_eq!(w.finishes,1);
        for now in 13..17{m.advance(now,&mut w);}
        assert_eq!(m.stream_snapshot(),Some(StreamSnapshot{produced:1024,consumed:0,buffered:1024}));assert_eq!(m.state,State::Receiving);
        let mut actual=Vec::new();let mut seq=4;let mut now=16;
        for _ in 0..1000 {
            let before=m.stream_snapshot().unwrap();assert!(before.buffered<=abi::STREAM_RING_BYTES);assert_eq!(before.produced-before.consumed,before.buffered);
            if before.buffered!=0 {
                let count=before.buffered.min(4);let offset=actual.len();let word=call_at(&mut m,&mut w,A,read(seq,t,offset,count),now).unwrap();seq+=1;
                actual.extend_from_slice(&word.to_le_bytes()[..count]);let after=m.stream_snapshot().unwrap();assert_eq!(after.produced,before.produced);assert_eq!(after.consumed,before.consumed+count);assert_eq!(after.buffered,before.buffered-count);
            }
            if actual.len()==wanted.len()&&m.state==State::Complete{break;}
            now+=1;m.advance(now,&mut w);assert_ne!(m.state,State::Failed);
        }
        assert_eq!(actual,wanted);assert_eq!(m.state,State::Complete);assert_eq!(m.stream_snapshot(),Some(StreamSnapshot{produced:2049,consumed:2049,buffered:0}));assert_eq!(w.finishes,1);
        assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::RELEASE,t),now),Ok(0));assert_eq!(m.stream_snapshot(),None);scrubbed(&m);
    }

    fn duplex(bytes:usize)->(Machine,TestWire,u32,u32){
        let(mut m,mut w)=(Machine::new(),TestWire::new());let mut begin=stream_begin(1);begin.kind=abi::BEGIN_DUPLEX_RESOLVED;begin.length=bytes as u16;
        let t=resolved_call(&mut m,&mut w,begin,10,Ok(observation(A,1))).unwrap();let mut seq=2;
        for offset in (0..bytes).step_by(8){let mut r=req(seq,abi::WRITE,t);seq+=1;r.offset=offset as u16;r.length=(bytes-offset).min(8)as u16;r.a=u64::MAX>>((8-r.length)*8);assert_eq!(call_at(&mut m,&mut w,A,r,10),Ok((offset+r.length as usize)as u32));}
        assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::START,t),10),Ok(0));seq+=1;assert!(m.is_duplex());assert!(!m.finish_requested());(m,w,t,seq)
    }
    fn duplex_write(seq:u32,t:u32,offset:usize,bytes:&[u8])->Request{
        let mut r=req(seq,abi::WRITE,t);r.offset=offset as u16;r.length=bytes.len()as u16;let mut word=[0;8];word[..bytes.len()].copy_from_slice(bytes);r.a=u64::from_le_bytes(word);r
    }
    #[test]fn duplex_abi_canonical_begin_finish_and_fail_closed_admission(){
        let mut begin=stream_begin(1);begin.kind=abi::BEGIN_DUPLEX_RESOLVED;assert!(begin.valid());assert_eq!(Request::decode(begin.encode()),Ok(begin));
        for length in [0,4097,u16::MAX]{let mut r=begin;r.length=length;assert!(!r.valid());}for a in [0,abi::MAX_TICKET as u64+1,1<<32]{let mut r=begin;r.a=a;assert!(!r.valid());}
        let finish=req(2,abi::FINISH,1);assert!(finish.valid());assert_eq!(Request::decode(finish.encode()),Ok(finish));for mode in 0..3{let mut r=finish;match mode{0=>r.offset=1,1=>r.length=1,_=>r.a=1}assert!(!r.valid());}
        let(mut m,mut w)=(Machine::new(),TestWire::new());assert_eq!(call_at(&mut m,&mut w,A,begin,10),Err(abi::DENIED));assert_eq!(resolved_call(&mut m,&mut w,begin,11,Ok(observation(A,1))),Err(abi::DENIED));assert!(!m.is_duplex());assert_eq!(m.next_ticket,1);assert_eq!(w.begins,0);
        begin.seq=2;assert!(m.handle_resolved(A,begin,11,true,Ok(observation(A,1)),&mut w));m.reply_accepted();assert_eq!(resolved_call(&mut m,&mut w,begin,12,Ok(observation(A,1))),Err(abi::BUSY));assert_eq!(m.owner(),None);
    }
    #[test]fn duplex_initial_drain_never_halfcloses_and_finish_requires_all_ingress_consumed(){
        let(mut m,mut w,t,mut seq)=duplex(1);m.advance(11,&mut w);assert_eq!((m.sent,w.finishes),(1,0));assert_eq!(m.state,State::Receiving);
        w.incoming=b"abc".to_vec();w.link=Link::PeerFin;m.advance(12,&mut w);assert_eq!(m.response_len,3);assert_eq!(m.state,State::Receiving);assert_eq!(w.finishes,0);
        assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::FINISH,t),12),Ok(0));seq+=1;m.advance(13,&mut w);assert_eq!(w.finishes,1);assert_eq!(m.state,State::Receiving);
        assert_eq!(call_at(&mut m,&mut w,A,read(seq,t,0,3),13),Ok(0x00636261));seq+=1;m.advance(14,&mut w);assert_eq!(m.state,State::Complete);assert_eq!(m.consumed,m.response_len);
        assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::FINISH,t),40_000),Ok(0));m.advance(40_001,&mut w);assert_eq!(w.finishes,1);
    }
    #[test]fn duplex_finish_seals_writes_and_waits_for_private_and_wire_queues(){
        let(mut m,mut w,t,mut seq)=duplex(600);w.send_limit=10;w.queued=1;assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::FINISH,t),11),Ok(0));seq+=1;
        assert_eq!(call_at(&mut m,&mut w,A,duplex_write(seq,t,600,b"x"),11),Err(abi::BUSY));seq+=1;assert_eq!(m.staged,600);
        for now in 12..72{m.advance(now,&mut w);}assert_eq!(m.sent,600);assert_eq!(w.finishes,0);w.queued=0;m.advance(72,&mut w);assert_eq!(w.finishes,1);
        assert_eq!(call_at(&mut m,&mut w,A,duplex_write(seq,t,600,b"x"),73),Err(abi::BUSY));assert_eq!(m.staged,600);
        let(mut legacy,mut wire)=(Machine::new(),TestWire::new());let ticket=start(&mut legacy,&mut wire,1);assert_eq!(call_at(&mut legacy,&mut wire,A,req(1001,abi::FINISH,ticket),11),Err(abi::BUSY));
    }
    #[test]fn duplex_both_directions_progress_under_continuous_rx_with_shared_turn_budget(){
        let(mut m,mut w,_,_)=duplex(MAX_BYTES);w.incoming=vec![0x42;2048];
        for now in 11..19{let before=(m.sent,m.response_len);m.advance(now,&mut w);assert_eq!(m.sent-before.0,TURN/2);assert_eq!(m.response_len-before.1,TURN/2);assert_eq!((m.sent-before.0)+(m.response_len-before.1),TURN);}
        let calls=w.receive_calls;let sent=m.sent;m.advance(19,&mut w);assert_eq!(w.receive_calls,calls);assert_eq!(m.sent-sent,TURN);assert_eq!(w.finishes,0);assert_eq!(m.response_len,abi::STREAM_RING_BYTES);
    }
    #[test]fn duplex_tx_full_rejects_before_effect_then_wraps_with_retained_write_receipt(){
        let(mut m,mut w,t,mut seq)=duplex(MAX_BYTES);w.link=Link::Opening;let full=duplex_write(seq,t,MAX_BYTES,b"abcdefgh");assert_eq!(call_at(&mut m,&mut w,A,full,11),Err(abi::BUSY));seq+=1;assert_eq!(m.staged,MAX_BYTES);
        w.link=Link::Open;m.advance(12,&mut w);let write=duplex_write(seq,t,MAX_BYTES,b"abcdefgh");assert!(m.handle(A,write,12,&mut w));let receipt=m.pending_reply();assert_eq!(receipt,Some((A,abi::reply(seq,(MAX_BYTES+8)as u32))));assert_eq!(&m.request[..8],b"abcdefgh");
        assert!(!m.handle(A,duplex_write(seq+1,t,MAX_BYTES+8,b"z"),13,&mut w));assert_eq!(m.staged,MAX_BYTES+8);for now in 13..32{m.advance(now,&mut w);}assert_eq!(w.sent.len(),MAX_BYTES+8);assert_eq!(&w.sent[MAX_BYTES..],b"abcdefgh");assert!(m.request.iter().all(|b|*b==0));
        assert!(m.handle(A,write,20_000,&mut w));assert_eq!(m.pending_reply(),receipt);m.reply_accepted();assert_eq!(call_at(&mut m,&mut w,A,write,20_000),Ok((MAX_BYTES+8)as u32));assert_eq!(w.sent.len(),MAX_BYTES+8);assert_eq!(m.staged,MAX_BYTES+8);
    }
    #[test]fn duplex_early_read_and_late_exact_replay_survive_ring_reuse(){
        let(mut m,mut w,t,seq)=duplex(1);w.link=Link::Opening;w.incoming=(0..2048).map(|n|(n%251)as u8).collect();m.advance(11,&mut w);assert_eq!(m.state,State::Connecting);
        let read=read(seq,t,0,4);assert!(m.handle(A,read,11,&mut w));let receipt=m.pending_reply();assert_eq!(receipt,Some((A,abi::reply(seq,0x03020100))));assert_eq!(m.consumed,4);assert_eq!(&m.response[..4],&[0;4]);
        for now in 12..22{m.advance(now,&mut w);}assert_eq!(m.response_len,1028);assert_eq!(&m.response[..4],&w.incoming[1024..1028]);assert!(m.handle(A,read,22,&mut w));assert_eq!(m.pending_reply(),receipt);m.reply_accepted();assert_eq!(call_at(&mut m,&mut w,A,read,23),Ok(0x03020100));assert_eq!(m.consumed,4);
    }
    #[test]fn duplex_absolute_cursor_and_exact_owner_rejections_preserve_buffers(){
        let(mut m,mut w,t,mut seq)=duplex(1);m.advance(11,&mut w);w.incoming=b"abcd".to_vec();m.advance(12,&mut w);let idle=m.idle_deadline;
        assert_eq!(call_at(&mut m,&mut w,B,duplex_write(1,t,1,b"x"),12),Err(abi::DENIED));assert_eq!(call_at(&mut m,&mut w,A,duplex_write(seq,t+1,1,b"x"),12),Err(abi::STALE));seq+=1;
        for offset in [0,2]{assert_eq!(call_at(&mut m,&mut w,A,duplex_write(seq,t,offset,b"x"),12),Err(abi::INVALID));seq+=1;}assert_eq!(m.staged,1);assert_eq!(m.idle_deadline,idle);
        assert_eq!(call_at(&mut m,&mut w,B,read(2,t,0,4),12),Err(abi::DENIED));assert_eq!(call_at(&mut m,&mut w,A,read(seq,t,1,3),12),Err(abi::INVALID));assert_eq!((m.consumed,m.response_len),(0,4));assert_eq!(m.idle_deadline,idle);
    }
    #[test]fn duplex_cumulative_tx_bound_never_wraps_or_extends_on_duplicate(){
        let(mut m,mut w,t,mut seq)=duplex(1);m.advance(11,&mut w);let mut expected=vec![255];
        while m.staged<abi::MAX_DUPLEX_BYTES{let n=(abi::MAX_DUPLEX_BYTES-m.staged).min(8);let bytes=[0x69;8];let r=duplex_write(seq,t,m.staged,&bytes[..n]);assert_eq!(call_at(&mut m,&mut w,A,r,11),Ok((m.staged)as u32));seq+=1;expected.extend_from_slice(&bytes[..n]);m.advance(11,&mut w);}
        assert_eq!(w.sent,expected);assert_eq!((m.sent,m.staged),(abi::MAX_DUPLEX_BYTES,abi::MAX_DUPLEX_BYTES));assert_eq!(call_at(&mut m,&mut w,A,duplex_write(seq,t,abi::MAX_DUPLEX_BYTES,b"x"),11),Err(abi::INVALID));assert_eq!(m.staged,abi::MAX_DUPLEX_BYTES);assert_eq!(w.finishes,0);
    }
    #[test]fn duplex_rx_limit_requires_real_fin_and_full_consumption_extra_byte_fails(){
        for extra in [0,1]{let(mut m,mut w,t,mut seq)=duplex(1);w.incoming=vec![0x61;abi::MAX_DUPLEX_BYTES+extra];let mut actual=Vec::new();let mut now=10;
            for _ in 0..9000{now+=1;m.advance(now,&mut w);if m.state==State::Failed{break;}let available=m.response_len-m.consumed;if available!=0{let n=available.min(4);let r=read(seq,t,actual.len(),n);seq+=1;let word=call_at(&mut m,&mut w,A,r,now).unwrap();actual.extend_from_slice(&word.to_le_bytes()[..n]);}if actual.len()==abi::MAX_DUPLEX_BYTES{break;}}
            if extra!=0{assert_eq!(m.state,State::Failed);scrubbed(&m);}else{assert_eq!(actual,w.incoming);assert_ne!(m.state,State::Complete);assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::FINISH,t),now),Ok(0));m.advance(now+1,&mut w);assert_ne!(m.state,State::Complete);w.link=Link::PeerFin;m.advance(now+2,&mut w);assert_eq!(m.state,State::Complete);}
        }
    }
    #[test]fn duplex_deadlines_and_cleanup_scrub_both_rings_and_clear_mode(){
        for cleanup in [abi::CANCEL,abi::RELEASE,0,255]{let(mut m,mut w,t,seq)=duplex(1);m.advance(11,&mut w);w.incoming=b"secret ciphertext".to_vec();m.advance(12,&mut w);
            if cleanup==0{assert!(m.retire(A,&mut w));assert_eq!(m.pending_reply(),None);assert!(m.cached.iter().all(Option::is_none));}
            else if cleanup==255{m.advance(5012,&mut w);assert_eq!(m.state,State::Failed);}
            else{assert_eq!(call_at(&mut m,&mut w,A,req(seq,cleanup,t),12),Ok(0));}
            scrubbed(&m);assert!(!m.is_duplex());assert!(!m.finish_requested());assert_eq!((m.staged,m.sent,m.consumed,m.response_len),(0,0,0,0));assert!(!m.can_transmit(t));
        }
    }

    #[test]fn duplex_status_and_staging_cannot_refresh_idle_or_total_deadlines(){
        let(mut m,mut w,t,mut seq)=duplex(1);m.advance(11,&mut w);w.link=Link::Opening;let idle=m.idle_deadline;
        assert_eq!(call_at(&mut m,&mut w,A,duplex_write(seq,t,1,b"staged"),1000),Ok(7));seq+=1;assert_eq!(m.idle_deadline,idle);
        assert_eq!(call_at(&mut m,&mut w,A,req(seq,abi::STATUS,t),4000),Ok(abi::status(State::Receiving,0)));m.advance(idle,&mut w);assert_eq!(m.state,State::Failed);scrubbed(&m);
        let(mut m,mut w,_,_)=duplex(1);w.receive_limit=1;w.incoming=vec![7;8];
        for now in [4000,8000,12000,16000,20000,24000,28000]{m.advance(now,&mut w);assert_ne!(m.state,State::Failed);}
        assert_eq!(m.deadline,30_010);m.advance(30_010,&mut w);assert_eq!(m.state,State::Failed);scrubbed(&m);
        let(mut m,mut w,_,_)=duplex(1);m.advance(11,&mut w);m.advance(10,&mut w);assert_eq!(m.state,State::Failed);scrubbed(&m);
    }

}
