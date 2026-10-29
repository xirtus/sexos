//! One immutable active batch plus latest desired text, retained under Full.
use super::view::Text;
#[derive(Clone, Copy)]
struct Batch {
    text: Text,
    paint: u64,
    begun: bool,
    offset: usize,
    awaiting: Option<u16>,
}
pub struct Outbox {
    active: Option<Batch>,
    latest: Option<Batch>,
    dead: bool,
    last_paint: u64,
}
impl Outbox {
    pub const fn new() -> Self {
        Self {
            active: None,
            latest: None,
            dead: false,
            last_paint: 0,
        }
    }
    pub fn queue(&mut self, text: Text, paint: u64) {
        if self.dead {
            return;
        }
        if paint == 0 || paint > u32::MAX as u64 || text.len > 512 {
            self.close();
            return;
        }
        if paint <= self.last_paint {
            return;
        }
        self.last_paint = paint;
        let b = Batch {
            text,
            paint,
            begun: false,
            offset: 0,
            awaiting: None,
        };
        if self.active.is_none() {
            // An ACK can leave latest queued until the next poll. A newer
            // queue in that gap supersedes it; never publish it afterward.
            self.latest = None;
            self.active = Some(b)
        } else {
            self.latest = Some(b)
        }
    }
    pub fn receive(&mut self, caller: u32, value: u64) -> Option<(u64, usize)> {
        let applied = sex_pdx::text_batch::Applied::decode(caller, value)?;
        let b = self.active.as_mut()?;
        if b.awaiting != Some(applied.surface) || b.paint != applied.sequence as u64 {
            return None;
        }
        let done = (b.paint, b.text.len);
        b.text.bytes.fill(0);
        self.active = None;
        Some(done)
    }
    pub fn pending(&self) -> bool {
        !self.dead && (self.active.is_some() || self.latest.is_some())
    }
    pub fn close(&mut self) {
        if let Some(b) = &mut self.active {
            b.text.bytes.fill(0)
        }
        if let Some(b) = &mut self.latest {
            b.text.bytes.fill(0)
        }
        self.active = None;
        self.latest = None;
        self.dead = true;
    }
    pub fn poll(
        &mut self,
        sid: u64,
        mut send: impl FnMut(u64, u64, u64, u64) -> Result<(), u64>,
        mut done: impl FnMut(u64, usize),
    ) {
        if self.dead {
            return;
        }
        let Ok(surface) = u16::try_from(sid) else {
            self.close();
            return;
        };
        if surface == 0 {
            self.close();
            return;
        }
        for _ in 0..4 {
            if self.active.is_none() {
                self.active = self.latest.take()
            }
            let Some(b) = self.active.as_ref() else {
                return;
            };
            if b.awaiting.is_some() {
                return;
            }
            let request = if !b.begun {
                sex_pdx::text_batch::Request::Begin {
                    sequence: b.paint as u32,
                    len: b.text.len,
                    color: sex_pdx::desktop_style::TEXT,
                }
            } else if b.offset < b.text.len {
                let count = (b.text.len - b.offset).min(8);
                let mut bytes = [0; 8];
                bytes[..count].copy_from_slice(&b.text.bytes[b.offset..b.offset + count]);
                sex_pdx::text_batch::Request::Chunk {
                    sequence: b.paint as u32,
                    offset: b.offset,
                    count,
                    bytes,
                }
            } else {
                sex_pdx::text_batch::Request::Commit {
                    sequence: b.paint as u32,
                    hash: sex_pdx::text_batch::hash(&b.text.bytes[..b.text.len]),
                }
            };
            let (op, a, packed) = request.wire();
            match send(op, sid, a, packed) {
                Ok(()) => {}
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => return,
                Err(_) => {
                    self.close();
                    return;
                }
            }
            let b = self.active.as_mut().unwrap();
            if !b.begun {
                b.begun = true;
            } else if b.offset < b.text.len {
                b.offset += (b.text.len - b.offset).min(8);
            } else {
                done(b.paint, b.text.len);
                b.awaiting = Some(surface);
                return;
            }
        }
    }
}
