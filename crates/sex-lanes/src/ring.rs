// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Single-producer, single-consumer rings in memory two domains share.
//!
//! The memory is a [`Header`] followed by `entries` slots of `T`. The producer owns
//! `tail` and the slots between `head` and `tail + entries`; the consumer owns `head`.
//! Indices run freely (wrapping `u32`) and are masked on use, so a full ring and an
//! empty one differ.
//!
//! The other side may be another protection domain, so it is not trusted: a peer
//! that writes nonsense into the cursors makes this side see [`Corrupt`], never read
//! outside the slots. What a slot *holds* is the receiver's to check (a buffer
//! address must lie in memory the sender lent it, and so on).
//!
//! Wake-ups follow the usual "need wakeup" handshake (io_uring's SQ_NEED_WAKEUP,
//! virtio's event suppression): a consumer about to sleep sets [`NEED_WAKEUP`] and
//! looks once more; a producer that published entries and sees the flag signals the
//! consumer (an IPC v2 notification) and clears it. While the consumer is busy the
//! flag is clear and the producer makes no system call at all.
use core::marker::PhantomData;
use core::sync::atomic::{fence, AtomicU32, Ordering};

/// Set by a consumer that is going to sleep; the producer then signals it.
pub const NEED_WAKEUP: u32 = 1;

/// One cursor alone on its cache line, so producer and consumer never write the same line.
#[repr(C, align(64))]
pub struct Cursor {
    pub value: AtomicU32,
}

impl Cursor {
    pub const fn new() -> Self { Self { value: AtomicU32::new(0) } }
}

impl Default for Cursor {
    fn default() -> Self { Self::new() }
}

/// The ring's shared header. `entries` is written once by whoever sets the ring up.
#[repr(C)]
pub struct Header {
    /// Next slot the producer fills.
    pub tail: Cursor,
    /// Next slot the consumer reads.
    pub head: Cursor,
    /// [`NEED_WAKEUP`], owned by the consumer.
    pub flags: Cursor,
    /// Slots, a power of two.
    pub entries: AtomicU32,
}

/// The peer left the cursors in a state no honest peer can reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corrupt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushError {
    Full,
    Corrupt,
}

/// Bytes a ring of `entries` slots of `T` needs, header included.
pub const fn bytes_for<T>(entries: u32) -> usize {
    let header = core::mem::size_of::<Header>();
    let align = core::mem::align_of::<T>();
    let slots = (header + align - 1) / align * align;
    slots + entries as usize * core::mem::size_of::<T>()
}

struct Raw<T> {
    header: *const Header,
    slots: *mut T,
    mask: u32,
}

impl<T: Copy> Raw<T> {
    /// # Safety
    /// `base` points to at least `bytes_for::<T>(entries)` bytes that stay mapped,
    /// aligned for `Header` and `T`.
    unsafe fn new(base: *mut u8, entries: u32) -> Option<Self> {
        if entries == 0 || !entries.is_power_of_two() || base as usize % core::mem::align_of::<Header>() != 0 { return None; }
        let header = base as *const Header;
        let offset = bytes_for::<T>(entries) - entries as usize * core::mem::size_of::<T>();
        Some(Self { header, slots: base.add(offset) as *mut T, mask: entries - 1 })
    }
    fn h(&self) -> &Header { unsafe { &*self.header } }
    fn entries(&self) -> u32 { self.mask + 1 }
}

/// Sets up a fresh ring in `base`: cursors zero, `entries` recorded.
///
/// # Safety
/// As [`Producer::attach`]; nobody may use the ring yet.
pub unsafe fn init<T: Copy>(base: *mut u8, entries: u32) -> bool {
    let Some(raw) = Raw::<T>::new(base, entries) else { return false };
    let h = raw.h();
    h.tail.value.store(0, Ordering::Relaxed);
    h.head.value.store(0, Ordering::Relaxed);
    h.flags.value.store(0, Ordering::Relaxed);
    h.entries.store(entries, Ordering::Release);
    true
}

/// The producing end. Only one may exist per ring.
pub struct Producer<'a, T> {
    raw: Raw<T>,
    /// Our own copy of the tail: the shared one is only ever written, never trusted.
    tail: u32,
    _life: PhantomData<&'a mut T>,
}

/// The consuming end. Only one may exist per ring.
pub struct Consumer<'a, T> {
    raw: Raw<T>,
    head: u32,
    _life: PhantomData<&'a mut T>,
}

impl<'a, T: Copy> Producer<'a, T> {
    /// # Safety
    /// `base` points to a ring set up by [`init`] with the same `entries`, mapped and
    /// writable for `'a`, and no other producer uses it.
    pub unsafe fn attach(base: *mut u8, entries: u32) -> Option<Self> {
        let raw = Raw::new(base, entries)?;
        if raw.h().entries.load(Ordering::Acquire) != entries { return None; }
        let tail = raw.h().tail.value.load(Ordering::Acquire);
        Some(Self { raw, tail, _life: PhantomData })
    }

    /// Slots free right now.
    pub fn free(&self) -> Result<u32, Corrupt> {
        let head = self.raw.h().head.value.load(Ordering::Acquire);
        let used = self.tail.wrapping_sub(head);
        if used > self.raw.entries() { return Err(Corrupt); }
        Ok(self.raw.entries() - used)
    }

    /// Writes `items` into free slots without publishing them; returns how many fit.
    fn stage(&mut self, items: &[T]) -> Result<u32, Corrupt> {
        let n = (items.len() as u32).min(self.free()?);
        for (i, item) in items.iter().take(n as usize).enumerate() {
            let at = self.tail.wrapping_add(i as u32) & self.raw.mask;
            unsafe { self.raw.slots.add(at as usize).write_volatile(*item); }
        }
        Ok(n)
    }

    /// Publishes everything staged up to `count` more entries and says whether the
    /// consumer asked to be woken (the caller then signals it).
    fn publish(&mut self, count: u32) -> bool {
        self.tail = self.tail.wrapping_add(count);
        self.raw.h().tail.value.store(self.tail, Ordering::Release);
        // Pairs with the consumer's fence in `prepare_sleep`: either it sees our tail,
        // or we see its flag.
        fence(Ordering::SeqCst);
        count != 0 && self.raw.h().flags.value.load(Ordering::Relaxed) & NEED_WAKEUP != 0
    }

    /// Adds one entry. `Ok(true)`: the consumer sleeps and must be signalled.
    pub fn push(&mut self, item: T) -> Result<bool, PushError> {
        match self.stage(core::slice::from_ref(&item)) {
            Ok(1) => Ok(self.publish(1)),
            Ok(_) => Err(PushError::Full),
            Err(Corrupt) => Err(PushError::Corrupt),
        }
    }

    /// Adds as many of `items` as fit, published at once. Returns (added, must signal).
    pub fn push_many(&mut self, items: &[T]) -> Result<(usize, bool), Corrupt> {
        let n = self.stage(items)?;
        Ok((n as usize, self.publish(n)))
    }
}

impl<'a, T: Copy> Consumer<'a, T> {
    /// # Safety
    /// As [`Producer::attach`], for the one consumer.
    pub unsafe fn attach(base: *mut u8, entries: u32) -> Option<Self> {
        let raw = Raw::new(base, entries)?;
        if raw.h().entries.load(Ordering::Acquire) != entries { return None; }
        let head = raw.h().head.value.load(Ordering::Acquire);
        Some(Self { raw, head, _life: PhantomData })
    }

    /// Entries waiting.
    pub fn ready(&self) -> Result<u32, Corrupt> {
        let tail = self.raw.h().tail.value.load(Ordering::Acquire);
        let n = tail.wrapping_sub(self.head);
        if n > self.raw.entries() { return Err(Corrupt); }
        Ok(n)
    }

    /// Takes up to `out.len()` entries; returns how many.
    pub fn pop_many(&mut self, out: &mut [T]) -> Result<usize, Corrupt> {
        let n = (out.len() as u32).min(self.ready()?);
        for (i, slot) in out.iter_mut().take(n as usize).enumerate() {
            let at = self.head.wrapping_add(i as u32) & self.raw.mask;
            *slot = unsafe { self.raw.slots.add(at as usize).read_volatile() };
        }
        if n != 0 {
            self.head = self.head.wrapping_add(n);
            self.raw.h().head.value.store(self.head, Ordering::Release);
        }
        Ok(n as usize)
    }

    pub fn pop(&mut self) -> Result<Option<T>, Corrupt> {
        let n = self.ready()?;
        if n == 0 { return Ok(None); }
        let at = self.head & self.raw.mask;
        let item = unsafe { self.raw.slots.add(at as usize).read_volatile() };
        self.head = self.head.wrapping_add(1);
        self.raw.h().head.value.store(self.head, Ordering::Release);
        Ok(Some(item))
    }

    /// About to sleep: asks for a wake-up, then looks once more. `Ok(true)`: the ring
    /// is still empty and the consumer may sleep; `Ok(false)`: entries arrived, keep
    /// going (the flag is cleared again).
    pub fn prepare_sleep(&mut self) -> Result<bool, Corrupt> {
        self.raw.h().flags.value.fetch_or(NEED_WAKEUP, Ordering::SeqCst);
        fence(Ordering::SeqCst);
        if self.ready()? == 0 { return Ok(true); }
        self.awake();
        Ok(false)
    }

    /// Back at work: producers need not signal.
    pub fn awake(&mut self) { self.raw.h().flags.value.fetch_and(!NEED_WAKEUP, Ordering::SeqCst); }
}

/// A block request on a lane: read or write `len` bytes at device byte `offset`
/// into or out of `buffer`, an address in memory the client lent the driver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockRequest {
    pub op: u8,
    pub flags: u8,
    /// The client's own name for the request; its completion carries it back.
    pub tag: u16,
    pub len: u32,
    pub offset: u64,
    pub buffer: u64,
}

pub const BLOCK_READ: u8 = 1;
pub const BLOCK_WRITE: u8 = 2;
pub const BLOCK_FLUSH: u8 = 3;
/// `flags`: make this write durable before completing it (FUA).
pub const BLOCK_FUA: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockCompletion {
    pub tag: u16,
    /// 0 on success; otherwise a `sex_pdx` block error code.
    pub status: u16,
    /// Bytes moved.
    pub len: u32,
}

/// A packet buffer on a lane: `len` bytes at `offset` in the lane's packet pool.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Packet {
    pub offset: u32,
    pub len: u16,
    pub flags: u16,
}
