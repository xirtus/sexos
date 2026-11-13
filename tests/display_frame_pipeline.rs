// Actual SDK/Store/Main functions, with explicitly controlled raster and IPC
// observation boundaries. HIGH_HALF_BASE=0 is a host scanout double, not a
// relaxation of the actual guest constant or proof of framebuffer authority.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
fn bar_height()->usize{28}
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
// This compositor fixture grants no capture authority. Compile/run the real
// capture observer inactive; active custody has its dedicated production gate.
#[path="../crates/sex-pdx/src/screen_capture.rs"]pub mod screen_capture;
#[path="../servers/sexdisplay/src/screen_capture.rs"]mod capture_server;
static mut SCREEN_CAPTURE:capture_server::Server=capture_server::Server::new();
#[derive(Clone,Copy)]pub struct PdxMessage{pub type_id:u64,pub arg0:u64,pub arg1:u64,pub arg2:u64,pub caller_pd:u32,pub caller_task:u32}
pub fn screen_capture_bank(_:bool)->Result<screen_capture::Bank,u64>{Err(ERR_CAP_INVALID)}
pub fn get_ticks()->u64{0}
pub fn pdx_reply(_:u32,_:u64)->u64{panic!("inactive capture cannot reply")}
pub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;
#[path="../crates/sex-pdx/src/display_frames.rs"]pub mod display_frames;
#[path="../servers/sexdisplay/src/frame_output.rs"]mod frame_output;
#[path="../servers/sexdisplay/src/damage.rs"]mod damage;
use silkbar_model::{SilkBar,DEFAULT_SILK_BAR,DEFAULT_THEME};
use core::sync::atomic::{AtomicU64,Ordering};
use display_frames::Layout;
const HIGH_HALF_BASE:u64=0;

const W:usize=19;const H:usize=53;
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut FRAME_OUTPUT:Option<frame_output::Store>=None;
static mut FB_PTR:u64=0;static mut FB_W:u32=0;static mut FB_H:u32=0;static mut FB_PITCH:u32=W as u32;static mut PUBLISHED_PIXELS:u64=0;static mut LAST_WRITTEN:u64=0;
static mut APP_PRESENT_BITS:u64=0;
static RENDER_FRAME_COUNTER:AtomicU64=AtomicU64::new(0);
static mut EVENTS:Vec<String>=Vec::new();
static mut BEFORE:Vec<u32>=Vec::new();static mut AFTER:Vec<u32>=Vec::new();
static mut MODE:u8=0; // 0 success;1 body rejects;2 scanout changes;3 strip rejects;4 nested attempt.
static mut COPY_ATTEMPTS:usize=0;static mut PUBLICATIONS:usize=0;static mut TARGET_ADDR:usize=0;
static mut CAPTURES:usize=0;static mut CURSORS:usize=0;static mut EXPECTED_PLAN:Option<Plan>=None;static mut SCENE_ADDR:usize=0;
static mut CLOCK_CANON_SS:u8=7;
const ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED:bool=false;const ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED:bool=false;
static mut RASTERS:usize=0;
static mut GRANT:Option<memory_region::RegionGrant>=None;
static mut QUERY_MODE:u8=0;
static mut QUERIES:Vec<(u64,u64,u64,u8)>=Vec::new();
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/desktop_look.rs"]pub mod desktop_look;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../servers/sexdisplay/src/surface_lifecycle.rs"]mod surface_lifecycle;
#[path="../servers/sexdisplay/src/surface_groups.rs"]mod surface_groups;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"]mod bitmap_binding;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]mod bitmap_transaction;
#[path="../servers/sexdisplay/src/bitmap_stores.rs"]mod bitmap_stores;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../servers/sexdisplay/src/text_transaction.rs"]mod text_transaction;
#[path="../servers/sexdisplay/src/scene.rs"]mod scene;
#[path="../servers/sexdisplay/src/paint_batch.rs"]mod paint_batch;
use paint_batch::{Content,Plan,Dirty};
use surface_lifecycle::{Surface,SURFACE_EMPTY,MAX_SURFACES};
// Actual Main pure background conversion, observed without changing its result.
// Actual SceneSnapshot calls this once for each capture attempt after bounds.
fn scene_bar_blur_rows(h:usize)->[u32;BAR_BG_H]{unsafe{CAPTURES+=1;}actual_scene_bar_blur_rows(h)}
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{let _=format_args!($($arg)*);$crate::event("receipt");}};}
include!(env!("DISPLAY_FRAME_SCENE_PARENT"));
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
// Observer calls the extracted actual CPU-copy function unchanged; counting
// callback entries/success does not substitute for checking complete device bytes.
fn publish_frame(fb:*mut u32,w:usize,h:usize,frame:frame_output::Publication<'_>)->bool{unsafe{COPY_ATTEMPTS+=1;}unsafe{if MODE==5{FB_W+=1;}}let ok=actual_publish_frame(fb,w,h,frame);if ok{unsafe{PUBLICATIONS+=1;}event("cpu-copy");}ok}
fn completed_effect(s:&str){unsafe{
 assert_eq!((COPY_ATTEMPTS,PUBLICATIONS),(1,1));assert_eq!(CURSORS,1,"effects before final raster overlay");assert_eq!(CAPTURES,1,"exactly one actual Scene capture");
 assert!(RENDER_SCENE.is_valid(),"lease must remain live through publication effects");
 assert_eq!(device(),AFTER,"effect before complete CPU copy");
}event(s);}
fn record_cursor_presented(){completed_effect("cursor-effect");}
fn report_app_presented(){completed_effect("app-effect");}
fn civil_clock_drawn(bar:&SilkBar,w:usize,h:usize,label:&str){assert_eq!((w,h),(W,H));unsafe{assert_eq!(bar.clock_ss,RENDER_SCENE.bar.clock_ss);}completed_effect(&format!("civil:{label}"));}
fn repaint_end(label:&str,started:u64){assert_eq!(started,41);completed_effect(&format!("timing:{label}"));}
fn clock_canon_apply_to_bar(bar:&mut SilkBar){unsafe{bar.clock_ss=CLOCK_CANON_SS;}}
fn observe_scene(scene:&scene::SceneSnapshot){unsafe{
 assert!(scene.is_valid());assert_eq!((scene.width,scene.height),(W,H));assert_eq!(scene as *const _ as usize,core::ptr::addr_of!(RENDER_SCENE)as usize);
 if SCENE_ADDR==0{SCENE_ADDR=scene as *const _ as usize;}else{assert_eq!(SCENE_ADDR,scene as *const _ as usize);}
 assert_eq!(scene.surfaces[0].color,0x1234,"second pass must use immutable captured source");
 assert_eq!(scene.frame,if EXPECTED_PLAN.unwrap().is_full(){43}else{42});
}}
fn mutate_bookkeeping(){unsafe{
 CURSOR_UNDER_SAVED.fill(0x77);CURSOR_UNDER_X=99;CURSOR_UNDER_Y=98;CURSOR_UNDER_VALID=false;APP_PRESENT_BITS=0x55;
 PREPARED_CURSOR=Some(PreparedCursor{x:8,y:9,seq:10,shell_tick:11});
}}
// Real Main raster_batch orders these finite primitive doubles. They receive
// actual Target and actual SceneSnapshot, never an unsafe synthetic Target.
fn primitive(target:&frame_output::Target<'_>,scene:&scene::SceneSnapshot,kind:&str)->bool{unsafe{
 RASTERS+=1;observe_scene(scene);assert_eq!((target.width(),target.height(),target.len()),(W,H,W*H));
 if TARGET_ADDR==0{TARGET_ADDR=target.pointer()as usize;}else{assert_eq!(TARGET_ADDR,target.pointer()as usize);}
 assert!((target.pointer()as u64)<memory_region::USER_VA_END);assert_ne!(target.pointer()as u64,FB_PTR);
 assert_eq!(device(),BEFORE,"device modified before batch completion");event(kind);
 match kind{
 "full"=>for i in 0..target.len(){target.pointer().add(i).write(0x22);},
 "area"=>for i in 28*W..target.len(){target.pointer().add(i).write(0x33);},
 "strip"=>for i in 0..28*W{target.pointer().add(i).write(0x66);},
 "damage"=>{let r=rect();for y in r.y..r.bottom{for x in r.x..r.right{target.pointer().add(y*W+x).write(0x44);}}},_=>panic!()}
 mutate_bookkeeping();SURFACES[0].color=0xabcd; // retained Scene must not follow live state.
 if MODE==2{FB_W+=1;}
 if MODE==4{assert!(render_prepared(FB_PTR as *mut u32,W,H,&DEFAULT_SILK_BAR,Plan::full()).is_none());assert_eq!(CAPTURES,1);}
 !(MODE==1||MODE==3&&kind=="strip")
}}
fn raster_full(t:&frame_output::Target<'_>,s:&scene::SceneSnapshot)->bool{primitive(t,s,"full")}
fn raster_strip(t:&frame_output::Target<'_>,s:&scene::SceneSnapshot)->bool{primitive(t,s,"strip")}
fn raster_area(t:&frame_output::Target<'_>,s:&scene::SceneSnapshot)->bool{primitive(t,s,"area")}
fn raster_damage(t:&frame_output::Target<'_>,r:damage::Rect,s:&scene::SceneSnapshot)->bool{assert_eq!(r,rect());primitive(t,s,"damage")}
fn restore_cursor_under(_fb:*mut u32,_w:usize,_h:usize,_len:usize,_:desktop_style::chrome::Metrics){event("restore");}
fn draw_atlas_cards_pass(_fb:*mut u32,_w:usize,_h:usize,_len:usize,s:&scene::SceneSnapshot){observe_scene(s);event("atlas");}
fn draw_atlas_frame_previews_pass(_fb:*mut u32,_w:usize,_h:usize,_len:usize,s:&scene::SceneSnapshot){observe_scene(s);event("preview");}
fn draw_launcher_panel(fb:*mut u32,_w:usize,_h:usize,len:usize,s:&scene::SceneSnapshot){observe_scene(s);unsafe{fb.add(len-2).write(0xdd);}event("launcher");}
fn draw_window_switcher(_:*mut u32,_:usize,_:usize,_:usize,_:&scene::SceneSnapshot){}
fn draw_cursor_z_top(fb:*mut u32,_w:usize,_h:usize,len:usize,save:bool,s:&scene::SceneSnapshot){observe_scene(s);assert!(save);unsafe{
 CURSORS+=1;assert_eq!(device(),BEFORE);fb.add(len-1).write(0xee);mutate_bookkeeping();}event("cursor-raster");}
fn rect()->damage::Rect{damage::Rect{x:1,y:28,right:4,bottom:30}}
fn expected_pixels(mut base:Vec<u32>,p:Plan)->Vec<u32>{
 if p.settle_only(){return base;}
 match p.content{Content::Full=>base.fill(0x22),Content::Area=>base[28*W..].fill(0x33),Content::Damage(r)=>for y in r.y..r.bottom{for x in r.x..r.right{base[y*W+x]=0x44;}},_=>{}}
 if p.strip{base[..28*W].fill(0x66);}if !p.cursor_only(){base[W*H-2]=0xdd;}base[W*H-1]=0xee;base
}
struct Harness{allocation:std::alloc::Layout,private:*mut u8,scanout:Vec<u32>,layout:Layout}
impl Harness{
 fn new()->Self{unsafe{
  assert!(FRAME_OUTPUT.is_none());assert!(!RENDER_SCENE.is_valid());
  let layout=Layout::new(W as u64,H as u64,(W*4)as u64).unwrap();let allocation=std::alloc::Layout::from_size_align(layout.total_bytes()as usize,4096).unwrap();
  let private=std::alloc::alloc(allocation);assert!(!private.is_null());core::ptr::write_bytes(private,0xa5,allocation.size());
  let mut scanout=vec![0x11;W*H];FB_PTR=scanout.as_mut_ptr()as u64;FB_W=W as u32;FB_H=H as u32;
  GRANT=Some(memory_region::RegionGrant{region_id:11,virtual_base:private as u64,length:layout.total_bytes(),rights:7,dma_base:None,peer_pd:None});QUERY_MODE=0;QUERIES.clear();
  // Only this exclusive Store owns the host allocation; descriptor queries do
  // not manufacture aliasing permission. Every reference dies before Harness.
  FRAME_OUTPUT=Some(frame_output::Store::new(display_frame_backing(layout).unwrap()));
  Self{allocation,private,scanout,layout}
 }}
 fn setup(&self,p:Plan,mode:u8){unsafe{
  MODE=mode;RASTERS=0;COPY_ATTEMPTS=0;PUBLICATIONS=0;TARGET_ADDR=0;CAPTURES=0;CURSORS=0;SCENE_ADDR=0;EVENTS.clear();BEFORE=self.scanout.clone();
  let effective=if FRAME_OUTPUT.as_ref().unwrap().front_pixels().is_none(){Plan::full()}else{p};
  EXPECTED_PLAN=Some(effective);AFTER=expected_pixels(BEFORE.clone(),effective);FB_W=W as u32;FB_H=H as u32;
  SURFACES.fill(SURFACE_EMPTY);SURFACES[0].color=0x1234;TEXT_STAGES.fill(text_transaction::Stage::EMPTY);
  CURSOR_UNDER_SAVED.fill(0x44);CURSOR_UNDER_X=2;CURSOR_UNDER_Y=3;CURSOR_UNDER_VALID=true;
  APP_PRESENT_BITS=7;PREPARED_CURSOR=Some(PreparedCursor{x:4,y:5,seq:6,shell_tick:7});RENDER_FRAME_COUNTER.store(42,Ordering::Relaxed);
 }}
 fn call(&mut self,p:Plan)->Option<PublishedFrame>{render_prepared(self.scanout.as_mut_ptr(),W,H,&DEFAULT_SILK_BAR,p)}
 fn attempt(&mut self,d:&mut Dirty)->Option<PublishedFrame>{actual_attempt(d,true,W,H,&mut DEFAULT_SILK_BAR.clone())}
 fn prime(&mut self){self.setup(Plan::full(),0);assert!(self.call(Plan::full()).is_some());}
 fn generation(&self)->u64{unsafe{FRAME_OUTPUT.as_ref().unwrap().generation()}}
 fn front(&self)->Option<Vec<u32>>{unsafe{FRAME_OUTPUT.as_ref().unwrap().front_pixels().map(<[u32]>::to_vec)}}
 fn span(&self,i:usize)->Vec<u32>{unsafe{core::slice::from_raw_parts(self.private.add(i*self.layout.span_bytes()as usize).cast(),self.layout.span_bytes()as usize/4).to_vec()}}
}
impl Drop for Harness{fn drop(&mut self){unsafe{assert!(!RENDER_SCENE.is_valid());if let Some(mut s)=FRAME_OUTPUT.take(){s.scrub();}GRANT=None;FB_PTR=0;std::alloc::dealloc(self.private,self.allocation);}}}
fn rollback_restored(){unsafe{
 assert!(CURSOR_UNDER_SAVED.iter().all(|&v|v==0x44));assert_eq!((CURSOR_UNDER_X,CURSOR_UNDER_Y,CURSOR_UNDER_VALID,APP_PRESENT_BITS),(2,3,true,7));
 let p=PREPARED_CURSOR.unwrap();assert_eq!((p.x,p.y,p.seq,p.shell_tick),(4,5,6,7));assert_eq!(RENDER_FRAME_COUNTER.load(Ordering::Relaxed),42);
}}
fn partial_plans()->[Plan;7]{[Plan{content:Content::Area,strip:false},Plan{content:Content::Damage(rect()),strip:false},Plan{content:Content::Cursor,strip:false},Plan{content:Content::None,strip:true},Plan{content:Content::Area,strip:true},Plan{content:Content::Damage(rect()),strip:true},Plan{content:Content::Cursor,strip:true}]}
fn no_effects(){unsafe{assert_eq!(PUBLICATIONS,0);assert!(!EVENTS.iter().any(|v|v.ends_with("-effect")||v.starts_with("civil:")||v.starts_with("timing:")||v=="receipt"));assert!(!RENDER_SCENE.is_valid());}}
#[test]fn actual_combined_batches_publish_once_with_one_immutable_scene(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();h.prime();
 let mut plans=partial_plans().to_vec();let mut dirty=Dirty::new(W,H);dirty.request_area();dirty.request_strip();dirty.request_damage(rect(),W,H,28);dirty.request_cursor();dirty.request_full();plans.push(dirty.plan(W,H));
 for p in plans{let gen=h.generation();h.setup(p,0);let receipt=h.call(p).unwrap();assert_eq!(receipt.plan,p);assert_eq!((receipt.present_bits,receipt.canon_ss),(0x55,7));assert_eq!(h.generation(),gen+1);
  unsafe{assert_eq!(h.scanout,AFTER);assert_eq!(h.front(),Some(AFTER.clone()));assert_eq!((CAPTURES,CURSORS,COPY_ATTEMPTS,PUBLICATIONS),(1,1,1,1));assert!(!RENDER_SCENE.is_valid());assert_eq!(RENDER_FRAME_COUNTER.load(Ordering::Relaxed),if p.is_full(){43}else{42});
   for label in["cursor-effect","app-effect","receipt"]{assert_eq!(EVENTS.iter().filter(|v|*v==label).count(),1);}
   assert_eq!(EVENTS.iter().filter(|v|v.starts_with("civil:")).count(),p.clock_dirty()as usize);assert_eq!(EVENTS.iter().filter(|v|v.starts_with("timing:")).count(),(!p.cursor_only())as usize);
   if p.strip{assert_eq!(EVENTS.iter().filter(|v|*v=="strip").count(),1);}if matches!(p.content,Content::Area|Content::Damage(_)){assert_eq!(SURFACES[0].color,0xabcd);}
  }
 }
}
#[test]fn every_body_failure_restores_bookkeeping_and_retains_front(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();h.prime();let old=h.front();let device=h.scanout.clone();
 for p in [Plan::full(),Plan{content:Content::Area,strip:true},Plan{content:Content::Damage(rect()),strip:true},Plan{content:Content::None,strip:true}]{h.setup(p,1);assert!(h.call(p).is_none());rollback_restored();no_effects();assert_eq!(h.front(),old);assert_eq!(h.scanout,device);assert_eq!(h.generation(),1);assert!(h.span(1).iter().all(|&v|v==0));}
}
#[test]fn failed_second_pass_preserves_dirty_and_retry_merges_exact_work(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();h.prime();let mut d=Dirty::new(W,H);d.request_damage(rect(),W,H,28);d.request_strip();let plan=d.plan(W,H);let saved=d;
 h.setup(plan,3);assert!(h.attempt(&mut d).is_none());assert_eq!(d,saved);rollback_restored();no_effects();assert_eq!(h.generation(),1);unsafe{assert!(EVENTS.iter().any(|e|e=="damage"));assert!(EVENTS.iter().any(|e|e=="strip"));assert_eq!(h.scanout,BEFORE);}
 d.request_area();d.request_cursor();let merged=d.plan(W,H);assert_eq!(merged,Plan{content:Content::Area,strip:true});h.setup(merged,0);assert!(h.attempt(&mut d).is_some());assert!(d.plan(W,H).is_empty());assert_eq!(h.generation(),2);
}
#[test]fn post_prepare_scanout_change_withholds_copy_and_all_effects(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();h.prime();let old=h.front();let device=h.scanout.clone();
 for p in[Plan::full(),Plan{content:Content::Area,strip:true}]{let mut d=Dirty::new(W,H);if p.is_full(){d.request_full();}else{d.request_area();d.request_strip();}let before=d;h.setup(p,2);assert!(h.attempt(&mut d).is_none());assert_eq!(d,before);rollback_restored();no_effects();assert_eq!(h.front(),old);assert_eq!(h.scanout,device);assert_eq!(h.generation(),1);assert!(h.span(1).iter().all(|&v|v==0));}
}
#[test]fn scanout_identity_layout_and_no_work_reject_before_capture(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();h.setup(Plan::full(),0);let fb=h.scanout.as_mut_ptr();
 for(ptr,w,hgt,p)in[(unsafe{fb.add(1)},W,H,Plan::full()),(fb,W+1,H,Plan::full()),(fb,W,0,Plan::full()),(fb,usize::MAX,H,Plan::full()),(fb,W,H,Plan{content:Content::None,strip:false})]{assert!(render_prepared(ptr,w,hgt,&DEFAULT_SILK_BAR,p).is_none());}
 unsafe{FB_W=(W+1)as u32;}assert!(render_prepared(fb,W+1,H,&DEFAULT_SILK_BAR,Plan::full()).is_none());unsafe{assert_eq!((RASTERS,CAPTURES,CURSORS),(0,0,0));}assert_eq!(h.generation(),0);assert!(h.front().is_none());no_effects();
 let mut pending=Dirty::new(W,H);pending.request_area();pending.request_strip();let retained=pending;
 assert!(actual_attempt(&mut pending,false,W,H,&mut DEFAULT_SILK_BAR.clone()).is_none());assert_eq!(pending,retained);unsafe{assert_eq!(CAPTURES,0);}
}
#[test]fn first_partial_and_cleaned_store_promote_to_full_without_stale_pixels(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();
 for p in partial_plans(){h.setup(p,0);assert_eq!(h.call(p).unwrap().plan,Plan::full());unsafe{FRAME_OUTPUT.as_mut().unwrap().scrub();}assert!(h.front().is_none());for i in 0..2{assert!(h.span(i).iter().all(|&v|v==0));}}
 assert_eq!(h.generation(),7);
}
#[test]fn geometry_promotion_survives_failure_and_nested_attempt_is_inert(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();h.prime();let mut d=Dirty::new(W+1,H);d.request_cursor();let p=d.plan(W,H);assert!(p.is_full());h.setup(p,1);assert!(h.attempt(&mut d).is_none());assert!(d.plan(W,H).is_full());
 h.setup(p,4);assert!(h.attempt(&mut d).is_some());assert!(d.plan(W,H).is_empty());unsafe{assert_eq!(CAPTURES,1);}assert_eq!(h.generation(),2);
}
#[test]fn partial_baseline_preserves_every_unrequested_pixel(){let _l=LOCK.lock().unwrap();let mut h=Harness::new();h.prime();let p=Plan{content:Content::Damage(rect()),strip:false};h.setup(p,0);let old=h.scanout.clone();assert!(h.call(p).is_some());
 for i in 0..W*H{let x=i%W;let y=i/W;let r=rect();if !(x>=r.x&&x<r.right&&y>=r.y&&y<r.bottom)&&i<W*H-2{assert_eq!(h.scanout[i],old[i]);}}
}
#[test]fn acquisition_uses_exact_slot_and_extent_and_rejects_foreign_descriptors(){let _l=LOCK.lock().unwrap();let h=Harness::new();unsafe{
 assert_eq!(QUERIES,[(98,u64::MAX,0,0),(98,0,h.layout.total_bytes(),7)]);for mode in 1..=8{QUERY_MODE=mode;QUERIES.clear();assert!(matches!(display_frame_backing(h.layout),Err(ERR_CAP_INVALID)));assert_eq!(QUERIES,if mode==1{vec![(98,u64::MAX,0,0)]}else{vec![(98,u64::MAX,0,0),(98,0,h.layout.total_bytes(),7)]});}assert_eq!(h.generation(),0);assert!(h.front().is_none());
}}

#[test]fn failed_output_keeps_canonical_text_and_success_carries_owned_candidate_context(){
 let _single=LOCK.lock().unwrap();let mut h=Harness::new();
 for mode in[1,2,0]{
  h.setup(Plan::full(),mode);
  let context=unsafe{
   SURFACES[0].active=true;SURFACES[0].owner_pd=9;SURFACES[0].surface_id=156;
   SURFACES[0].w=200;SURFACES[0].h=120;SURFACES[0].text_len=3;SURFACES[0].text_buf[..3].copy_from_slice(b"old");
   let stage=&mut TEXT_STAGES[0];let bytes="éA\nß".as_bytes();
   stage.receive(9,90,156,text_batch::Request::Begin{sequence:7,len:bytes.len(),color:0xff123456});
   let mut word=[0;8];word[..bytes.len()].copy_from_slice(bytes);
   stage.receive(9,90,156,text_batch::Request::Chunk{sequence:7,offset:0,count:bytes.len(),bytes:word});
   assert!(stage.receive(9,90,156,text_batch::Request::Commit{sequence:7,hash:text_batch::hash(bytes)}).is_some());
   stage.pending().unwrap().context()
  };
  let receipt=h.call(Plan::full());
  unsafe{
   assert_eq!(&SURFACES[0].text_buf[..3],b"old");assert_eq!(SURFACES[0].text_len,3);
   assert!(TEXT_STAGES[0].needs_publication());TEXT_STAGES[0].poll_reply(|_,_|panic!("render must not commit or release text"));
   assert!(!RENDER_SCENE.is_valid());assert!(RENDER_SCENE.text_projections.iter().all(Option::is_none));
  }
  if mode==0{
   let receipt=receipt.unwrap();assert_eq!(receipt.text_projections[0].unwrap().context,context);assert!(receipt.text_projections[1..].iter().all(Option::is_none));
  }else{assert!(receipt.is_none());assert_eq!(h.generation(),0);}
 }
}

#[test]fn pure_settle_preserves_device_underlay_and_effect_state_then_carries_no_pixels_projection(){
 let _single=LOCK.lock().unwrap();let mut h=Harness::new();h.prime();let old=h.front();let device=h.scanout.clone();
 for mode in[5,0]{
  let plan=Plan{content:Content::Settle,strip:false};h.setup(plan,mode);
  unsafe{
   SURFACES[0]=Surface{active:true,owner_pd:9,surface_id:156,x:1000,y:1000,w:200,h:120,color:0x1234,..SURFACE_EMPTY};
   TEXT_STAGES[0].receive(9,90,156,text_batch::Request::Begin{sequence:9,len:1,color:0xff123456});
   TEXT_STAGES[0].receive(9,90,156,text_batch::Request::Chunk{sequence:9,offset:0,count:1,bytes:[b'x',0,0,0,0,0,0,0]});
   assert!(TEXT_STAGES[0].receive(9,90,156,text_batch::Request::Commit{sequence:9,hash:text_batch::hash(b"x")}).is_some());
  }
  let receipt=h.call(plan);
  unsafe{
   assert_eq!((RASTERS,CURSORS,COPY_ATTEMPTS),(0,0,1));
   assert!(CURSOR_UNDER_SAVED.iter().all(|v|*v==0x44));assert_eq!((CURSOR_UNDER_X,CURSOR_UNDER_Y,CURSOR_UNDER_VALID),(2,3,true));
   assert_eq!(APP_PRESENT_BITS,7);let cursor=PREPARED_CURSOR.unwrap();assert_eq!((cursor.x,cursor.y,cursor.seq,cursor.shell_tick),(4,5,6,7));
   assert_eq!(RENDER_FRAME_COUNTER.load(Ordering::Relaxed),42);assert_eq!(SURFACES[0].text_len,0);assert!(TEXT_STAGES[0].needs_publication());
   assert!(!EVENTS.iter().any(|v|v=="restore"||v=="launcher"||v=="cursor-raster"||v=="begin"||v.ends_with("-effect")||v.starts_with("civil:")||v.starts_with("timing:")));
   assert!(!RENDER_SCENE.is_valid());
  }
  assert_eq!(h.front(),old);assert_eq!(h.scanout,device);
  if mode==5{assert!(receipt.is_none());assert_eq!(h.generation(),1);unsafe{assert_eq!(PUBLICATIONS,0);}}
  else{let receipt=receipt.unwrap();assert!(receipt.plan.settle_only());assert_eq!(receipt.present_bits,0);assert_eq!(receipt.text_projections[0].unwrap().coverage,scene::TextCoverage::NoPixels);assert_eq!(h.generation(),2);}
 }
}
#[test]fn no_front_settle_uses_full_bootstrap_instead_of_an_uninitialized_identity(){
 let _single=LOCK.lock().unwrap();let mut h=Harness::new();let plan=Plan{content:Content::Settle,strip:false};h.setup(plan,0);let receipt=h.call(plan).unwrap();assert!(receipt.plan.is_full());assert_eq!(h.generation(),1);unsafe{assert_eq!((RASTERS,CURSORS,PUBLICATIONS),(1,1,1));}
}
#[test]fn actual_text_join_derives_visible_damage_no_pixels_and_tiny_full_fallback(){
 let _single=LOCK.lock().unwrap();let h=Harness::new();h.setup(Plan::full(),0);
 unsafe{
  SURFACES[0]=Surface{active:true,owner_pd:9,surface_id:156,x:0,y:40,w:19,h:53,color:0x1234,..SURFACE_EMPTY};
  TEXT_STAGES[0].receive(9,90,156,text_batch::Request::Begin{sequence:1,len:1,color:0xff123456});
  TEXT_STAGES[0].receive(9,90,156,text_batch::Request::Chunk{sequence:1,offset:0,count:1,bytes:[b'x',0,0,0,0,0,0,0]});
  TEXT_STAGES[0].receive(9,90,156,text_batch::Request::Commit{sequence:1,hash:text_batch::hash(b"x")}).unwrap();
 }
 let mut visible=Dirty::new(W,H);join_pending_text(&mut visible,W,H);assert_eq!(visible.plan(W,H),Plan{content:Content::Damage(damage::Rect{x:0,y:52,right:19,bottom:53}),strip:false});
 unsafe{SURFACES[0].x=1000;SURFACES[0].y=1000;}
 let mut hidden=Dirty::new(W,H);join_pending_text(&mut hidden,W,H);assert!(hidden.plan(W,H).settle_only());
 let mut tiny=Dirty::new(W,27);join_pending_text(&mut tiny,W,27);assert!(tiny.plan(W,27).is_full());
 unsafe{assert_eq!(SURFACES[0].text_len,0);assert!(TEXT_STAGES[0].needs_publication());}
}
