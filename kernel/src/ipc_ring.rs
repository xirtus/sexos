use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::mem::MaybeUninit;
use core::cell::UnsafeCell;

#[repr(align(64))]
struct CacheAligned<T>(T);

/// MPSC queue with lock-free producer admission and a bounded consumer turn.
/// The consumer guard rejects a concurrent dequeue without waiting. Payload
/// slots use explicit interior mutability and are erased before reuse.
///
/// BUG 2026-07-18: this was SPSC, but every client PD enqueues into a
/// server's single `message_ring` — multiple producers. Two producers could
/// load the same `write_idx`, write the same slot, and both return Ok: one
/// message silently lost (observed as storage requests vanishing while
/// sexfiles served another client; the caller then hung awaiting a reply).
/// A timer preemption between the index load and the store is enough — no
/// SMP needed.
///
/// Producers now claim a slot with CAS on `write_idx`, then publish it by
/// storing `pos + 1` into the slot's `seq`. The consumer only reads a slot
/// once its seq matches `read_idx + 1`, so a claimed-but-unwritten slot
/// reads as empty (bounded head-of-line wait) instead of garbage.
pub struct RingBuffer<T, const N: usize> {
    buffer: [UnsafeCell<MaybeUninit<T>>; N],
    reader: AtomicBool,
    /// Per-slot publish sequence: `pos + 1` once the item at absolute
    /// position `pos` is fully written. Absolute counters never wrap in
    /// practice (usize, 64-bit).
    seq: [AtomicUsize; N],

    // Aligned to separate cache lines to prevent false sharing.
    write_idx: CacheAligned<AtomicUsize>,
    read_idx: CacheAligned<AtomicUsize>,
}

// Producers uniquely claim a slot; a consumer holds `reader` until its value
// has moved out and the slot is erased. read_idx release permits producer reuse.
unsafe impl<T: Send, const N: usize> Sync for RingBuffer<T,N> {}
struct ReadTurn<'a>(&'a AtomicBool);
impl Drop for ReadTurn<'_>{fn drop(&mut self){self.0.store(false,Ordering::Release);}}

impl<T, const N: usize> RingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            buffer: [const { UnsafeCell::new(MaybeUninit::uninit()) }; N],
            reader: AtomicBool::new(false),
            seq: [const { AtomicUsize::new(0) }; N],
            write_idx: CacheAligned(AtomicUsize::new(0)),
            read_idx: CacheAligned(AtomicUsize::new(0)),
        }
    }

    /// Enqueues an item (multi-producer safe).
    pub fn enqueue(&self, item: T) -> Result<(), &'static str> {
        loop {
            let w = self.write_idx.0.load(Ordering::Acquire);
            let r = self.read_idx.0.load(Ordering::Acquire);

            if w.wrapping_sub(r) >= N {
                return Err("Ring: Buffer is full");
            }

            // Claim position w. On success no other producer can hold it:
            // r is monotonic, so the fullness check above still holds and
            // slot w % N has been consumed (its last use was pos w - N).
            if self
                .write_idx
                .0
                .compare_exchange_weak(w, w + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                unsafe {
                    let ptr = (*self.buffer[w % N].get()).as_mut_ptr();
                    ptr.write(item);
                }
                // Publish: consumer may read this slot only from here on.
                self.seq[w % N].store(w + 1, Ordering::Release);
                return Ok(());
            }
            // Lost the claim race — retry with fresh indices.
        }
    }

    /// Dequeues an item (single consumer).
    pub fn dequeue(&self) -> Option<T> {
        if self.reader.compare_exchange(false,true,Ordering::Acquire,Ordering::Relaxed).is_err(){return None;}
        let _turn=ReadTurn(&self.reader);
        let r = self.read_idx.0.load(Ordering::Relaxed);
        let w = self.write_idx.0.load(Ordering::Acquire);

        if r == w {
            return None;
        }

        // Slot claimed but not yet published by its producer: treat as
        // empty rather than reading a partially written message.
        if self.seq[r % N].load(Ordering::Acquire) != r + 1 {
            return None;
        }

        let item = unsafe {
            let slot=(*self.buffer[r % N].get()).as_mut_ptr();
            let item=slot.read();
            // The value has moved out; zero the uninitialized backing bytes,
            // including padding, before any producer can reclaim this slot.
            let bytes=core::mem::size_of::<T>();
            let words=if core::mem::align_of::<T>()>=core::mem::align_of::<u64>(){bytes/8}else{0};
            for offset in 0..words{core::ptr::write_volatile(slot.cast::<u64>().add(offset),0);}
            for offset in words*8..bytes{
                core::ptr::write_volatile(slot.cast::<u8>().add(offset),0);
            }
            item
        };

        self.read_idx.0.store(r + 1, Ordering::Release);
        Some(item)
    }

    /// Returns the number of queued items.
    pub fn len(&self) -> usize {
        let w = self.write_idx.0.load(Ordering::Acquire);
        let r = self.read_idx.0.load(Ordering::Acquire);
        w.saturating_sub(r)
    }
}

pub type SpscRing<T> = RingBuffer<T, 256>;

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn consumed_payload_bytes_are_zero_before_wrap_reuse(){
        let ring=RingBuffer::<u64,2>::new();ring.enqueue(u64::MAX).unwrap();ring.enqueue(19).unwrap();
        assert_eq!(ring.dequeue(),Some(u64::MAX));
        unsafe{let slot=(*ring.buffer[0].get()).as_ptr().cast::<u8>();for at in 0..8{assert_eq!(core::ptr::read_volatile(slot.add(at)),0);}}
        ring.enqueue(23).unwrap();assert_eq!(ring.dequeue(),Some(19));assert_eq!(ring.dequeue(),Some(23));assert_eq!(ring.dequeue(),None);
        for slot in &ring.buffer{unsafe{assert_eq!((*slot.get()).assume_init_read(),0);}}
    }
    #[test]fn busy_consumer_turn_cannot_claim_or_erase_a_slot(){
        let ring=RingBuffer::<u64,1>::new();ring.enqueue(7).unwrap();ring.reader.store(true,Ordering::Release);
        assert_eq!(ring.dequeue(),None);assert_eq!(ring.len(),1);ring.reader.store(false,Ordering::Release);assert_eq!(ring.dequeue(),Some(7));
    }
    #[test]fn concurrent_consumers_move_each_owned_value_once(){
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering as O}};
        let ring=Arc::new(RingBuffer::<usize,64>::new());let next=Arc::new(AtomicUsize::new(0));let sum=Arc::new(AtomicUsize::new(0));
        let producer={let ring=ring.clone();std::thread::spawn(move||for value in 1..=2000{while ring.enqueue(value).is_err(){std::thread::yield_now();}})};
        let readers:std::vec::Vec<_>=(0..4).map(|_|{let ring=ring.clone();let next=next.clone();let sum=sum.clone();std::thread::spawn(move||while next.load(O::Acquire)<2000{if let Some(value)=ring.dequeue(){sum.fetch_add(value,O::Relaxed);next.fetch_add(1,O::Release);}else{std::thread::yield_now();}})}).collect();
        producer.join().unwrap();for reader in readers{reader.join().unwrap();}assert_eq!(next.load(O::Relaxed),2000);assert_eq!(sum.load(O::Relaxed),2000*2001/2);
    }
}
