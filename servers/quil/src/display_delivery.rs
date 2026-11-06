//! One immutable active batch plus latest desired text, retained under Full.
use super::view::Text;
#[derive(Clone, Copy)]
struct Batch {
    text: Text,
    paint: u64,
    begun: bool,
    offset: usize,
    selected: bool,
    awaiting: Option<u16>,
    /// Sent as changes against the last applied text.
    delta: bool,
    /// A re-send is always complete.
    full: bool,
    /// Polls spent waiting for the reply; a lost reply re-sends the batch.
    waits: u32,
}
/// Polls to wait for Display's reply before sending the batch again.
const REPLY_RETRY: u32 = 4096;
pub struct Outbox {
    active: Option<Batch>,
    latest: Option<Batch>,
    dead: bool,
    last_paint: u64,
    deltas: bool,
    applied: [u8; super::view::CAPACITY],
    applied_len: Option<usize>,
}
impl Outbox {
    pub const fn new() -> Self {
        Self {
            active: None,
            latest: None,
            dead: false,
            last_paint: 0,
            deltas: false,
            applied: [0; super::view::CAPACITY],
            applied_len: None,
        }
    }
    /// Send only changed chunks once Display holds an applied text.
    pub fn enable_delta(&mut self) {
        self.deltas = true;
    }
    pub fn queue(&mut self, text: Text, paint: u64) {
        if self.dead {
            return;
        }
        if paint == 0
            || paint > u32::MAX as u64
            || text.len > super::view::CAPACITY
            || text
                .selection
                .is_some_and(|(start, end)| start >= end || end > text.len)
        {
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
            selected: false,
            awaiting: None,
            delta: false,
            full: false,
            waits: 0,
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
        // A reply may land while a re-send of the same paint is under way.
        if b.awaiting.is_some_and(|surface| surface != applied.surface)
            || !b.begun
            || b.paint != applied.sequence as u64
        {
            return None;
        }
        let done = (b.paint, b.text.len);
        if self.deltas {
            self.applied[..b.text.len].copy_from_slice(&b.text.bytes[..b.text.len]);
            self.applied_len = Some(b.text.len);
        }
        b.text.bytes.fill(0);
        self.active = None;
        Some(done)
    }
    pub fn pending(&self) -> bool {
        !self.dead && (self.active.is_some() || self.latest.is_some())
    }
    /// A fresh, exact shell viewport notice follows its display create/chrome
    /// admission. Abandon any upload whose old surface binding was scrubbed;
    /// only a newly rendered, strictly newer paint may resume publication.
    pub fn rebind(&mut self) {
        if self.dead {
            return;
        }
        self.applied_len = None;
        if let Some(b) = self.active.as_mut() {
            b.text.bytes.fill(0);
        }
        self.active = None;
        if let Some(b) = self.latest.as_mut() {
            b.text.bytes.fill(0);
        }
        self.latest = None;
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
        for _ in 0..128 {
            if self.active.is_none() {
                self.active = self.latest.take()
            }
            let Some(b) = self.active.as_mut() else {
                return;
            };
            if b.awaiting.is_some() {
                b.waits += 1;
                if b.waits < REPLY_RETRY {
                    return;
                }
                // No reply: Display refused this batch while it still owed the
                // previous one, or scrubbed the surface. Send the newest again.
                if let Some(next) = self.latest.take() {
                    b.text.bytes.fill(0);
                    *b = next;
                } else {
                    // Display holds a failed delta poisoned under its old number.
                    if b.delta {
                        self.last_paint += 1;
                        b.paint = self.last_paint;
                    }
                    (b.begun, b.offset, b.selected, b.awaiting, b.waits, b.delta, b.full) = (false, 0, false, None, 0, false, true);
                }
            }
            let applied_len = self.applied_len.unwrap_or(0);
            let b = self.active.as_mut().unwrap();
            if b.begun && b.delta {
                while b.offset < b.text.len {
                    let end = b.offset + (b.text.len - b.offset).min(8);
                    if end > applied_len || b.text.bytes[b.offset..end] != self.applied[b.offset..end] {
                        break;
                    }
                    b.offset = end;
                }
            }
            let start_delta = !b.begun && !b.full && self.deltas && self.applied_len.is_some();
            let request = if start_delta {
                sex_pdx::text_batch::Request::BeginDelta {
                    sequence: b.paint as u32,
                    len: b.text.len,
                    color: sex_pdx::desktop_style::TEXT,
                }
            } else if !b.begun {
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
            } else if !b.selected && b.text.selection.is_some() {
                let (start, end) = b.text.selection.unwrap();
                sex_pdx::text_batch::Request::Select {
                    sequence: b.paint as u32,
                    start,
                    end,
                }
            } else {
                let hash = if let Some((start, end)) = b.text.selection {
                    let Some(hash) =
                        sex_pdx::text_batch::selected_hash(&b.text.bytes[..b.text.len], start, end)
                    else {
                        self.close();
                        return;
                    };
                    hash
                } else {
                    sex_pdx::text_batch::hash(&b.text.bytes[..b.text.len])
                };
                sex_pdx::text_batch::Request::Commit {
                    sequence: b.paint as u32,
                    hash,
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
                b.delta = start_delta;
            } else if b.offset < b.text.len {
                b.offset += (b.text.len - b.offset).min(8);
            } else if !b.selected && b.text.selection.is_some() {
                b.selected = true;
            } else {
                done(b.paint, b.text.len);
                b.awaiting = Some(surface);
                return;
            }
        }
    }
}
