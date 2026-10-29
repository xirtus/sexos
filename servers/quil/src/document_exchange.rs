//! Bounded main-loop loader. No nested receive, no partial publication.
use sex_pdx::document as w;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Info,
    Load,
    Name,
    Read,
    End,
    Abort,
    Create,
    BeginSave,
    Write,
    Commit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Loaded,
    Saved,
    Failed,
    Uncertain,
    Cancelled,
}
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub outcome: Outcome,
    pub object: u8,
    pub revision: u64,
    pub error: u32,
}
const WINDOW: usize = w::WINDOW;
#[derive(Clone, Copy)]
struct DataRequest {
    request: w::Request,
    count: usize,
    enqueued: bool,
}
pub struct Client {
    pub bytes: [u8; w::MAX],
    pub name: [u8; 16],
    pub len: usize,
    object: u8,
    revision: u64,
    phase: Phase,
    offset: usize,
    generation: u32,
    seq: u32,
    req: Option<w::Request>,
    awaiting: bool,
    window: [Option<DataRequest>; WINDOW],
    issued: usize,
    completed: usize,
    busy: bool,
    fenced: bool,
    dead: bool,
    stall: u32,
    total: u32,
    error: u32,
    saving: bool,
    creating: bool,
    known: [Option<u32>; 15],
    quiescent: bool,
}
impl Client {
    pub const fn new() -> Self {
        Self {
            bytes: [0; w::MAX],
            name: [0; 16],
            len: 0,
            object: 0,
            revision: 0,
            phase: Phase::Info,
            offset: 0,
            generation: 0,
            seq: 0,
            req: None,
            awaiting: false,
            window: [None; WINDOW],
            issued: 0,
            completed: 0,
            busy: false,
            fenced: false,
            dead: false,
            stall: 0,
            total: 0,
            error: 0,
            saving: false,
            creating: false,
            known: [None; 15],
            quiescent: false,
        }
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn loading(&self) -> bool {
        self.busy && !self.saving
    }
    pub fn status(&self) -> &'static str {
        if self.fenced {
            "UNCERTAIN"
        } else if self.busy {
            if self.saving {
                "SAVING"
            } else {
                "LOADING"
            }
        } else if self.error != 0 {
            "ERROR"
        } else if self.quiescent {
            "REOPEN TO CHECK"
        } else {
            "READY"
        }
    }
    pub fn load(&mut self, id: u8, revision: u64) -> bool {
        if self.busy || self.fenced || self.dead || !(2..15).contains(&id) {
            return false;
        }
        self.bytes.fill(0);
        self.name.fill(0);
        self.len = 0;
        self.object = id;
        self.revision = revision;
        self.saving = false;
        self.creating = false;
        self.phase = Phase::Info;
        self.offset = 0;
        self.clear_window();
        self.error = 0;
        self.quiescent = false;
        self.busy = true;
        self.stall = 0;
        self.total = 0;
        true
    }
    pub fn save(&mut self, id: Option<u8>, name: [u8; 16], bytes: &[u8], revision: u64) -> bool {
        if self.busy
            || self.fenced
            || self.dead
            || bytes.len() > w::MAX
            || id.map_or(false, |i| !(2..15).contains(&i))
        {
            return false;
        }
        self.bytes.fill(0);
        self.bytes[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        self.name = name;
        self.object = id.unwrap_or(0);
        self.revision = revision;
        self.saving = true;
        self.creating = id.is_none();
        self.phase = if self.creating {
            Phase::Create
        } else {
            Phase::Info
        };
        self.offset = 0;
        self.clear_window();
        self.error = 0;
        self.quiescent = false;
        self.busy = true;
        self.stall = 0;
        self.total = 0;
        true
    }
    pub fn cancel(&mut self) -> bool {
        if self.dead {
            return false;
        }
        // Repeated Cancel while the same ABORT is pending consumes no further
        // sequence or transport capacity and cannot extend its deadline.
        if self.busy && self.phase == Phase::Abort {
            return true;
        }
        if self.seq == w::SEQ_MAX {
            return false;
        }
        self.req = None;
        self.awaiting = false;
        self.clear_window();
        self.phase = Phase::Abort;
        self.busy = true;
        self.stall = 0;
        self.total = 0;
        true
    }
    pub fn close(&mut self) {
        self.bytes.fill(0);
        self.name.fill(0);
        self.len = 0;
        self.busy = false;
        self.req = None;
        self.awaiting = false;
        self.clear_window();
        self.dead = true;
    }
    fn finish(&mut self, outcome: Outcome, error: u32) -> Event {
        self.busy = false;
        self.req = None;
        self.awaiting = false;
        self.clear_window();
        self.error = error;
        self.fenced = outcome == Outcome::Uncertain;
        if !matches!(outcome, Outcome::Loaded | Outcome::Saved) {
            self.bytes.fill(0);
            self.name.fill(0);
            self.len = 0;
        }
        Event {
            outcome,
            object: self.object,
            revision: self.revision,
            error,
        }
    }
    fn fail(&mut self, error: u32) -> Option<Event> {
        if !self.saving && matches!(self.phase, Phase::Info | Phase::Load) {
            Some(self.finish(Outcome::Failed, error))
        } else {
            Some(self.finish(Outcome::Uncertain, error))
        }
    }
    fn clear_window(&mut self) {
        self.window = [None; WINDOW];
        self.issued = 0;
        self.completed = 0;
    }
    fn data_step(&mut self, send: &mut impl FnMut(w::Request) -> Result<(), u64>) -> Option<Event> {
        // Four privately owned descriptors, no shared pointer or new authority.
        // Retry only a send explicitly reported as not enqueued, before issuing
        // another request. An admitted request is never retransmitted.
        for _ in 0..WINDOW {
            let retry = self
                .window
                .iter()
                .enumerate()
                .filter_map(|(i, e)| e.filter(|e| !e.enqueued).map(|e| (i, e.request.seq)))
                .min_by_key(|(_, seq)| *seq)
                .map(|(i, _)| i);
            let i = if let Some(i) = retry {
                i
            } else {
                if self.issued == self.len {
                    break;
                }
                let Some(i) = self.window.iter().position(Option::is_none) else {
                    break;
                };
                // Leave one sequence available for exact terminal ABORT.
                if self.seq >= w::SEQ_MAX - 1 {
                    return Some(self.finish(Outcome::Uncertain, w::STALE));
                }
                self.seq += 1;
                let read = self.phase == Phase::Read;
                let count = (self.len - self.issued).min(if read { 4 } else { 16 });
                let (a, b) = if read {
                    (count as u64, 0)
                } else {
                    let mut raw = [0; 16];
                    raw[..count].copy_from_slice(&self.bytes[self.issued..self.issued + count]);
                    (
                        u64::from_le_bytes(raw[..8].try_into().unwrap()),
                        u64::from_le_bytes(raw[8..].try_into().unwrap()),
                    )
                };
                let request = w::Request {
                    seq: self.seq,
                    kind: if read { w::READ } else { w::WRITE },
                    object: self.object,
                    offset: self.issued as u32,
                    a,
                    b,
                };
                self.window[i] = Some(DataRequest {
                    request,
                    count,
                    enqueued: false,
                });
                self.issued += count;
                i
            };
            match send(self.window[i].unwrap().request) {
                Ok(()) => self.window[i].as_mut().unwrap().enqueued = true,
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => break,
                Err(_) => return Some(self.finish(Outcome::Uncertain, w::DENIED)),
            }
        }
        None
    }
    fn data_receive(&mut self, value: u64) -> Option<Event> {
        let (i, entry, value) = self.window.iter().enumerate().find_map(|(i, entry)| {
            let entry = (*entry)?;
            if !entry.enqueued {
                return None;
            }
            w::decode_reply(value, entry.request.seq).map(|value| (i, entry, value))
        })?;
        let v = match value {
            Ok(v) => v,
            Err(e) => return self.fail(e),
        };
        if self.phase == Phase::Read {
            if entry.count < 4 && v >> (entry.count * 8) != 0 {
                return self.fail(w::INVALID);
            }
            let offset = entry.request.offset as usize;
            self.bytes[offset..offset + entry.count]
                .copy_from_slice(&v.to_le_bytes()[..entry.count]);
        } else if v as usize != entry.count {
            return self.fail(w::INVALID);
        }
        self.window[i] = None;
        self.completed += entry.count;
        self.stall = 0;
        if self.completed == self.len {
            self.phase = if self.saving {
                Phase::Commit
            } else {
                Phase::End
            };
        }
        None
    }
    pub fn step(&mut self, mut send: impl FnMut(w::Request) -> Result<(), u64>) -> Option<Event> {
        if !self.busy {
            return None;
        }
        self.stall += 1;
        self.total += 1;
        if self.stall >= 1_048_576 || self.total >= 16_777_216 {
            return Some(self.finish(Outcome::Uncertain, w::TIMEOUT));
        }
        if matches!(self.phase, Phase::Read | Phase::Write) {
            return self.data_step(&mut send);
        }
        if self.awaiting {
            return None;
        }
        if self.req.is_none() {
            let limit = if self.phase == Phase::Abort {
                w::SEQ_MAX
            } else {
                w::SEQ_MAX - 1
            };
            if self.seq >= limit {
                return Some(self.finish(Outcome::Uncertain, w::STALE));
            }
            self.seq += 1;
            let (mut a, mut b, mut offset) = (0, 0, 0);
            let kind = match self.phase {
                Phase::Info => w::INFO,
                Phase::Load => {
                    a = self.generation as u64;
                    w::LOAD
                }
                Phase::Name => {
                    offset = self.offset as u32;
                    w::NAME
                }
                Phase::Read | Phase::Write => unreachable!(),
                Phase::End => w::END,
                Phase::Abort => w::ABORT,
                Phase::Create => {
                    a = u64::from_le_bytes(self.name[..8].try_into().unwrap());
                    b = u64::from_le_bytes(self.name[8..].try_into().unwrap());
                    w::CREATE
                }
                Phase::BeginSave => {
                    a = self.generation as u64;
                    b = self.len as u64;
                    w::BEGIN_SAVE
                }
                Phase::Commit => {
                    a = w::hash(&self.bytes[..self.len]);
                    w::COMMIT
                }
            };
            self.req = Some(w::Request {
                seq: self.seq,
                kind,
                object: if self.phase == Phase::Abort {
                    0
                } else {
                    self.object
                },
                offset,
                a,
                b,
            });
        }
        match send(self.req.unwrap()) {
            Ok(()) => self.awaiting = true,
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => {}
            Err(_) => return Some(self.finish(Outcome::Uncertain, w::DENIED)),
        }
        None
    }
    pub fn receive(&mut self, caller: u32, value: u64) -> Option<Event> {
        if caller != 11 || !self.busy {
            return None;
        }
        if matches!(self.phase, Phase::Read | Phase::Write) {
            return self.data_receive(value);
        }
        if !self.awaiting {
            return None;
        }
        let r = self.req?;
        let value = w::decode_reply(value, r.seq)?;
        self.req = None;
        self.awaiting = false;
        self.clear_window();
        self.stall = 0;
        let v = match value {
            Ok(v) => v,
            Err(e) => return self.fail(e),
        };
        match self.phase {
            Phase::Info => {
                if self.saving
                    && !self.creating
                    && self.known[self.object as usize].map_or(self.object != 2, |g| g != v)
                {
                    return self.fail(w::STALE);
                }
                self.generation = v;
                self.phase = if self.saving {
                    Phase::BeginSave
                } else {
                    Phase::Load
                }
            }
            Phase::Load => {
                self.known[self.object as usize] = Some(self.generation);
                if v as usize > w::MAX {
                    return self.fail(w::INVALID);
                }
                self.len = v as usize;
                self.offset = 0;
                self.phase = Phase::Name
            }
            Phase::Name => {
                self.name[self.offset..self.offset + 4].copy_from_slice(&v.to_le_bytes());
                self.offset += 4;
                if self.offset == 16 {
                    self.offset = 0;
                    self.phase = if self.len == 0 {
                        Phase::End
                    } else {
                        Phase::Read
                    }
                }
            }
            Phase::Read | Phase::Write => unreachable!(),
            Phase::End => {
                if v != 0 {
                    return self.fail(w::INVALID);
                }
                return Some(self.finish(
                    if self.saving {
                        Outcome::Saved
                    } else {
                        Outcome::Loaded
                    },
                    0,
                ));
            }
            Phase::Create => {
                if !(3..15).contains(&v) {
                    return self.fail(w::INVALID);
                }
                self.object = v as u8;
                self.phase = Phase::Info
            }
            Phase::BeginSave => {
                self.known[self.object as usize] = Some(self.generation);
                if v != 0 {
                    return self.fail(w::INVALID);
                }
                self.phase = if self.len == 0 {
                    Phase::Commit
                } else {
                    Phase::Write
                }
            }
            Phase::Commit => {
                if v as usize != self.len {
                    return self.fail(w::INVALID);
                }
                self.phase = Phase::End
            }
            Phase::Abort => {
                if v != 0 {
                    return self.fail(w::INVALID);
                }
                self.fenced = false;
                self.quiescent = true;
                return Some(self.finish(Outcome::Cancelled, 0));
            }
        }
        None
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::*;
    #[test]
    fn partially_issued_window_leaves_a_fresh_abort_sequence() {
        let mut c = Client::new();
        c.busy = true;
        c.phase = Phase::Read;
        c.object = 3;
        c.len = 32;
        c.seq = w::SEQ_MAX - 3;
        let mut accepted = Vec::new();
        let e = c
            .step(|r| {
                accepted.push(r);
                Ok(())
            })
            .unwrap();
        assert_eq!(e.outcome, Outcome::Uncertain);
        assert_eq!(accepted.len(), 2);
        assert_eq!(accepted[1].seq, w::SEQ_MAX - 1);
        assert!(c.cancel());
        let mut abort = None;
        c.step(|r| {
            abort = Some(r);
            Ok(())
        });
        let abort = abort.unwrap();
        assert_eq!((abort.seq, abort.kind), (w::SEQ_MAX, w::ABORT));
        assert_eq!(
            c.receive(11, w::reply(abort.seq, Ok(0))).unwrap().outcome,
            Outcome::Cancelled
        );
        assert!(!c.cancel());
        assert!(c.load(3, 1));
        assert_eq!(
            c.step(|_| panic!("sequence wrapped")).unwrap().outcome,
            Outcome::Uncertain
        );
    }
}
