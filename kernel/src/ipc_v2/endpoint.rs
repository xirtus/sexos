//! Endpoint: a rendezvous object one server thread receives on.
//!
//! The waiting receiver is a single atomic word, so a call that finds its server
//! parked takes it with one compare-and-swap and no lock: that is the fastpath the
//! syscall layer turns into a direct switch. Callers that arrive while the server
//! is busy park in a short FIFO under the endpoint's own spin lock, with their
//! message left in their saved registers (nothing is copied until the receive).
//!
//! No wakeup is lost: a receiver parks only under the lock after seeing the queue
//! empty, and a caller queues only under the lock after seeing no receiver.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::TaskId;

/// Callers that may wait on one endpoint at once. A caller that finds the queue
/// full gets [`SendOutcome::Full`] and retries after yielding.
pub const QUEUE_DEPTH: usize = 16;

const NO_WAITER: u64 = 0;
const CLOSED: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    /// Expects an answer: the receiver gets a one-shot reply capability.
    Call,
    /// One-way: nothing comes back.
    Send,
}

/// Who is calling, as the receiver learns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sender {
    pub task: TaskId,
    /// Minted into the caller's endpoint capability; the server's view of who calls.
    pub badge: u64,
    /// The caller's incarnation generation at call time; a reply to a caller whose
    /// generation moved on (it died) is refused.
    pub generation: u32,
    pub kind: CallKind,
}

impl Sender {
    const NONE: Self = Self { task: 0, badge: 0, generation: 0, kind: CallKind::Send };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// The receiver was waiting and is now the caller's to switch to.
    Rendezvous { receiver: TaskId },
    /// No receiver waiting: the caller is queued and should block.
    Queued,
    /// Queue full: the caller should yield and try again.
    Full,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvOutcome {
    /// A queued caller, now dequeued: copy its message and wake it (Send) or keep
    /// it blocked for the reply (Call).
    Message(Sender),
    /// Nothing queued: the receiver is now the endpoint's waiter and should block.
    Parked,
    /// Another thread is already waiting; an endpoint has one receiver at a time.
    Busy,
    Closed,
}

/// A task released by [`Endpoint::close`], to be woken with an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Released {
    Receiver(TaskId),
    Caller(Sender),
}

struct Queue {
    items: [Sender; QUEUE_DEPTH],
    head: usize,
    len: usize,
}

pub struct Endpoint {
    waiter: AtomicU64,
    lock: AtomicBool,
    queue: UnsafeCell<Queue>,
}

// The queue is only touched with `lock` held; `waiter` is atomic.
unsafe impl Sync for Endpoint {}

struct Guard<'a>(&'a Endpoint);

impl Drop for Guard<'_> {
    fn drop(&mut self) { self.0.lock.store(false, Ordering::Release); }
}

impl Guard<'_> {
    #[allow(clippy::mut_from_ref)]
    fn queue(&self) -> &mut Queue { unsafe { &mut *self.0.queue.get() } }
}

impl Default for Endpoint {
    fn default() -> Self { Self::new() }
}

impl Endpoint {
    pub const fn new() -> Self {
        Self {
            waiter: AtomicU64::new(NO_WAITER),
            lock: AtomicBool::new(false),
            queue: UnsafeCell::new(Queue { items: [Sender::NONE; QUEUE_DEPTH], head: 0, len: 0 }),
        }
    }

    fn lock(&self) -> Guard<'_> {
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        Guard(self)
    }

    fn take_waiter(&self) -> Result<Option<TaskId>, ()> {
        let current = self.waiter.load(Ordering::Acquire);
        match current {
            CLOSED => Err(()),
            NO_WAITER => Ok(None),
            w => match self.waiter.compare_exchange(w, NO_WAITER, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => Ok(Some(w - 1)),
                Err(CLOSED) => Err(()),
                // Lost to another caller or a cancel: the endpoint has no waiter now.
                Err(_) => Ok(None),
            },
        }
    }

    /// A caller arrives. Fastpath first: take a parked receiver without locking.
    pub fn send(&self, sender: Sender) -> SendOutcome {
        match self.take_waiter() {
            Err(()) => return SendOutcome::Closed,
            Ok(Some(receiver)) => return SendOutcome::Rendezvous { receiver },
            Ok(None) => {}
        }
        let guard = self.lock();
        // A receiver may have parked between the fastpath and the lock.
        match self.take_waiter() {
            Err(()) => return SendOutcome::Closed,
            Ok(Some(receiver)) => return SendOutcome::Rendezvous { receiver },
            Ok(None) => {}
        }
        let queue = guard.queue();
        if queue.len == QUEUE_DEPTH {
            return SendOutcome::Full;
        }
        queue.items[(queue.head + queue.len) % QUEUE_DEPTH] = sender;
        queue.len += 1;
        SendOutcome::Queued
    }

    /// The server asks for its next message.
    pub fn recv(&self, receiver: TaskId) -> RecvOutcome {
        debug_assert!(receiver < CLOSED - 1);
        let guard = self.lock();
        let waiter = self.waiter.load(Ordering::Acquire);
        if waiter == CLOSED {
            return RecvOutcome::Closed;
        }
        let queue = guard.queue();
        if queue.len > 0 {
            let sender = queue.items[queue.head];
            queue.head = (queue.head + 1) % QUEUE_DEPTH;
            queue.len -= 1;
            return RecvOutcome::Message(sender);
        }
        if waiter == receiver + 1 {
            return RecvOutcome::Parked; // still the waiter (woken spuriously, asking again)
        }
        if waiter != NO_WAITER {
            return RecvOutcome::Busy;
        }
        self.waiter.store(receiver + 1, Ordering::Release);
        RecvOutcome::Parked
    }

    /// The parked receiver stops waiting (timeout or exit). False means a caller
    /// already took it, so a message is on its way and the receiver must take it.
    pub fn cancel_recv(&self, receiver: TaskId) -> bool {
        self.waiter
            .compare_exchange(receiver + 1, NO_WAITER, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// A queued caller leaves (it died or gave up). Keeps the others in order.
    pub fn cancel_send(&self, task: TaskId) -> Option<Sender> {
        let guard = self.lock();
        let queue = guard.queue();
        let found = (0..queue.len).find(|&i| queue.items[(queue.head + i) % QUEUE_DEPTH].task == task)?;
        let removed = queue.items[(queue.head + found) % QUEUE_DEPTH];
        for i in found..queue.len - 1 {
            queue.items[(queue.head + i) % QUEUE_DEPTH] = queue.items[(queue.head + i + 1) % QUEUE_DEPTH];
        }
        queue.len -= 1;
        Some(removed)
    }

    /// Callers queued now (diagnostics and tests).
    pub fn queued(&self) -> usize { self.lock().queue().len }

    /// The parked receiver, if any (diagnostics and tests).
    pub fn waiter(&self) -> Option<TaskId> {
        match self.waiter.load(Ordering::Acquire) {
            NO_WAITER | CLOSED => None,
            w => Some(w - 1),
        }
    }

    pub fn is_closed(&self) -> bool { self.waiter.load(Ordering::Acquire) == CLOSED }

    /// Revocation or server death: every later call fails, and every task waiting
    /// here is handed to `release` to be woken with an error. Returns how many.
    pub fn close(&self, mut release: impl FnMut(Released)) -> usize {
        let guard = self.lock();
        let previous = self.waiter.swap(CLOSED, Ordering::AcqRel);
        let mut released = 0;
        if previous != NO_WAITER && previous != CLOSED {
            release(Released::Receiver(previous - 1));
            released += 1;
        }
        let queue = guard.queue();
        while queue.len > 0 {
            release(Released::Caller(queue.items[queue.head]));
            queue.head = (queue.head + 1) % QUEUE_DEPTH;
            queue.len -= 1;
            released += 1;
        }
        released
    }

    /// Makes a closed, drained endpoint usable again (a fresh object generation).
    pub fn reopen(&self) -> bool {
        let _guard = self.lock();
        self.waiter.compare_exchange(CLOSED, NO_WAITER, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
}
