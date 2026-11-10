// Host-owned aligned spans and explicit unsafe simulated held-region replies.
// This fixture exercises actual SDK validation; metadata is not native authority.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
pub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;
#[path="../crates/sex-pdx/src/display_frames.rs"]pub mod display_frames;
#[path="../servers/sexdisplay/src/frame_output.rs"]mod frame_output;
use display_frames::{FrameBacking,Layout};
use frame_output::Store;
struct Region{pointer:*mut u8,allocation:std::alloc::Layout,layout:Layout}
impl Region{
 fn new()->Self{
  let layout=Layout::new(19,7,76).unwrap();
  let allocation=std::alloc::Layout::from_size_align(layout.total_bytes()as usize,4096).unwrap();
  let pointer=unsafe{std::alloc::alloc(allocation)};assert!(!pointer.is_null());
  unsafe{core::ptr::write_bytes(pointer,0xa5,allocation.size());}
  Self{pointer,allocation,layout}
 }
 // Each test transfers this descriptor once and keeps Region alive until Store
 // and every borrowed pixel/target reference have been dropped.
 unsafe fn held(&self)->FrameBacking{
  use memory_region::*;
  let grant=RegionGrant{region_id:11,virtual_base:self.pointer as u64,length:self.layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE,dma_base:None,peer_pd:None};
  let info=resolve_info(Some(&grant)).unwrap();
  let view=resolve_view(Some(&grant),ViewRequest{offset:0,length:self.layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE}).unwrap();
  FrameBacking::from_resolved(self.layout,info,view).unwrap()
 }
 fn words(&self,index:usize)->&[u32]{unsafe{core::slice::from_raw_parts(self.pointer.add(index*self.layout.span_bytes()as usize)as*const u32,self.layout.span_bytes()as usize/4)}}
}
impl Drop for Region{fn drop(&mut self){unsafe{std::alloc::dealloc(self.pointer,self.allocation);}}}
fn fill(store:&mut Store,value:u32){let mut p=store.prepare(true).unwrap();{let t=p.target();assert_eq!((t.width(),t.height(),t.len()),(19,7,133));for i in 0..t.len(){unsafe{t.pointer().add(i).write(value);}}}assert!(p.commit(|f|f.previous.is_none()&&f.rows==frame_output::Rows{first:0,end:7}&&f.pixels.iter().all(|&p|p==value)));}
#[test]fn first_partial_refuses_without_mutation_and_full_initializes_then_swaps(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};
 assert_eq!(s.layout(),region.layout);assert_eq!(s.generation(),0);assert!(s.front_pixels().is_none());assert!(s.prepare(false).is_none());assert!(region.words(0).iter().chain(region.words(1)).all(|&v|v==0xa5a5a5a5));
 {let p=s.prepare(true).unwrap();assert!(p.commit(|f|f.pixels.len()==133&&f.pixels.iter().all(|&p|p==0)));}
 let first=s.front_pixels().unwrap().as_ptr();assert_eq!(s.generation(),1);
 fill(&mut s,0x12345678);assert_ne!(first,s.front_pixels().unwrap().as_ptr());assert_eq!(s.generation(),2);assert!(region.words(0).iter().all(|&v|v==0));
}
#[test]fn partial_starts_equal_to_front_and_reports_only_changed_rows(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};fill(&mut s,0x11223344);let front=s.front_pixels().unwrap().as_ptr()as usize;
 let mut p=s.prepare(false).unwrap();{let t=p.target();let candidate=t.pointer()as usize;assert_eq!(candidate.abs_diff(front),region.layout.span_bytes()as usize);
  assert!((0..t.len()).all(|i|unsafe{t.pointer().add(i).read()}==0x11223344),"candidate begins as the front");
  unsafe{t.pointer().add(2).write(7);t.pointer().add(t.len()-1).write(9);}}
 assert!(p.commit(|f|f.previous.is_some_and(|old|old.iter().all(|&v|v==0x11223344))&&f.rows==frame_output::Rows{first:0,end:7}
  &&f.pixels.iter().enumerate().all(|(i,&v)|v==if i==2{7}else if i==132{9}else{0x11223344})));
 assert_eq!(s.generation(),2);assert_eq!(s.front_pixels().unwrap()[132],9);assert!(region.words(0)[..133].iter().all(|&v|v==0x11223344));assert!(region.words(1)[133..].iter().all(|&v|v==0));
 // Next partial: the lagging span catches up on exactly the changed rows; one changed row is reported.
 let mut p=s.prepare(false).unwrap();{let t=p.target();assert_eq!(unsafe{t.pointer().add(2).read()},7);assert_eq!(unsafe{t.pointer().add(132).read()},9);unsafe{t.pointer().add(19*3+4).write(5);}}
 assert!(p.commit(|f|f.rows==frame_output::Rows{first:3,end:4}));
 let p=s.prepare(false).unwrap();assert!(p.commit(|f|f.rows.is_empty()&&f.previous.is_some()));
 assert_eq!(s.front_pixels().unwrap()[19*3+4],5);assert_eq!(s.front_pixels().unwrap()[2],7);
}
#[test]fn failed_partial_forces_a_full_catch_up(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};fill(&mut s,1);fill(&mut s,2);
 {let mut p=s.prepare(false).unwrap();let t=p.target();unsafe{t.pointer().write(3);}}
 let mut p=s.prepare(false).unwrap();{let t=p.target();assert!((0..t.len()).all(|i|unsafe{t.pointer().add(i).read()}==2),"erased candidate recopied in full");}
 assert!(p.commit(|f|f.rows.is_empty()));
}
#[test]fn failed_external_prefix_publish_keeps_old_front_but_external_output_is_uncertain(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};fill(&mut s,11);let front=s.front_pixels().unwrap().as_ptr();let mut external=vec![11;133];
 let mut p=s.prepare(true).unwrap();{let t=p.target();for i in 0..t.len(){unsafe{t.pointer().add(i).write(22);}}}
 assert!(!p.commit(|f|{external[..5].copy_from_slice(&f.pixels[..5]);false}));
 assert_eq!(&external[..6],&[22,22,22,22,22,11]);assert_eq!(s.generation(),1);assert_eq!(s.front_pixels().unwrap().as_ptr(),front);assert!(s.front_pixels().unwrap().iter().all(|&v|v==11));assert!(region.words(1).iter().all(|&v|v==0));
}
#[test]fn abort_and_callback_unwind_scrub_candidate_including_padding(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};fill(&mut s,77);
 {let mut p=s.prepare(false).unwrap();let t=p.target();unsafe{t.pointer().write(88);}}
 assert!(region.words(1).iter().all(|&v|v==0));assert_eq!(s.generation(),1);
 let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{let p=s.prepare(false).unwrap();p.commit(|_|panic!("callback failure"));}));assert!(result.is_err());assert!(region.words(1).iter().all(|&v|v==0));assert!(s.front_pixels().unwrap().iter().all(|&v|v==77));
}
#[test]fn cleanup_erases_both_full_spans_without_reusing_generation(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};fill(&mut s,3);fill(&mut s,4);assert_eq!(s.generation(),2);s.scrub();assert!(s.front_pixels().is_none());assert_eq!(s.generation(),2);assert!(region.words(0).iter().chain(region.words(1)).all(|&v|v==0));assert!(s.prepare(false).is_none());fill(&mut s,5);assert_eq!(s.generation(),3);
}
#[test]fn maximum_generation_refuses_before_any_candidate_mutation(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};fill(&mut s,123);s.set_generation_for_test(u64::MAX-1);fill(&mut s,456);let bytes:Vec<_>=region.words(0).iter().chain(region.words(1)).copied().collect();assert_eq!(s.generation(),u64::MAX);assert!(s.prepare(false).is_none());assert!(s.prepare(true).is_none());assert_eq!(region.words(0).iter().chain(region.words(1)).copied().collect::<Vec<_>>(),bytes);s.scrub();assert_eq!(s.generation(),u64::MAX);assert!(s.prepare(true).is_none());assert!(s.front_pixels().is_none());
}
#[test]fn first_failed_publish_never_makes_front_valid(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};assert!(!s.prepare(true).unwrap().commit(|_|false));assert!(s.front_pixels().is_none());assert_eq!(s.generation(),0);assert!(region.words(0).iter().all(|&v|v==0));
}

#[test]fn cleanup_erases_both_never_prepared_spans_and_padding(){
 let region=Region::new();let mut s=unsafe{Store::new(region.held())};s.scrub();assert_eq!(s.generation(),0);assert!(s.front_pixels().is_none());assert!(region.words(0).iter().chain(region.words(1)).all(|&v|v==0));
}
