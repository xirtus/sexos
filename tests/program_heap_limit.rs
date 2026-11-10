//! Production Talc source; test allocations explicitly use a private allocator.
extern crate self as sex_pdx;
use core::alloc::{GlobalAlloc,Layout};
use std::cell::RefCell;
pub mod program_memory{
 use super::*;
 pub const PAGE:u64=4096;pub const MAX_PAGES_PER_CALL:u64=256;
 #[derive(Clone,Copy,Debug)]pub struct Memory{pub base:u64,pub pages:u64}
 impl Memory{pub fn end(&self)->u64{self.base+self.pages*PAGE}}
 pub struct World{pub base:u64,pub capacity:u64,pub held:u64,pub calls:Vec<u64>,pub reject_after:Option<usize>,pub accepted:usize,pub bad_span:bool,pub reported_base:Option<u64>,pub growth_base:Option<u64>}
 impl Drop for World{fn drop(&mut self){if self.base!=0{unsafe{std::alloc::dealloc(self.base as *mut u8,Layout::from_size_align((self.capacity*PAGE)as usize,PAGE as usize).unwrap())}}}}
 thread_local!{pub static WORLD:RefCell<World>=RefCell::new(World{base:0,capacity:0,held:0,calls:vec![],reject_after:None,accepted:0,bad_span:false,reported_base:None,growth_base:None});}
 pub fn reset(pages:u64){let layout=Layout::from_size_align((pages*PAGE)as usize,PAGE as usize).unwrap();let p=unsafe{std::alloc::alloc_zeroed(layout)};assert!(!p.is_null());WORLD.with(|w|*w.borrow_mut()=World{base:p as u64,capacity:pages,held:0,calls:vec![],reject_after:None,accepted:0,bad_span:false,reported_base:None,growth_base:None});}
 pub fn grow(pages:u64)->Result<Memory,u64>{WORLD.with(|w|{let mut w=w.borrow_mut();w.calls.push(pages);if pages>0{if w.reject_after.is_some_and(|n|w.accepted>=n)||w.held+pages>w.capacity{return Err(6)}w.held+=pages;w.accepted+=1;}
 let reported=if pages>0&&w.bad_span{w.held+1024}else{w.held};Ok(Memory{base:if pages>0{w.growth_base.or(w.reported_base).unwrap_or(w.base)}else{w.reported_base.unwrap_or(w.base)},pages:reported})})}
}
#[path="../crates/sex-app/src/heap.rs"]mod heap;
fn bounded(pages:usize)->heap::Heap{let a=heap::Heap::new(heap::Window::new());a.lock().source.set_limit(pages*4096).unwrap();a}
#[test]fn ordinary_source_retains_existing_growth_calls(){
 program_memory::reset(64);let a=heap::Heap::new(heap::Window::new());let l=Layout::from_size_align(100,8).unwrap();let p=unsafe{a.alloc(l)};assert!(!p.is_null());assert_eq!(program_memory::WORLD.with(|w|w.borrow().calls.clone()),vec![16]);unsafe{a.dealloc(p,l)};
}
#[test]fn cap_refuses_growth_and_live_allocations_remain_intact(){
 program_memory::reset(64);let a=bounded(16);let l=Layout::from_size_align(24*1024,16).unwrap();let p=unsafe{a.alloc(l)};assert!(!p.is_null());unsafe{p.write_bytes(0x5a,l.size())};let p2=unsafe{a.alloc(l)};assert!(!p2.is_null());let p3=unsafe{a.alloc(l)};assert!(p3.is_null());
 assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),16);assert!(unsafe{std::slice::from_raw_parts(p,l.size())}.iter().all(|b|*b==0x5a));unsafe{a.dealloc(p,l);a.dealloc(p2,l)};
}
#[test]fn small_caps_use_exact_fallback_without_exceeding_cap(){
 program_memory::reset(64);let a=bounded(2);let l=Layout::from_size_align(100,8).unwrap();let p=unsafe{a.alloc(l)};assert!(!p.is_null());assert!(program_memory::WORLD.with(|w|w.borrow().held)<=2);assert!(program_memory::WORLD.with(|w|w.borrow().calls.iter().filter(|&&n|n>0).all(|&n|n<=2)));unsafe{a.dealloc(p,l)};
}
#[test]fn partial_accepted_calls_are_charged_before_fallback(){
 program_memory::reset(1024);let a=bounded(300);program_memory::WORLD.with(|w|w.borrow_mut().reject_after=Some(1));let l=Layout::from_size_align(280*4096,8).unwrap();assert!(unsafe{a.alloc(l)}.is_null());assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),256);
 program_memory::WORLD.with(|w|w.borrow_mut().reject_after=None);assert!(unsafe{a.alloc(l)}.is_null());assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),256,"retry must charge the prior partially accepted pages");let small=Layout::from_size_align(32*4096,8).unwrap();let p=unsafe{a.alloc(small)};assert!(!p.is_null());assert!(program_memory::WORLD.with(|w|w.borrow().held)<=300);
 let fill=Layout::from_size_align(200*4096,8).unwrap();let p2=unsafe{a.alloc(fill)};assert!(!p2.is_null());let more=unsafe{a.alloc(small)};assert!(!more.is_null(),"valid partial chunks must remain extendable after ordinary OOM");assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),289);
 unsafe{a.dealloc(p,small);a.dealloc(p2,fill);a.dealloc(more,small)};
}
#[test]fn configuration_rejects_invalid_repeated_and_late_limits(){
 program_memory::reset(64);let a=heap::Heap::new(heap::Window::new());assert_eq!(a.lock().source.set_limit(0),Err(heap::LimitError::Invalid));assert_eq!(a.lock().source.set_limit(4097),Err(heap::LimitError::Invalid));a.lock().source.set_limit(65536).unwrap();assert_eq!(a.lock().source.set_limit(131072),Err(heap::LimitError::Configured));
 let l=Layout::from_size_align(100,8).unwrap();let p=unsafe{a.alloc(l)};assert!(!p.is_null());assert_eq!(a.lock().source.set_limit(131072),Err(heap::LimitError::Configured));unsafe{a.dealloc(p,l)};
}
#[test]fn unbounded_live_heap_cannot_be_reconfigured(){
 program_memory::reset(64);let a=heap::Heap::new(heap::Window::new());let l=Layout::from_size_align(100,8).unwrap();let p=unsafe{a.alloc(l)};assert!(!p.is_null());assert_eq!(a.lock().source.set_limit(65536),Err(heap::LimitError::Started));unsafe{a.dealloc(p,l)};
}
#[test]fn malformed_returned_span_cannot_raise_configured_limit(){
 program_memory::reset(4096);let a=bounded(16);program_memory::WORLD.with(|w|w.borrow_mut().bad_span=true);let l=Layout::from_size_align(100,8).unwrap();assert!(unsafe{a.alloc(l)}.is_null());assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),16);program_memory::WORLD.with(|w|w.borrow_mut().bad_span=false);assert!(unsafe{a.alloc(l)}.is_null());assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),16);
}

#[test]fn configuration_cannot_ignore_preexisting_kernel_heap_pages(){
 program_memory::reset(64);program_memory::grow(1).unwrap();let a=heap::Heap::new(heap::Window::new());assert_eq!(a.lock().source.set_limit(65536),Err(heap::LimitError::Started));
}

#[test]fn changed_query_base_cannot_extend_an_existing_heap_or_recover_into_another_span(){
 program_memory::reset(1024);let a=bounded(64);let live=Layout::from_size_align(1024,8).unwrap();let p=unsafe{a.alloc(live)};assert!(!p.is_null());unsafe{p.write_bytes(0x6d,live.size())};
 let request=Layout::from_size_align(64*1024,8).unwrap();program_memory::WORLD.with(|w|{let mut w=w.borrow_mut();w.reported_base=Some(w.base+128*4096);});
 assert!(unsafe{a.alloc(request)}.is_null(),"a changed grow(0) base must never extend the old Talc span");
 let charged=program_memory::WORLD.with(|w|w.borrow().held);assert_eq!(charged,16,"invalid query base must be rejected before another loan");program_memory::WORLD.with(|w|w.borrow_mut().reported_base=None);
 assert!(unsafe{a.alloc(request)}.is_null(),"invalid span failure must remain sticky on later allocation");assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),charged);
 assert!(unsafe{std::slice::from_raw_parts(p,live.size())}.iter().all(|b|*b==0x6d));unsafe{a.dealloc(p,live)};
}

#[test]fn malformed_growth_is_sticky_across_fallback_and_later_retry(){
 program_memory::reset(1024);let a=bounded(64);let live=Layout::from_size_align(1024,8).unwrap();let p=unsafe{a.alloc(live)};assert!(!p.is_null());unsafe{p.write_bytes(0x72,live.size())};
 let request=Layout::from_size_align(64*1024,8).unwrap();program_memory::WORLD.with(|w|{let mut w=w.borrow_mut();w.growth_base=Some(w.base+128*4096);});
 assert!(unsafe{a.alloc(request)}.is_null());let charged=program_memory::WORLD.with(|w|w.borrow().held);assert_eq!(charged,33,"only one bounded growth may occur before malformed reply is fenced");
 program_memory::WORLD.with(|w|w.borrow_mut().growth_base=None);assert!(unsafe{a.alloc(request)}.is_null());assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),charged);
 assert!(unsafe{std::slice::from_raw_parts(p,live.size())}.iter().all(|b|*b==0x72));unsafe{a.dealloc(p,live)};
}
#[test]fn unaccounted_query_range_cannot_be_claimed_as_allocator_memory(){
 program_memory::reset(1024);let a=bounded(64);let live=Layout::from_size_align(1024,8).unwrap();let p=unsafe{a.alloc(live)};assert!(!p.is_null());program_memory::WORLD.with(|w|w.borrow_mut().held+=1);
 let request=Layout::from_size_align(64*1024,8).unwrap();assert!(unsafe{a.alloc(request)}.is_null());assert_eq!(program_memory::WORLD.with(|w|w.borrow().held),17);unsafe{a.dealloc(p,live)};
}
