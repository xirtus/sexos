// SPDX-License-Identifier: GPL-3.0-only
//! Nonblocking admission of complete Linen text repaints. There is one immutable
//! active batch and one replaceable latest desired batch; no receive or allocation.
pub const CAPACITY: usize = 512;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata { pub count: usize, pub first: usize, pub rows: usize, pub selected: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt { pub metadata: Metadata, pub bytes: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation { pub opcode: u64, pub surface: u64, pub packed: u64, pub control: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission { Accepted, Full, Rejected(u64) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll { Idle, Pending, Deferred, Complete(Receipt), Rejected(u64) }
#[derive(Clone, Copy)]
struct Batch { bytes: [u8; CAPACITY], len: usize, color: u32, metadata: Metadata, cleared: bool, offset: usize }
pub struct Outbox { surface: u64, active: Option<Batch>, latest: Option<Batch> }
impl Outbox {
    pub const fn new(surface: u64) -> Self { Self { surface, active: None, latest: None } }
    /// An oversized input leaves both existing batches untouched. Replacement
    /// affects only the not-yet-started batch, never interleaves active chunks.
    pub fn queue(&mut self, text: &[u8], color: u32, metadata: Metadata) -> bool {
        if text.len() > CAPACITY { return false; }
        let mut batch = Batch { bytes: [0;CAPACITY], len: text.len(), color, metadata, cleared: false, offset: 0 };
        batch.bytes[..text.len()].copy_from_slice(text);
        if self.active.is_none() { self.active = Some(batch); self.latest = None; }
        else { self.latest = Some(batch); }
        true
    }
    pub fn pending(&self) -> bool { self.active.is_some() || self.latest.is_some() }
    pub fn cancel(&mut self) { self.active=None; self.latest=None; }
    /// At most `budget` calls. Full preserves the exact operation; unexpected
    /// errors abandon only the active batch without a success receipt. A queued
    /// or subsequent repaint can recover by starting with its own clear.
    pub fn poll(&mut self, budget: usize, mut send: impl FnMut(Operation) -> Admission) -> Poll {
        if self.active.is_none() { self.active = self.latest.take(); }
        if self.active.is_none() { return Poll::Idle; }
        for _ in 0..budget {
            let b = self.active.as_mut().unwrap();
            let chunk = (b.len - b.offset).min(8);
            let operation = if !b.cleared { Operation { opcode: 0xFA, surface: self.surface, packed: 0, control: 0 } }
                else {
                    let mut packed = 0;
                    for i in 0..chunk { packed |= (b.bytes[b.offset+i] as u64) << (i*8); }
                    Operation { opcode: 0xFB, surface: self.surface, packed,
                        control: (b.offset as u64 & 0xff) | ((chunk as u64) << 8)
                            | (((b.offset as u64 >> 8) & 0xf) << 12) | ((b.color as u64) << 32) }
                };
            match send(operation) {
                Admission::Full => return Poll::Deferred,
                Admission::Rejected(status) => { self.active = None; return Poll::Rejected(status); }
                Admission::Accepted => {
                    if !b.cleared { b.cleared = true; } else { b.offset += chunk; }
                    if b.offset == b.len {
                        let receipt = Receipt { metadata: b.metadata, bytes: b.len };
                        self.active = None;
                        return Poll::Complete(receipt);
                    }
                }
            }
        }
        Poll::Pending
    }
}

/// Latest desired visibility replaces an unsent hide/show. Already admitted
/// operations retain FIFO order at the display endpoint.
pub struct LatestOperation { operation: Option<Operation> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatestPoll { Idle, Deferred, Applied(Operation), Rejected(u64) }
impl LatestOperation {
    pub const fn new() -> Self { Self { operation: None } }
    pub fn set(&mut self, operation: Operation) { self.operation = Some(operation); }
    pub fn pending(&self) -> bool { self.operation.is_some() }
    pub fn poll(&mut self, send: impl FnOnce(Operation) -> Admission) -> LatestPoll {
        let Some(op) = self.operation else { return LatestPoll::Idle; };
        match send(op) {
            Admission::Full => LatestPoll::Deferred,
            Admission::Accepted => { self.operation = None; LatestPoll::Applied(op) }
            Admission::Rejected(status) => { self.operation = None; LatestPoll::Rejected(status) }
        }
    }
}

/// Five BOOT desktop surfaces (including the reserved successor) only. Observation preserves the caller's actual
/// transport result; a full-ring state operation is retried independently.
pub struct DesktopStates { slots: [Option<Operation>;5], chrome: [Option<Operation>;5], cursor: usize }
impl DesktopStates {
    pub const fn new() -> Self { Self { slots:[None;5],chrome:[None;5],cursor:0 } }
    pub fn tracks(surface:u64)->bool { matches!(surface,200|201|153|300|301) }
    pub fn observe(&mut self, op:Operation, result:Admission) {
        let index=match op.surface {200=>0,201=>1,153=>2,300=>3,301=>4,_=>return};
        if op.opcode==0xFD {self.chrome[index]=Some(op);}
        match op.opcode {
            0xEC => self.slots[index]=match result {
                Admission::Full=>Some(op),
                Admission::Accepted=>Some(self.chrome[index].unwrap_or(Operation{opcode:0xFD,surface:op.surface,packed:0,control:0})),
                Admission::Rejected(_)=>None,
            },
            0xEE => self.slots[index]=if result==Admission::Full {Some(op)} else {None},
            // A chrome enqueue before its missing create cannot complete that create.
            0xFD if !self.slots[index].map_or(false,|p|matches!(p.opcode,0xEC|0xEE)) =>
                self.slots[index]=if result==Admission::Full {Some(op)} else {None},
            _=>{},
        }
    }
    pub fn pending(&self)->bool {self.slots.iter().any(Option::is_some)}
    pub fn pending_surface(&self,surface:u64)->bool {self.slots.iter().flatten().any(|op|op.surface==surface)}
    /// Validation can replace old create geometry with the shell's latest
    /// bounds, or discard it after minimize/death. Hides remain independently
    /// eligible. No nested work/receive is performed by this bounded pump.
    pub fn poll(&mut self,budget:usize,mut validate:impl FnMut(Operation)->Option<Operation>,mut send:impl FnMut(Operation)->Admission) {
        let mut attempts=0;
        for _ in 0..5 {
            if attempts>=budget {break;}
            let index=self.cursor;self.cursor=(self.cursor+1)%5;
            let Some(original)=self.slots[index] else {continue;};
            let Some(op)=validate(original) else {self.slots[index]=None;continue;};
            attempts+=1;
            self.slots[index]=match send(op) {
                Admission::Full=>Some(op),
                Admission::Accepted if op.opcode==0xEC=>Some(self.chrome[index].unwrap_or(Operation{opcode:0xFD,surface:op.surface,packed:0,control:0})),
                _=>None,
            };
        }
    }
}
