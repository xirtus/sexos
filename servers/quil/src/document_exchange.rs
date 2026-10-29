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
        self.error = 0;
        self.quiescent = false;
        self.busy = true;
        self.stall = 0;
        self.total = 0;
        true
    }
    pub fn cancel(&mut self) -> bool {
        if self.dead || self.seq == w::SEQ_MAX {
            return false;
        }
        self.req = None;
        self.awaiting = false;
        self.phase = Phase::Abort;
        self.busy = true;
        self.stall = 0;
        self.total = 0;
        true
    }
    pub fn close(&mut self) {
        self.bytes.fill(0);
        self.name.fill(0);
        self.busy = false;
        self.req = None;
        self.awaiting = false;
        self.dead = true;
    }
    fn finish(&mut self, outcome: Outcome, error: u32) -> Event {
        self.busy = false;
        self.req = None;
        self.awaiting = false;
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
    pub fn step(&mut self, mut send: impl FnMut(w::Request) -> Result<(), u64>) -> Option<Event> {
        if !self.busy {
            return None;
        }
        self.stall += 1;
        self.total += 1;
        if self.stall >= 1_048_576 || self.total >= 16_777_216 {
            return Some(self.finish(Outcome::Uncertain, w::TIMEOUT));
        }
        if self.awaiting {
            return None;
        }
        if self.req.is_none() {
            if self.seq == w::SEQ_MAX {
                self.dead = true;
                return self.fail(w::STALE);
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
                Phase::Read => {
                    offset = self.offset as u32;
                    a = (self.len - self.offset).min(4) as u64;
                    w::READ
                }
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
                Phase::Write => {
                    offset = self.offset as u32;
                    let n = (self.len - self.offset).min(16);
                    let mut raw = [0; 16];
                    raw[..n].copy_from_slice(&self.bytes[self.offset..self.offset + n]);
                    a = u64::from_le_bytes(raw[..8].try_into().unwrap());
                    b = u64::from_le_bytes(raw[8..].try_into().unwrap());
                    w::WRITE
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
        if caller != 11 || !self.awaiting {
            return None;
        }
        let r = self.req?;
        let value = w::decode_reply(value, r.seq)?;
        self.req = None;
        self.awaiting = false;
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
            Phase::Read => {
                let n = (self.len - self.offset).min(4);
                if n < 4 && v >> (n * 8) != 0 {
                    return self.fail(w::INVALID);
                }
                self.bytes[self.offset..self.offset + n].copy_from_slice(&v.to_le_bytes()[..n]);
                self.offset += n;
                if self.offset == self.len {
                    self.phase = Phase::End
                }
            }
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
            Phase::Write => {
                let n = (self.len - self.offset).min(16);
                if v as usize != n {
                    return self.fail(w::INVALID);
                }
                self.offset += n;
                if self.offset == self.len {
                    self.phase = Phase::Commit
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
