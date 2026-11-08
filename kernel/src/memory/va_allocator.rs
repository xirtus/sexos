// kernel/src/memory/va_allocator.rs
//! Monotonic GLOBAL_VAS address reservation only. This neither maps memory nor
//! grants access, allocates physical frames, or creates a capability.
use core::sync::atomic::{AtomicU64, Ordering};

const SHARED_VA_START:u64=0x_4000_0000_0000;
/// Reservations end where the kernel heap's fixed range begins (`HEAP_START`), so a
/// large reservation can never run into it or the program records above it.
pub const SHARED_VA_END:u64=0x_4444_4444_0000;
const USER_VA_END:u64=SHARED_VA_END;
const PAGE_SIZE:u64=4096;
static NEXT_VA:AtomicU64=AtomicU64::new(SHARED_VA_START);

fn allocate_from(cursor:&AtomicU64,size:usize)->Option<u64>{
    let bytes=u64::try_from(size).ok()?;
    if bytes==0{return None;}
    let rounded=bytes.checked_add(PAGE_SIZE-1)? & !(PAGE_SIZE-1);
    let mut current=cursor.load(Ordering::Relaxed);
    loop {
        if current<SHARED_VA_START || current>=USER_VA_END || current%PAGE_SIZE!=0{return None;}
        let next=current.checked_add(rounded)?;
        if next>USER_VA_END{return None;}
        match cursor.compare_exchange_weak(current,next,Ordering::SeqCst,Ordering::Relaxed){
            Ok(_)=>return Some(current),
            Err(observed)=>current=observed,
        }
    }
}

pub fn allocate_va(size:usize)->Option<u64>{allocate_from(&NEXT_VA,size)}

// Observation only; the cursor itself confers no authority.
pub fn current_va_cursor()->u64{NEXT_VA.load(Ordering::Relaxed)}

#[cfg(test)]
mod tests {
    use super::*;
    fn unchanged(initial:u64,size:usize){
        let cursor=AtomicU64::new(initial);
        assert_eq!(allocate_from(&cursor,size),None);
        assert_eq!(cursor.load(Ordering::SeqCst),initial);
    }
    #[test]
    fn page_rounding_preserves_exact_start_and_nonoverlap(){
        let cursor=AtomicU64::new(SHARED_VA_START);
        let mut expected=SHARED_VA_START;
        for(size,pages)in[(1usize,1u64),(4095,1),(4096,1),(4097,2),(8192,2)]{
            assert_eq!(allocate_from(&cursor,size),Some(expected));
            expected+=pages*PAGE_SIZE;assert_eq!(cursor.load(Ordering::SeqCst),expected);
        }
    }
    #[test]
    fn zero_and_rounding_overflow_leave_cursor_unchanged(){
        unchanged(SHARED_VA_START,0);
        #[cfg(target_pointer_width="64")]
        for size in[usize::MAX,usize::MAX-4094,usize::MAX-4095]{unchanged(SHARED_VA_START,size);}
    }
    #[test]
    fn final_page_can_be_reserved_once_then_arena_is_exhausted(){
        let cursor=AtomicU64::new(USER_VA_END-PAGE_SIZE);
        assert_eq!(allocate_from(&cursor,4096),Some(USER_VA_END-PAGE_SIZE));
        assert_eq!(cursor.load(Ordering::SeqCst),USER_VA_END);
        assert_eq!(allocate_from(&cursor,1),None);assert_eq!(cursor.load(Ordering::SeqCst),USER_VA_END);
        unchanged(USER_VA_END-PAGE_SIZE,4097);
    }
    #[test]
    fn invalid_or_noncanonical_cursor_cannot_wrap_or_escape_arena(){
        for initial in[0,SHARED_VA_START-PAGE_SIZE,SHARED_VA_START-1,SHARED_VA_START+1,USER_VA_END,USER_VA_END+PAGE_SIZE,u64::MAX,u64::MAX-4095]{
            unchanged(initial,4096);
        }
    }
    #[test]
    fn exact_remaining_arena_succeeds_but_oversized_reservation_does_not(){
        let cursor=AtomicU64::new(USER_VA_END-2*PAGE_SIZE);
        assert_eq!(allocate_from(&cursor,8192),Some(USER_VA_END-2*PAGE_SIZE));
        assert_eq!(cursor.load(Ordering::SeqCst),USER_VA_END);
        unchanged(USER_VA_END-2*PAGE_SIZE,8193);
        #[cfg(target_pointer_width="64")]
        unchanged(SHARED_VA_START,(USER_VA_END-SHARED_VA_START+PAGE_SIZE)as usize);
    }
    #[test]
    fn contended_successes_are_distinct_page_reservations(){
        let cursor=std::sync::Arc::new(AtomicU64::new(SHARED_VA_START));
        let mut workers=Vec::new();
        for _ in 0..8{
            let cursor=std::sync::Arc::clone(&cursor);
            workers.push(std::thread::spawn(move||{
                (0..128).map(|_|allocate_from(&cursor,1).unwrap()).collect::<Vec<_>>()
            }));
        }
        let mut starts=Vec::new();for worker in workers{starts.extend(worker.join().unwrap());}starts.sort_unstable();
        assert_eq!(starts.len(),1024);
        for(index,start)in starts.into_iter().enumerate(){assert_eq!(start,SHARED_VA_START+index as u64*PAGE_SIZE);}
        assert_eq!(cursor.load(Ordering::SeqCst),SHARED_VA_START+1024*PAGE_SIZE);
    }
}
