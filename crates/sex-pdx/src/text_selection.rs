//! Pure bounded byte selection shared by document consumers.
//!
//! Source is correlation data, never a capability or endpoint. The owning app
//! must assign a fresh generation for each distinct document, even if both
//! documents have revision zero, and advance revision after any byte mutation.
//! This model never owns or retains a body, allocates, validates text formats,
//! interprets glyphs, or publishes anything to a clipboard service.

pub const MAX_BYTES: usize = 12_288;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub generation: u64,
    pub revision: u64,
}

impl Source {
    pub const fn valid(self) -> bool {
        self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unbound,
    InvalidSource,
    WrongSource,
    BodyTooLarge,
    WrongBodyLength,
    InvalidPosition,
    DestinationTooSmall,
    SnapshotChanged,
    InvalidSelectionLength,
    CapacityExceeded,
}

/// Canonical immutable capture of a selection and its exact source version.
/// Private fields prevent constructing out-of-bounds snapshots through the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    source: Source,
    len: usize,
    anchor: usize,
    caret: usize,
}

impl Snapshot {
    pub const fn source(self) -> Source {
        self.source
    }
    pub const fn len(self) -> usize {
        self.len
    }
    pub const fn anchor(self) -> usize {
        self.anchor
    }
    pub const fn caret(self) -> usize {
        self.caret
    }
    /// Half-open byte range; selection direction remains in anchor/caret.
    pub fn range(self) -> (usize, usize) {
        (self.anchor.min(self.caret), self.anchor.max(self.caret))
    }
}

pub struct Selection {
    bound: Option<Snapshot>,
}

impl Selection {
    pub const fn new() -> Self {
        Self { bound: None }
    }

    /// Explicitly bind/rebind a document version and collapse at its first byte.
    /// Invalid source/size leaves any previous selection completely unchanged.
    pub fn bind(&mut self, source: Source, len: usize) -> Result<(), Error> {
        if !source.valid() {
            return Err(Error::InvalidSource);
        }
        if len > MAX_BYTES {
            return Err(Error::BodyTooLarge);
        }
        self.bound = Some(Snapshot {
            source,
            len,
            anchor: 0,
            caret: 0,
        });
        Ok(())
    }

    pub fn rebind(&mut self, source: Source, len: usize) -> Result<(), Error> {
        self.bind(source, len)
    }

    /// Drop all source/range state when a document is unloaded or an app closes.
    /// Collapsing an active range instead uses an unshifted move.
    pub fn clear(&mut self) {
        self.bound = None;
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.bound
    }
    pub fn range(&self) -> Option<(usize, usize)> {
        self.bound.map(Snapshot::range)
    }

    /// Pending paste may proceed only if the whole captured state is identical.
    /// The caller must also own current document bytes matching this source.
    pub fn unchanged(&self, expected: Snapshot) -> bool {
        self.bound == Some(expected)
    }

    pub fn move_to(&mut self, source: Source, position: usize, extend: bool) -> Result<(), Error> {
        let mut next = self.current(source)?;
        if position > next.len {
            return Err(Error::InvalidPosition);
        }
        next.caret = position;
        if !extend {
            next.anchor = position;
        }
        self.bound = Some(next);
        Ok(())
    }

    pub fn left(&mut self, source: Source, extend: bool) -> Result<(), Error> {
        let current = self.current(source)?;
        let position = if !extend && current.anchor != current.caret {
            current.range().0
        } else {
            current.caret.saturating_sub(1)
        };
        self.move_to(source, position, extend)
    }

    pub fn right(&mut self, source: Source, extend: bool) -> Result<(), Error> {
        let current = self.current(source)?;
        let position = if !extend && current.anchor != current.caret {
            current.range().1
        } else {
            current.caret.saturating_add(1).min(current.len)
        };
        self.move_to(source, position, extend)
    }

    pub fn home(&mut self, source: Source, extend: bool) -> Result<(), Error> {
        self.move_to(source, 0, extend)
    }

    pub fn end(&mut self, source: Source, extend: bool) -> Result<(), Error> {
        let len = self.current(source)?.len;
        self.move_to(source, len, extend)
    }

    pub fn select_all(&mut self, source: Source) -> Result<(), Error> {
        let mut current = self.current(source)?;
        current.anchor = 0;
        current.caret = current.len;
        self.bound = Some(current);
        Ok(())
    }

    /// Copy the entire selected range or nothing. All validation precedes the
    /// first output write; the remainder of out stays untouched. Bytes remain
    /// opaque, including controls, NUL and non-ASCII values.
    pub fn copy(&self, source: Source, body: &[u8], out: &mut [u8]) -> Result<usize, Error> {
        let current = self.current(source)?;
        if body.len() > MAX_BYTES {
            return Err(Error::BodyTooLarge);
        }
        if body.len() != current.len {
            return Err(Error::WrongBodyLength);
        }
        let (first, end) = current.range();
        let count = end - first;
        if out.len() < count {
            return Err(Error::DestinationTooSmall);
        }
        out[..count].copy_from_slice(&body[first..end]);
        Ok(count)
    }

    /// Validate an exact pending-paste snapshot and full replacement size.
    /// This helper never mutates body bytes or selection state on success/error.
    pub fn replacement_len(&self, expected: Snapshot, incoming: usize) -> Result<usize, Error> {
        if !self.unchanged(expected) {
            return Err(Error::SnapshotChanged);
        }
        let (first, end) = expected.range();
        replacement_len(expected.len, end - first, incoming)
    }

    fn current(&self, source: Source) -> Result<Snapshot, Error> {
        if !source.valid() {
            return Err(Error::InvalidSource);
        }
        let current = self.bound.ok_or(Error::Unbound)?;
        if current.source != source {
            return Err(Error::WrongSource);
        }
        Ok(current)
    }
}

/// Preflight an entire byte replacement; callers must not perform a partial
/// write if this returns an error. No truncation, wrapping or body mutation.
pub fn replacement_len(len: usize, selected: usize, incoming: usize) -> Result<usize, Error> {
    if len > MAX_BYTES {
        return Err(Error::BodyTooLarge);
    }
    let remaining = len
        .checked_sub(selected)
        .ok_or(Error::InvalidSelectionLength)?;
    let result = remaining
        .checked_add(incoming)
        .ok_or(Error::CapacityExceeded)?;
    if result > MAX_BYTES {
        return Err(Error::CapacityExceeded);
    }
    Ok(result)
}
