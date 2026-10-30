//! Finite, metadata-only delivery of the Browser's owned bitmap transaction.
//!
//! The caller keeps decoded pixels immutable until their unsent chunks finish.
//! No pixel slice is retained, no image is copied, and no allocation is made.
//! Clear abandons unsent image operations, so the caller may immediately erase
//! its pixels. Accepted COMMIT/CLEAR wait for their original exact Applied;
//! that receipt describes historical state publication, never current pixels.
use sex_pdx::bitmap_batch::{hash, Applied, Geometry, Request};

pub const SURFACE: u64 = 300;
pub const SENDS_PER_TURN: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub viewport: u64,
    pub source: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub sequence: u32,
    pub geometry: Option<Geometry>,
    pub hash: u64,
    pub metadata: Metadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation {
    pub opcode: u64,
    pub surface: u64,
    pub packed: u64,
    pub control: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Accepted,
    Full,
    /// A known pre-admission rejection, never an ambiguous accepted mutation.
    Rejected(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    Closed,
    InvalidGeometry,
    InvalidPixels,
    ClearPending,
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    Idle,
    Pending,
    Deferred,
    Rejected(u64),
    InvalidPixels,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Begin,
    Chunk(usize),
    Commit,
    Clear,
    Awaiting,
}

#[derive(Clone, Copy)]
struct Batch {
    receipt: Receipt,
    phase: Phase,
    unsent: Option<Request>,
}

fn erase_cached(batch: &mut Option<Batch>) {
    if let Some(Batch {
        unsent: Some(Request::Chunk { pixels, .. }),
        ..
    }) = batch
    {
        for pixel in pixels {
            // Only a two-word unsent register request is locally retained.
            unsafe {
                core::ptr::write_volatile(pixel, 0);
            }
        }
    }
}

pub struct Outbox {
    active: Option<Batch>,
    desired: Option<Batch>,
    sequence: u32,
    closed: bool,
}

impl Outbox {
    pub const fn new() -> Self {
        Self::with_sequence_floor(0)
    }

    /// Reconstruct only with the caller's retained monotonic sequence floor.
    /// Recreating an outbox at zero on the same live surface is not a reset.
    pub const fn with_sequence_floor(sequence: u32) -> Self {
        Self {
            active: None,
            desired: None,
            sequence,
            closed: false,
        }
    }

    /// Captures one immutable image description. Reflows coalesce only the
    /// not-yet-started desired batch, never an active chunk stream or receipt.
    pub fn queue(
        &mut self,
        pixels: &[u32],
        geometry: Geometry,
        metadata: Metadata,
    ) -> Result<u32, QueueError> {
        if self.closed {
            return Err(QueueError::Closed);
        }
        if !geometry.valid() {
            return Err(QueueError::InvalidGeometry);
        }
        if pixels.len() != geometry.len() {
            return Err(QueueError::InvalidPixels);
        }
        // An explicit erasure is a barrier. It cannot be overwritten by a
        // later repaint while its send or historical receipt is outstanding.
        if self
            .active
            .iter()
            .chain(self.desired.iter())
            .any(|b| b.receipt.geometry.is_none())
        {
            return Err(QueueError::ClearPending);
        }
        let digest = hash(pixels);
        if let Some(batch) = self.desired.as_ref().or(self.active.as_ref()) {
            if batch.receipt.geometry == Some(geometry)
                && batch.receipt.hash == digest
                && batch.receipt.metadata == metadata
            {
                return Ok(batch.receipt.sequence);
            }
        }
        let sequence = self.allocate_sequence()?;
        let batch = Batch {
            receipt: Receipt {
                sequence,
                geometry: Some(geometry),
                hash: digest,
                metadata,
            },
            phase: Phase::Begin,
            unsent: None,
        };
        if self.active.is_none() {
            self.active = Some(batch);
            self.desired = None;
        } else {
            self.desired = Some(batch);
        }
        Ok(sequence)
    }

    /// Stops all unsent image work before returning. The caller may immediately
    /// scrub/drop decoded pixels, even while an old COMMIT receipt is pending.
    pub fn clear(&mut self, metadata: Metadata) -> Result<u32, QueueError> {
        if self.closed {
            return Err(QueueError::Closed);
        }
        if let Some(batch) = self.desired.as_ref().or(self.active.as_ref()) {
            if batch.receipt.geometry.is_none() && batch.receipt.metadata == metadata {
                return Ok(batch.receipt.sequence);
            }
        }
        let waiting = self
            .active
            .as_ref()
            .is_some_and(|b| b.phase == Phase::Awaiting);
        if self.sequence == u32::MAX {
            // Exhaustion still stops unsent image work, but must not discard
            // an already reserved Clear barrier or accepted historical ACK.
            if self
                .desired
                .as_ref()
                .is_some_and(|b| b.receipt.geometry.is_some())
            {
                self.desired = None;
            }
            if !waiting
                && self
                    .active
                    .as_ref()
                    .is_some_and(|b| b.receipt.geometry.is_some())
            {
                erase_cached(&mut self.active);
                self.active = None;
            }
            return Err(QueueError::SequenceExhausted);
        }
        self.desired = None;
        if !waiting {
            erase_cached(&mut self.active);
            self.active = None;
        }
        let sequence = self.allocate_sequence()?;
        let batch = Batch {
            receipt: Receipt {
                sequence,
                geometry: None,
                hash: 0,
                metadata,
            },
            phase: Phase::Clear,
            unsent: None,
        };
        if waiting {
            self.desired = Some(batch);
        } else {
            self.active = Some(batch);
        }
        Ok(sequence)
    }

    pub fn pending(&self) -> bool {
        self.active.is_some() || self.desired.is_some()
    }

    pub fn sequence(&self) -> u32 {
        self.sequence
    }

    /// No success is inferred from send admission. Only the canonical display
    /// caller1/surface300/active-sequence receipt completes an accepted batch.
    pub fn receive(&mut self, caller: u32, word: u64) -> Option<Receipt> {
        if self.closed {
            return None;
        }
        let applied = Applied::decode(caller, word)?;
        let active = self.active.as_ref()?;
        if active.phase != Phase::Awaiting
            || applied.surface as u64 != SURFACE
            || applied.sequence != active.receipt.sequence
        {
            return None;
        }
        let receipt = active.receipt;
        self.active = None;
        Some(receipt)
    }

    /// At most min(budget,4) send attempts. Full preserves the exact unsent
    /// register request. Awaiting COMMIT/CLEAR and Clear never read pixels.
    /// The caller retains the active image until its chunks finish or clear().
    pub fn poll(
        &mut self,
        pixels: &[u32],
        budget: usize,
        mut send: impl FnMut(Operation) -> Admission,
    ) -> Poll {
        if self.closed {
            return Poll::Closed;
        }
        if self.active.is_none() {
            self.active = self.desired.take();
        }
        let Some(batch) = self.active.as_ref() else {
            return Poll::Idle;
        };
        if batch.phase == Phase::Awaiting {
            return Poll::Pending;
        }
        // Validate once per turn, including an unsent cached chunk after Full.
        // This remains bounded by the supported 2,304-pixel asset, not input IO.
        if matches!(batch.phase, Phase::Begin | Phase::Chunk(_)) {
            let geometry = batch.receipt.geometry.unwrap();
            if pixels.len() != geometry.len() || hash(pixels) != batch.receipt.hash {
                erase_cached(&mut self.active);
                self.active = None;
                return Poll::InvalidPixels;
            }
        }
        for _ in 0..budget.min(SENDS_PER_TURN) {
            let batch = self.active.as_mut().unwrap();
            if batch.phase == Phase::Awaiting {
                return Poll::Pending;
            }
            let request = match batch.unsent {
                Some(request) => request,
                None => {
                    let sequence = batch.receipt.sequence;
                    let request = match batch.phase {
                        Phase::Begin => Request::Begin {
                            sequence,
                            geometry: batch.receipt.geometry.unwrap(),
                        },
                        Phase::Chunk(offset) => {
                            let count = (pixels.len() - offset).min(2);
                            let mut pair = [0; 2];
                            pair[..count].copy_from_slice(&pixels[offset..offset + count]);
                            Request::Chunk {
                                sequence,
                                offset,
                                count,
                                pixels: pair,
                            }
                        }
                        Phase::Commit => Request::Commit {
                            sequence,
                            hash: batch.receipt.hash,
                        },
                        Phase::Clear => Request::Clear { sequence },
                        Phase::Awaiting => unreachable!(),
                    };
                    batch.unsent = Some(request);
                    request
                }
            };
            let (opcode, packed, control) = request.wire();
            match send(Operation {
                opcode,
                surface: SURFACE,
                packed,
                control,
            }) {
                Admission::Full => return Poll::Deferred,
                Admission::Rejected(error) => {
                    erase_cached(&mut self.active);
                    self.active = None;
                    return Poll::Rejected(error);
                }
                Admission::Accepted => {
                    batch.unsent = None;
                    batch.phase = match request {
                        Request::Begin { .. } => Phase::Chunk(0),
                        Request::Chunk { offset, count, .. } => {
                            let next = offset + count;
                            if next == batch.receipt.geometry.unwrap().len() {
                                Phase::Commit
                            } else {
                                Phase::Chunk(next)
                            }
                        }
                        Request::Commit { .. } | Request::Clear { .. } => Phase::Awaiting,
                    };
                }
            }
        }
        Poll::Pending
    }

    /// Terminal local fence, not a display Clear receipt. The app lifecycle is
    /// responsible for retiring display-owned staging/published storage.
    pub fn close(&mut self) {
        erase_cached(&mut self.active);
        erase_cached(&mut self.desired);
        self.active = None;
        self.desired = None;
        self.closed = true;
    }

    fn allocate_sequence(&mut self) -> Result<u32, QueueError> {
        let next = self
            .sequence
            .checked_add(1)
            .ok_or(QueueError::SequenceExhausted)?;
        self.sequence = next;
        Ok(next)
    }
}
