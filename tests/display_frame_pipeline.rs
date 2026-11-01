// Actual SDK/Store/Main functions, with explicitly controlled raster and IPC
// observation boundaries. HIGH_HALF_BASE=0 is a host scanout double, not a
// relaxation of the actual guest constant or proof of framebuffer authority.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
pub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;
#[path="../crates/sex-pdx/src/display_frames.rs"]pub mod display_frames;
#[path="../servers/sexdisplay/src/frame_output.rs"]mod frame_output;
#[path="../servers/sexdisplay/src/damage.rs"]mod damage;
use silkbar_model::{SilkBar,DEFAULT_SILK_BAR};
use core::sync::atomic::{AtomicU64,Ordering};
use display_frames::Layout;
const HIGH_HALF_BASE:u64=0;
const MAX_FB_W:usize=8192;const MAX_FB_H:usize=4320;
const W:usize=19;const H:usize=53;
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut FRAME_OUTPUT:Option<frame_output::Store>=None;
static mut FB_PTR:u64=0;static mut FB_W:u32=0;static mut FB_H:u32=0;
static mut APP_PRESENT_BITS:u64=0;
static RENDER_FRAME_COUNTER:AtomicU64=AtomicU64::new(0);
static mut EVENTS:Vec<String>=Vec::new();
static mut BEFORE:Vec<u32>=Vec::new();static mut AFTER:Vec<u32>=Vec::new();
static mut MODE:u8=0; // 0 success, 1 raster rejection, 2 invalidate scanout after preflight.
static mut RASTERS:usize=0;
static mut GRANT:Option<memory_region::RegionGrant>=None;
static mut QUERY_MODE:u8=0;
static mut QUERIES:Vec<(u64,u64,u64,u8)>=Vec::new();
include!(env!("DISPLAY_FRAME_MAIN"));
include!(env!("DISPLAY_FRAME_ACQUIRE"));
fn memory_region_info(slot:u64)->Result<MemoryRegionInfo,u64>{unsafe{
 QUERIES.push((slot,u64::MAX,0,0));assert_eq!(slot,98);
 if QUERY_MODE==1{return Err(ERR_CAP_INVALID)}
 let mut info=memory_region::resolve_info(GRANT.as_ref()).map_err(|_|ERR_CAP_INVALID)?;
 if QUERY_MODE==3{info.region_id=12;} if QUERY_MODE==4{info.length-=4096;}
 if QUERY_MODE==5{info.peer_pd=Some(2);} if QUERY_MODE==6{info.rights|=memory_region::DMA_ADDRESS;}
 Ok(info)
}}
fn memory_region_view(slot:u64,offset:u64,length:u64,rights:u8)->Result<memory_region::RegionView,u64>{unsafe{
 QUERIES.push((slot,offset,length,rights));assert_eq!((slot,offset,rights),(98,0,7));assert_eq!(length,GRANT.unwrap().length);
 if QUERY_MODE==2{return Err(ERR_CAP_INVALID)}
 let mut view=memory_region::resolve_view(GRANT.as_ref(),memory_region::ViewRequest{offset,length,rights}).map_err(|_|ERR_CAP_INVALID)?;
 if QUERY_MODE==7{view.dma_address=Some(0x1000);}if QUERY_MODE==8{view.region_id=12;}
 Ok(view)
}}
fn event(s:&str){unsafe{EVENTS.push(s.to_owned());}}
fn device()->Vec<u32>{unsafe{core::slice::from_raw_parts(FB_PTR as *const u32,W*H).to_vec()}}
fn repaint_begin()->u64{event("begin");41}
fn completed_effect(s:&str){unsafe{
 assert!(EVENTS.iter().any(|e|e=="raster-done"),"effect before preparation completed");
 assert_eq!(device(),AFTER,"effect before complete CPU copy");
}event(s);}
fn record_cursor_presented(){completed_effect("cursor");}
fn report_app_presented(){completed_effect("app");}
fn civil_clock_drawn(_bar:&SilkBar,w:usize,h:usize,label:&str){assert_eq!((w,h),(W,H));completed_effect(&format!("civil:{label}"));}
fn repaint_end(label:&str,started:u64){assert_eq!(started,41);completed_effect(&format!("timing:{label}"));}
// These are renderer doubles, not replacement Target/Store implementations.
// They are handed the actual safely borrowed Target from actual Preparation.
fn raster(target:&frame_output::Target<'_>,full:bool)->bool{unsafe{
 RASTERS+=1;assert_eq!((target.width(),target.height(),target.len()),(W,H,W*H));
 assert!((target.pointer()as u64)<memory_region::USER_VA_END);assert_ne!(target.pointer()as u64,FB_PTR);
 assert_eq!(device(),BEFORE,"device modified before raster completion");event("raster-start");
 let start=if full{0}else{3};let end=if full{target.len()}else{6};
 for i in start..end{core::ptr::write(target.pointer().add(i),AFTER[i]);}
 CURSOR_UNDER_SAVED.fill(0x77);CURSOR_UNDER_X=99;CURSOR_UNDER_Y=98;CURSOR_UNDER_VALID=false;
 APP_PRESENT_BITS=0x55;RENDER_FRAME_COUNTER.store(99,Ordering::Relaxed);
 PREPARED_CURSOR=Some(PreparedCursor{x:8,y:9,seq:10,shell_tick:11});
 assert_eq!(device(),BEFORE,"device modified during raster");event("raster-done");
 if MODE==2{FB_W+=1;} MODE!=1
}}
fn raster_full(t:&frame_output::Target<'_>,_:&SilkBar)->bool{raster(t,true)}
fn raster_strip(t:&frame_output::Target<'_>,_:&SilkBar)->bool{raster(t,false)}
fn raster_area(t:&frame_output::Target<'_>,_:&SilkBar)->bool{raster(t,false)}
fn raster_damage(t:&frame_output::Target<'_>,r:damage::Rect,_:&SilkBar)->bool{assert_eq!(r,rect());raster(t,false)}
fn raster_cursor(t:&frame_output::Target<'_>,_:&SilkBar)->bool{raster(t,false)}
fn rect()->damage::Rect{damage::Rect{x:1,y:51,right:4,bottom:53}}
struct Harness{allocation:std::alloc::Layout,private:*mut u8,scanout:Vec<u32>,layout:Layout}
impl Harness{
 fn new()->Self{unsafe{
  assert!(FRAME_OUTPUT.is_none());let layout=Layout::new(W as u64,H as u64,(W*4)as u64).unwrap();
  let allocation=std::alloc::Layout::from_size_align(layout.total_bytes()as usize,4096).unwrap();
  let private=std::alloc::alloc(allocation);assert!(!private.is_null());core::ptr::write_bytes(private,0xa5,allocation.size());
  let mut scanout=vec![0x11;W*H];FB_PTR=scanout.as_mut_ptr()as u64;FB_W=W as u32;FB_H=H as u32;
  GRANT=Some(memory_region::RegionGrant{region_id:11,virtual_base:private as u64,length:layout.total_bytes(),rights:7,dma_base:None,peer_pd:None});
  QUERY_MODE=0;QUERIES.clear();
  let backing=display_frame_backing(layout).unwrap();
  // Exactly one Store; Harness outlives it and every borrowed slice. The mock
  // syscall metadata itself does not establish this exclusive Rust ownership.
  FRAME_OUTPUT=Some(frame_output::Store::new(backing));
  Self{allocation,private,scanout,layout}
 }}
 fn setup(&self,full:bool,mode:u8){unsafe{
  MODE=mode;RASTERS=0;EVENTS.clear();BEFORE=self.scanout.clone();AFTER=BEFORE.clone();
  if full{AFTER.fill(0x22);}else{AFTER[3..6].fill(0x33);}
  FB_W=W as u32;FB_H=H as u32;
  CURSOR_UNDER_SAVED.fill(0x44);CURSOR_UNDER_X=2;CURSOR_UNDER_Y=3;CURSOR_UNDER_VALID=true;
  APP_PRESENT_BITS=7;PREPARED_CURSOR=Some(PreparedCursor{x:4,y:5,seq:6,shell_tick:7});RENDER_FRAME_COUNTER.store(42,Ordering::Relaxed);
 }}
 fn call(&mut self,which:usize)->bool{let fb=self.scanout.as_mut_ptr();let bar=DEFAULT_SILK_BAR;match which{
  0=>render(fb,W,H,&bar),1=>redraw_top_strip(fb,W,H,&bar),2=>redraw_surface_area(fb,W,H,&bar),
  3=>redraw_surface_damage(fb,W,H,rect(),&bar),4=>redraw_cursor(fb,W,H,&bar),_=>panic!()
 }}
 fn generation(&self)->u64{unsafe{FRAME_OUTPUT.as_ref().unwrap().generation()}}
 fn front(&self)->Option<Vec<u32>>{unsafe{FRAME_OUTPUT.as_ref().unwrap().front_pixels().map(<[u32]>::to_vec)}}
 fn span(&self,i:usize)->Vec<u32>{unsafe{core::slice::from_raw_parts(self.private.add(i*self.layout.span_bytes()as usize).cast(),self.layout.span_bytes()as usize/4).to_vec()}}
}
impl Drop for Harness{fn drop(&mut self){unsafe{
 if let Some(mut store)=FRAME_OUTPUT.take(){store.scrub();drop(store);}
 GRANT=None;FB_PTR=0;std::alloc::dealloc(self.private,self.allocation);
}}}
fn rollback_restored(){unsafe{
 assert!(CURSOR_UNDER_SAVED.iter().all(|&v|v==0x44));assert_eq!((CURSOR_UNDER_X,CURSOR_UNDER_Y,CURSOR_UNDER_VALID,APP_PRESENT_BITS),(2,3,true,7));
 let p=PREPARED_CURSOR.unwrap();assert_eq!((p.x,p.y,p.seq,p.shell_tick),(4,5,6,7));assert_eq!(RENDER_FRAME_COUNTER.load(Ordering::Relaxed),42);
}}
#[test]fn actual_wrappers_copy_before_present_with_low_va_private_target(){let _lock=LOCK.lock().unwrap();let mut h=Harness::new();
 for which in 0..5{h.setup(which==0,0);assert!(h.call(which));assert_eq!(h.generation(),which as u64+1);
  unsafe{assert_eq!(h.scanout,AFTER);assert_eq!(h.front(),Some(AFTER.clone()));
   let label=["full","strip","area","text","cursor"][which];let mut want=Vec::new();if which!=4{want.push("begin".to_string());}want.extend(["raster-start".to_string(),"raster-done".into(),"cursor".into(),"app".into()]);if which!=4{want.push(format!("timing:{label}"));}
   if which<2{want.push(format!("civil:{label}"));}assert_eq!(EVENTS,want);
   if which!=0{assert_eq!(&h.scanout[..3],&BEFORE[..3]);assert_eq!(&h.scanout[6..],&BEFORE[6..]);}
  }
 }
}
#[test]fn every_wrapper_restores_bookkeeping_and_retains_front_after_raster_abort(){let _lock=LOCK.lock().unwrap();let mut h=Harness::new();h.setup(true,0);assert!(h.call(0));
 let front=h.front();let generation=h.generation();let old=h.scanout.clone();
 for which in 0..5{h.setup(which==0,1);assert!(!h.call(which));rollback_restored();assert_eq!(h.front(),front);assert_eq!(h.generation(),generation);assert_eq!(h.scanout,old);assert!(h.span(1).iter().all(|&v|v==0));
  unsafe{assert_eq!(EVENTS,if which==4{vec!["raster-start","raster-done"]}else{vec!["begin","raster-start","raster-done"]});}
 }
}
#[test]fn post_prepare_scanout_change_withholds_copy_and_all_effects(){let _lock=LOCK.lock().unwrap();let mut h=Harness::new();h.setup(true,0);assert!(h.call(0));
 let front=h.front();let old=h.scanout.clone();for which in 0..5{h.setup(which==0,2);assert!(!h.call(which));rollback_restored();assert_eq!(h.scanout,old);assert_eq!(h.front(),front);assert_eq!(h.generation(),1);assert!(h.span(1).iter().all(|&v|v==0));unsafe{assert_eq!(EVENTS,if which==4{vec!["raster-start","raster-done"]}else{vec!["begin","raster-start","raster-done"]});}}
}
#[test]fn scanout_identity_and_layout_reject_before_raster(){let _lock=LOCK.lock().unwrap();let mut h=Harness::new();h.setup(true,0);let fb=h.scanout.as_mut_ptr();
 assert!(!render(unsafe{fb.add(1)},W,H,&DEFAULT_SILK_BAR));assert!(!render(fb,W+1,H,&DEFAULT_SILK_BAR));
 assert!(!render(fb,W,0,&DEFAULT_SILK_BAR));assert!(!render(fb,usize::MAX,H,&DEFAULT_SILK_BAR));
 unsafe{FB_W=(W+1)as u32;}assert!(!render(fb,W+1,H,&DEFAULT_SILK_BAR));
 unsafe{assert_eq!(RASTERS,0);assert!(EVENTS.iter().all(|s|s=="begin"));}assert_eq!(h.generation(),0);assert!(h.front().is_none());assert!(h.scanout.iter().all(|&v|v==0x11));
}
#[test]fn first_partial_and_cleaned_store_have_no_publishable_front(){let _lock=LOCK.lock().unwrap();let mut h=Harness::new();
 for which in 1..5{h.setup(false,0);assert!(!h.call(which));unsafe{assert_eq!(RASTERS,0);assert_eq!(EVENTS,if which==4{vec![]}else{vec!["begin"]});}rollback_restored();}
 h.setup(true,0);assert!(h.call(0));unsafe{FRAME_OUTPUT.as_mut().unwrap().scrub();}let device=h.scanout.clone();
 assert!(h.front().is_none());assert_eq!(h.generation(),1);for i in 0..2{assert!(h.span(i).iter().all(|&v|v==0));}
 for which in 1..5{h.setup(false,0);assert!(!h.call(which));assert_eq!(h.scanout,device);assert_eq!(h.generation(),1);}
 h.setup(true,0);assert!(h.call(0));assert_eq!(h.generation(),2);
}
#[test]fn acquisition_uses_exact_slot_and_extent_and_rejects_foreign_descriptors(){let _lock=LOCK.lock().unwrap();let h=Harness::new();unsafe{
 assert_eq!(QUERIES,[(98,u64::MAX,0,0),(98,0,h.layout.total_bytes(),7)]);
 for mode in 1..=8{QUERY_MODE=mode;QUERIES.clear();assert!(matches!(display_frame_backing(h.layout),Err(ERR_CAP_INVALID)));
  if mode==1{assert_eq!(QUERIES,[(98,u64::MAX,0,0)]);}else{assert_eq!(QUERIES,[(98,u64::MAX,0,0),(98,0,h.layout.total_bytes(),7)]);}
 }assert_eq!(h.generation(),0);assert!(h.front().is_none());
}}
