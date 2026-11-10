// Host fixture uses actual SDK, Surface, FrameGroups, bitmap publication and Scene.
// Controlled held metadata is not a runtime grant or native composition proof.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_stack.rs"]pub mod desktop_stack;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
pub const ERR_CAP_INVALID:u64=u64::MAX-3;
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
#[path="../crates/sex-pdx/src/display_frames.rs"]pub mod display_frames;
#[path="../servers/sexdisplay/src/frame_output.rs"]mod frame_output;
#[path="../servers/sexdisplay/src/paint_batch.rs"]mod paint_batch;
#[path="../servers/sexdisplay/src/damage.rs"]mod damage;
use paint_batch::{Content,Plan};
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod app_lifecycle_sdk;
pub mod app_lifecycle{
 pub use crate::app_lifecycle_sdk::*;
 pub fn info(slot:usize)->Result<LifecycleInfo,u64>{crate::boundary::info(slot)}
 pub fn ack(slot:usize,task:u32)->Result<(),u64>{crate::boundary::ack(slot,task)}
}
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/desktop_look.rs"]pub mod desktop_look;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../servers/sexdisplay/src/surface_lifecycle.rs"]mod surface_lifecycle;
#[path="../servers/sexdisplay/src/surface_groups.rs"]mod surface_groups;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"]mod bitmap_binding;
#[path="../servers/sexdisplay/src/bitmap_transaction.rs"]mod bitmap_transaction;
#[path="../servers/sexdisplay/src/bitmap_stores.rs"]mod bitmap_stores;
include!(env!("DISPLAY_SCENE_PARENT"));
include!(env!("DISPLAY_SCENE_LEASE"));
#[path="../servers/sexdisplay/src/scene.rs"]mod scene;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../servers/sexdisplay/src/text_transaction.rs"]mod text_transaction;
use surface_lifecycle::{Surface,SURFACE_EMPTY,MAX_SURFACES,Catalog};
use surface_groups::FrameGroups;
use bitmap_binding::Binding;
use bitmap_batch::{Request,Geometry};
use bitmap_stores::Stores;
use scene::SceneSnapshot;
const N:usize=app_catalog::count(app_catalog::App::Browser);
type Set=Stores<N>;
fn bind(target:u32,task:u32)->Binding{
 let row=app_catalog::by_target(target).unwrap();
 Binding::from_catalog(row.lifecycle_slot,app_lifecycle::CatalogInfo{target_pd:target,task_id:task,surface_id:row.surface,coordinator_pd:3}).unwrap()
}
fn publish(stores:&mut Set,b:Binding,sequence:u32,pixel:u32){
 let g=Geometry{x:10,y:40,width:1,height:1,scale:2};
 assert!(stores.receive(b.target(),b.task(),b.surface(),Request::Begin{sequence,geometry:g}).is_none());
 assert!(stores.receive(b.target(),b.task(),b.surface(),Request::Chunk{sequence,offset:0,count:1,pixels:[pixel,0]}).is_none());
 assert!(stores.receive(b.target(),b.task(),b.surface(),Request::Commit{sequence,hash:bitmap_batch::hash(&[pixel])}).is_some());
}
fn capture(scene:&mut SceneSnapshot,surfaces:&[Surface;MAX_SURFACES],groups:&FrameGroups,stores:&Set)->bool{
 scene.capture(surfaces,groups,stores,&[text_transaction::Stage::EMPTY;MAX_SURFACES],&silkbar_model::DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,153,19,1024,768)
}
fn assert_scrubbed(scene:&SceneSnapshot){
 assert!(!scene.is_valid());assert_eq!((scene.focus,scene.frame,scene.width,scene.height),(0,0,0,0));assert!(scene.blur_rows.iter().all(|row|*row==0));
 assert!(scene.text_projections.iter().all(Option::is_none));
 assert!(scene.bitmaps.pixels_scrubbed());
 for s in &scene.surfaces{
  assert!(!s.active);assert_eq!((s.owner_pd,s.surface_id,s.text_len,s.selection_start,s.selection_end,s.cell_count,s.line_count),(0,0,0,0,0,0,0));
  assert_eq!((s.x,s.y,s.w,s.h,s.fill_count),(0,0,0,0,0));
  assert!(s.text_buf.iter().all(|b|*b==0));assert!(s.cell_starts.iter().all(|b|*b==0));assert!(s.line_starts.iter().all(|b|*b==0));assert!(s.line_cells.iter().all(|b|*b==0));
  assert!(s.fill_color.iter().all(|b|*b==0));assert!(s.fill_sx.iter().all(|b|*b==0));
 }
 assert_eq!(scene.bar.civil_clock.sequence(),0);assert_eq!(scene.bar.civil_clock.seconds(),0);
 assert_eq!(scene.tokens.frame_rim_color,DEFAULT_RENDER_TOKENS.frame_rim_color);
}
#[test]fn constant_static_backing_is_fixed_and_starts_invalid(){
 static BACKING:std::sync::Mutex<SceneSnapshot>=std::sync::Mutex::new(SceneSnapshot::EMPTY);
 let mut scene=BACKING.lock().unwrap();let pointer=&*scene as*const SceneSnapshot;
 assert_scrubbed(&scene);assert!(capture(&mut scene,&[SURFACE_EMPTY;MAX_SURFACES],&FrameGroups::new(),&Set::UNBOUND));
 assert_eq!(&*scene as*const SceneSnapshot,pointer);assert!(scene.is_valid());assert!(scene.bitmaps.pixels_scrubbed());
 println!("[scene.storage] scene={} surfaces={} groups={} published={} bar={} tokens={} count={}",core::mem::size_of::<SceneSnapshot>(),core::mem::size_of::<[Surface;MAX_SURFACES]>(),core::mem::size_of::<FrameGroups>(),core::mem::size_of::<bitmap_stores::PublishedStores<N>>(),core::mem::size_of::<silkbar_model::SilkBar>(),core::mem::size_of::<RenderTokensV1>(),N);
}
#[test]fn every_surface_slot_preserves_all_bytes_selection_layout_fill_and_cursor(){
 let mut source=[SURFACE_EMPTY;MAX_SURFACES];let mut scene=SceneSnapshot::EMPTY;
 for(index,s)in source.iter_mut().enumerate(){
  s.owner_pd=3;s.surface_id=if index==15{0x90}else{1000+index as u64};s.active=true;s.x=index as i32;s.y=51+index as i32;s.w=100;s.h=80;s.color=0x800000+index as u32;
  s.text_len=7;s.text_buf.fill(index as u8);s.text_buf[..7].copy_from_slice("é\tA\nß".as_bytes());s.selection_start=2;s.selection_end=7;s.text_color=0xfedcba;
  s.line_starts.fill(index as u16);s.cell_starts.fill(index as u16+1);s.line_cells.fill(index as u16+2);s.cell_count=7;s.line_count=2;s.wrap_cols=18;
  s.fill_count=8;s.fill_color.fill(index as u32+1);s.fill_sx.fill(index as i32+2);s.tab_count=3;s.active_tab=1;s.chrome_flags=1;
 }
 assert!(capture(&mut scene,&source,&FrameGroups::new(),&Set::UNBOUND));
 for(index,s)in scene.surfaces.iter().enumerate(){
  let original=&source[index];assert_eq!(s.text_buf,original.text_buf);assert_eq!(s.line_starts,original.line_starts);assert_eq!(s.cell_starts,original.cell_starts);assert_eq!(s.line_cells,original.line_cells);assert_eq!(s.fill_color,original.fill_color);assert_eq!(s.fill_sx,original.fill_sx);
  assert_eq!((s.text_len,s.selection_start,s.selection_end,s.cell_count,s.line_count,s.wrap_cols),(7,2,7,7,2,18));assert_eq!((s.owner_pd,s.surface_id,s.x,s.y,s.color,s.tab_count,s.active_tab,s.chrome_flags),(original.owner_pd,original.surface_id,original.x,original.y,original.color,3,1,1));
 }
 source.fill(SURFACE_EMPTY);assert_eq!(&scene.surfaces[0].text_buf[..7],"é\tA\nß".as_bytes());assert_eq!(scene.surfaces[15].surface_id,0x90);assert!(scene.surfaces[15].active);
}
#[test]fn copied_group_parent_state_and_bar_tokens_do_not_follow_source_mutation(){
 let catalog=Catalog{slot:39,target_pd:12,task_id:120,surface_id:154,coordinator_pd:3};let mut groups=FrameGroups::new();groups.register(catalog);
 let child=Surface{owner_pd:12,surface_id:154,active:true,..SURFACE_EMPTY};let mut surfaces=[SURFACE_EMPTY;MAX_SURFACES];surfaces[0]=child;
 let mut bar=silkbar_model::DEFAULT_SILK_BAR;
 use silkbar_model::civil_clock::{Update,State};let update=Update{state:State::Snapshot,seconds:1793491200,sequence:1}.encode();
 bar.civil_enabled=true;bar.civil_clock.receive((6,6),(6,6),[update.kind as u64,(update.index as u64)<<32|update.a as u64,update.b as u64],1577836800,4102444799,1).unwrap();
 let frame=core::sync::atomic::AtomicU64::new(91);let mut tokens=DEFAULT_RENDER_TOKENS;tokens.frame_rim_color=0x123456;let mut scene=SceneSnapshot::EMPTY;
 assert!(scene.capture(&surfaces,&groups,&Set::UNBOUND,&[text_transaction::Stage::EMPTY;MAX_SURFACES],&bar,tokens,153,frame.load(core::sync::atomic::Ordering::Relaxed),1280,800));
 let parent=Surface{owner_pd:3,surface_id:153,active:true,..SURFACE_EMPTY};groups.observe(&[parent]);bar.civil_clock=silkbar_model::civil_clock::CivilClock::new();tokens.frame_rim_color=0;frame.store(92,core::sync::atomic::Ordering::Relaxed);assert_eq!(bar.civil_clock.seconds(),0);assert_eq!(tokens.frame_rim_color,0);
 assert_eq!(groups.awaiting_parent(&child),None);assert_eq!(scene.groups.awaiting_parent(&child),Some(153));assert_eq!(scene.bar.civil_clock.seconds(),1793491200);assert_eq!(scene.tokens.frame_rim_color,0x123456);assert_eq!((scene.width,scene.height,scene.focus,scene.frame),(1280,800,153,91));assert_eq!(frame.load(core::sync::atomic::Ordering::Relaxed),92);let retained=scene.blur_rows;let next=scene_bar_blur_rows(200);assert_eq!(retained,scene_bar_blur_rows(800));assert_ne!(retained,next);
}
#[test]fn pending_bitmap_protocol_is_excluded_and_committed_pixels_are_owned(){
 let mut stores=Set::UNBOUND;let b=bind(15,150);assert!(stores.initialize_once(b));publish(&mut stores,b,1,0xff123456);
 // Settle the prior transfer before deliberately staging the next one.
 assert_eq!(stores.release_after_publication(),1);stores.poll_reply(|_,_|0);
 let mut scene=SceneSnapshot::EMPTY;assert!(capture(&mut scene,&[SURFACE_EMPTY;MAX_SURFACES],&FrameGroups::new(),&stores));
 assert_eq!(scene.bitmaps.sample(15,300,10,40),Some(0xff123456));
 stores.receive(15,150,300,Request::Begin{sequence:2,geometry:Geometry{x:10,y:40,width:1,height:1,scale:2}});
 stores.receive(15,150,300,Request::Chunk{sequence:2,offset:0,count:1,pixels:[0xffabcdef,0]});
 assert!(capture(&mut scene,&[SURFACE_EMPTY;MAX_SURFACES],&FrameGroups::new(),&stores));assert_eq!(scene.bitmaps.sample(15,300,10,40),Some(0xff123456));
 stores.retire(15,150);assert_eq!(stores.sample(15,300,10,40),None);assert_eq!(scene.bitmaps.sample(15,300,10,40),Some(0xff123456));
 scene.scrub();assert_scrubbed(&scene);assert_eq!(scene.bitmaps.binding_for(15,300),None);assert_eq!(scene.bitmaps.sample(15,300,10,40),None);
}
#[test]fn failed_bitmap_capture_scrubs_the_entire_old_scene(){
 let mut source=[SURFACE_EMPTY;MAX_SURFACES];source[15].text_buf.fill(0x73);source[15].active=true;source[15].selection_end=512;
 let mut good=Set::UNBOUND;let b=bind(15,150);assert!(good.initialize_once(b));publish(&mut good,b,1,0xff123456);
 let mut scene=SceneSnapshot::EMPTY;assert!(capture(&mut scene,&source,&FrameGroups::new(),&good));
 let mut malformed=Set::UNBOUND;assert!(malformed.initialize_once(bind(15,150)));assert!(malformed.initialize_once(bind(30,150)));
 assert!(!capture(&mut scene,&source,&FrameGroups::new(),&malformed));assert_scrubbed(&scene);assert_eq!(scene.bitmaps.binding_for(15,300),None);
}
#[test]fn invalid_dimensions_clear_old_data_and_positive_parent_bounds_are_preserved(){
 let mut scene=SceneSnapshot::EMPTY;let mut source=[SURFACE_EMPTY;MAX_SURFACES];source[0].text_buf.fill(0x53);
 for(width,height)in[(0,768),(1024,0),(MAX_FB_W+1,768),(1024,MAX_FB_H+1),(usize::MAX,usize::MAX)]{
  assert!(capture(&mut scene,&source,&FrameGroups::new(),&Set::UNBOUND));
  assert!(!scene.capture(&source,&FrameGroups::new(),&Set::UNBOUND,&[text_transaction::Stage::EMPTY;MAX_SURFACES],&silkbar_model::DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,153,19,width,height));assert_scrubbed(&scene);
 }
 for(width,height)in[(1,1),(MAX_FB_W,MAX_FB_H)]{assert!(scene.capture(&source,&FrameGroups::new(),&Set::UNBOUND,&[text_transaction::Stage::EMPTY;MAX_SURFACES],&silkbar_model::DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,153,19,width,height));assert_eq!((scene.width,scene.height),(width,height));}
 // Compare the actual pure capture helper with the actual legacy blur
 // implementation, not a fixture-only copy of its arithmetic.
 for height in [1usize,2,27,28,49,50,768,800]{
  let rows=scene_bar_blur_rows(height);assert!(fill_bar_bg_buffer(1280,height as u32));assert!(blur_bar_bg_buffer_radius1(1280,height as u32));
  for y in 0..28usize.min(height){for x in [0,1,640,1279]{assert_eq!(rows[y],unsafe{BAR_BLUR_BUF[y*BAR_BG_W_CAP+x]},"height{height} y{y} x{x}");}}
  assert!(rows[28usize.min(height)..].iter().all(|row|*row==0));
 }
 scene.scrub();scene.scrub();assert_scrubbed(&scene);
}

// Only kernel-service and renderer/IPC boundaries are doubles. Scene, lease,
// lifecycle and cleanup/finish function bodies are actual production sources.
#[macro_export]macro_rules! serial_record{($($args:tt)*)=>{{let _=format_args!($($args)*);}}}
static HOST_LEASE_LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut FB_PTR:u64=0;
static mut FB_W:u32=1024;
static mut FB_H:u32=768;
static mut LAST_REDRAWN_CLOCK_CANON_SS:u8=0;
mod boundary{
 use super::*;
 #[derive(Default,Clone,Copy)]pub struct Counts{pub queries:usize,pub acks:usize,pub replies:usize,pub renders:usize,pub cursor_scrubs:usize,pub terminal:bool,pub fail_render:bool,pub fail_ack:bool}
 static STATE:std::sync::Mutex<Counts>=std::sync::Mutex::new(Counts{queries:0,acks:0,replies:0,renders:0,cursor_scrubs:0,terminal:false,fail_render:false,fail_ack:false});
 pub fn reset(){*STATE.lock().unwrap()=Counts::default();}
 pub fn get()->Counts{*STATE.lock().unwrap()}
 pub fn terminal(){STATE.lock().unwrap().terminal=true;}
 pub fn fail_ack(value:bool){STATE.lock().unwrap().fail_ack=value;}
 pub fn fail_render(value:bool){STATE.lock().unwrap().fail_render=value;}
 pub fn rendering()->bool{let mut s=STATE.lock().unwrap();s.renders+=1;s.fail_render}
 pub fn cursor(){STATE.lock().unwrap().cursor_scrubs+=1;}
 pub fn info(slot:usize)->Result<app_lifecycle::LifecycleInfo,u64>{let mut s=STATE.lock().unwrap();s.queries+=1;assert_eq!(slot,40);if !s.terminal{return Err(app_lifecycle::LIVE)}Ok(app_lifecycle::LifecycleInfo{target_pd:15,task_id:150,required_mask:app_lifecycle::ROLE_DISPLAY,acked_mask:0})}
 pub fn ack(slot:usize,task:u32)->Result<(),u64>{assert_eq!((slot,task),(40,150));unsafe{assert_scrubbed(&*core::ptr::addr_of!(RENDER_SCENE));}let mut state=STATE.lock().unwrap();if state.fail_ack{return Err(ERR_SERVICE_NOT_READY);}state.acks+=1;Ok(())}
 pub fn reply(_target:u32,_word:u64)->u64{unsafe{assert!(!(*core::ptr::addr_of!(RENDER_SCENE)).is_valid());}STATE.lock().unwrap().replies+=1;0}
}
pub fn pdx_reply(target:u32,word:u64)->u64{boundary::reply(target,word)}
fn scrub_cursor_under(){boundary::cursor();}
// Controlled publication boundary: obtain/drop an actual scoped Scene lease,
// then construct the descriptive receipt. No device write/native success is
// claimed; the actual prepared-output pipeline has its own registered tests.
fn rendered_receipt(plan:Plan,present_bits:u64,bar:&SilkBar)->Option<PublishedFrame>{
 let fail=boundary::rendering();let (text_projections,bar_height)={
  let lease=SceneLease::capture(1024,768,bar)?;assert!(lease.view().is_valid());if fail{return None;}
  (lease.view().text_projections,lease.view().metrics.bar()as usize)
 };
 scene_scrubbed();Some(PublishedFrame{plan,bar_height,present_bits,canon_ss:0,text_projections})
}

fn reset_host_inputs(){
 boundary::reset();unsafe{assert!(!(*core::ptr::addr_of!(RENDER_SCENE)).is_valid());(*core::ptr::addr_of_mut!(RENDER_SCENE)).scrub();SURFACES.fill(SURFACE_EMPTY);FRAME_GROUPS=FrameGroups::new();DISPLAY_TOKENS=DEFAULT_RENDER_TOKENS;FOCUSED_SURFACE_ID=0;TOKEN_BUF_CALL1_RECEIVED=false;TOKEN_BUF_OWNER=0;CURSOR_UNDER_VALID=false;BITMAP_STORES=Set::UNBOUND;TEXT_STAGES.fill(text_transaction::Stage::EMPTY);FRAME_OUTPUT=None;PREPARED_CURSOR=None;FB_W=1024;FB_H=768;}
 RENDER_FRAME_COUNTER.store(0,core::sync::atomic::Ordering::Relaxed);
}
fn scene_scrubbed(){unsafe{assert_scrubbed(&*core::ptr::addr_of!(RENDER_SCENE));}}
#[test]fn actual_lease_rejects_nested_capture_without_changing_live_snapshot(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;
 unsafe{SURFACES[0].text_buf.fill(0x61);SURFACES[0].text_len=1;SURFACES[0].active=true;}RENDER_FRAME_COUNTER.store(77,core::sync::atomic::Ordering::Relaxed);
 let lease=SceneLease::capture(1024,768,&bar).unwrap();let addr=lease.view()as*const _;
 // Keep no shared borrow across the deliberately broken recapture mutant.
 // The separate source guard verifies shared-before-exclusive reentry checks.
 assert!(SceneLease::capture(1,1,&bar).is_none());let view=lease.view();assert_eq!(view as*const _,addr);assert_eq!(view.frame,77);assert_eq!(view.surfaces[0].text_buf[511],0x61);assert_eq!((view.width,view.height),(1024,768));drop(lease);scene_scrubbed();
}
#[test]fn actual_lease_owns_surface_frame_bar_and_blur_after_live_inputs_change(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let mut bar=silkbar_model::DEFAULT_SILK_BAR;bar.phase1.active_app_sid=300;
 unsafe{SURFACES[0].text_buf.fill(0x62);SURFACES[0].text_len=1;SURFACES[0].active=true;DISPLAY_TOKENS.frame_rim_color=0x123456;FOCUSED_SURFACE_ID=300;}
 RENDER_FRAME_COUNTER.store(77,core::sync::atomic::Ordering::Relaxed);let lease=SceneLease::capture(1024,768,&bar).unwrap();
 unsafe{SURFACES[0].text_buf.fill(0x99);DISPLAY_TOKENS.frame_rim_color=0;FOCUSED_SURFACE_ID=344;FB_H=400;BAR_BLUR_BUF.fill(0xdeadbeef);}bar.phase1.active_app_sid=344;RENDER_FRAME_COUNTER.store(88,core::sync::atomic::Ordering::Relaxed);
 let view=lease.view();assert_eq!(view.frame,77);assert_eq!(view.surfaces[0].text_buf[511],0x62);assert_eq!(view.bar.phase1.active_app_sid,300);assert_eq!(view.tokens.frame_rim_color,0x123456);assert_eq!(view.focus,300);assert_eq!(view.blur_rows,scene_bar_blur_rows(768));assert_ne!(view.blur_rows,scene_bar_blur_rows(400));drop(lease);scene_scrubbed();assert_eq!(bar.phase1.active_app_sid,344);
}
#[test]fn actual_lease_drop_early_return_and_host_unwind_scrub_owned_payload(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;unsafe{SURFACES[0].text_buf.fill(0x71);SURFACES[0].text_len=1;SURFACES[0].active=true;}
 {let lease=SceneLease::capture(1024,768,&bar).unwrap();assert_eq!(lease.view().surfaces[0].text_buf[511],0x71);}scene_scrubbed();
 let early=||{let lease=SceneLease::capture(1024,768,&bar).unwrap();assert!(lease.view().is_valid());return 7;};assert_eq!(early(),7);scene_scrubbed();
 let panic=std::panic::catch_unwind(||{let lease=SceneLease::capture(1024,768,&bar).unwrap();assert_eq!(lease.view().surfaces[0].text_buf[511],0x71);panic!("controlled host-only unwind");});assert!(panic.is_err());scene_scrubbed();
 let lease=SceneLease::capture(1024,768,&bar).unwrap();assert!(lease.view().is_valid());drop(lease);scene_scrubbed();
}
#[test]fn actual_failed_lease_capture_returns_none_and_erases_invalid_old_backing(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;
 for(w,h)in[(0,768),(1024,0),(MAX_FB_W+1,768),(1024,MAX_FB_H+1),(usize::MAX,usize::MAX)]{
  // Fault injection in an invalid destination checks failure erasure, not an
  // authorized producer path. No held lease/reference exists here.
  unsafe{let dst=&mut*core::ptr::addr_of_mut!(RENDER_SCENE);assert!(!dst.is_valid());dst.surfaces[0].text_buf.fill(0x77);dst.blur_rows.fill(0xff);dst.frame=99;}
  assert!(SceneLease::capture(w,h,&bar).is_none());scene_scrubbed();
 }
}
// Small owned aligned host spans exercise the actual Store cleanup. They are
// explicit simulated syscall replies, not native held-region authority proof.
struct HostFrameRegion{pointer:*mut u8,allocation:std::alloc::Layout,layout:display_frames::Layout}
impl HostFrameRegion{
 fn new()->Self{
  let layout=display_frames::Layout::new(19,7,76).unwrap();let allocation=std::alloc::Layout::from_size_align(layout.total_bytes()as usize,4096).unwrap();
  let pointer=unsafe{std::alloc::alloc(allocation)};assert!(!pointer.is_null());unsafe{core::ptr::write_bytes(pointer,0xa5,allocation.size());}
  Self{pointer,allocation,layout}
 }
 unsafe fn install(&self){
  use memory_region::*;
  let grant=RegionGrant{region_id:display_frames::REGION_ID,virtual_base:self.pointer as u64,length:self.layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE,dma_base:None,peer_pd:None};
  let info=resolve_info(Some(&grant)).unwrap();let view=resolve_view(Some(&grant),ViewRequest{offset:0,length:self.layout.total_bytes(),rights:QUERY_VIEW|READ|WRITE}).unwrap();
  FRAME_OUTPUT=Some(frame_output::Store::new(display_frames::FrameBacking::from_resolved(self.layout,info,view).unwrap()));
  for color in [0x11223344,0x55667788]{let output=FRAME_OUTPUT.as_mut().unwrap();let mut preparation=output.prepare(true).unwrap();{let target=preparation.target();for i in 0..target.len(){target.pointer().add(i).write(color);}}assert!(preparation.commit(|_|true));}
 }
 fn zeroed(&self)->bool{unsafe{(0..self.allocation.size()).all(|i|self.pointer.add(i).read_volatile()==0)}}
}
impl Drop for HostFrameRegion{fn drop(&mut self){unsafe{FRAME_OUTPUT=None;std::alloc::dealloc(self.pointer,self.allocation);}}}
#[test]fn actual_cleanup_functions_withhold_effects_and_ack_until_render_lease_drops(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let frames=HostFrameRegion::new();unsafe{frames.install();PREPARED_CURSOR=Some(PreparedCursor{x:1,y:2,seq:3,shell_tick:4});}assert!(!frames.zeroed());boundary::terminal();let mut bar=silkbar_model::DEFAULT_SILK_BAR;
 let mut lifecycle=surface_lifecycle::Lifecycle::new();assert!(lifecycle.register(Catalog{slot:40,target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}));
 unsafe{SURFACES[0]=Surface{owner_pd:15,surface_id:300,active:true,text_len:1,..SURFACE_EMPTY};SURFACES[0].text_buf.fill(0x73);}
 let lease=SceneLease::capture(1024,768,&bar).unwrap();assert!(!prepare_app_cleanup(&mut lifecycle,&mut bar));assert_eq!(boundary::get().queries,0);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Live));assert_eq!(lease.view().surfaces[0].text_buf[511],0x73);assert!(!frames.zeroed());unsafe{assert!(PREPARED_CURSOR.is_some());}drop(lease);scene_scrubbed();
 assert!(prepare_app_cleanup(&mut lifecycle,&mut bar));assert!(frames.zeroed());unsafe{assert!(PREPARED_CURSOR.is_none());assert!(FRAME_OUTPUT.as_ref().unwrap().front_pixels().is_none());assert_eq!(FRAME_OUTPUT.as_ref().unwrap().generation(),2);}assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Prepared));assert_eq!(boundary::get().queries,1);
 let full=rendered_receipt(Plan::full(),0,&bar).unwrap();let before=boundary::get().renders;
 let lease=SceneLease::capture(1024,768,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&full));assert_eq!(boundary::get().renders,before);assert_eq!(boundary::get().acks,0);drop(lease);scene_scrubbed();
 finish_display_transactions(&mut lifecycle,None);assert_eq!(boundary::get().acks,0);
 let partial=rendered_receipt(Plan{content:Content::Area,strip:true},0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&partial));assert_eq!(boundary::get().acks,0);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Prepared));
 boundary::fail_render(true);let failed=rendered_receipt(Plan::full(),0,&bar);assert!(failed.is_none());let before=boundary::get().renders;finish_display_transactions(&mut lifecycle,failed.as_ref());scene_scrubbed();assert_eq!(boundary::get().renders,before);assert_eq!(boundary::get().acks,0);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Prepared));
 boundary::fail_render(false);let full=rendered_receipt(Plan::full(),0,&bar).unwrap();let before=boundary::get().renders;boundary::fail_ack(true);finish_display_transactions(&mut lifecycle,Some(&full));scene_scrubbed();assert_eq!(boundary::get().acks,0);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Committed));boundary::fail_ack(false);finish_display_transactions(&mut lifecycle,None);assert_eq!(boundary::get().renders,before);assert_eq!(boundary::get().acks,1);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Acknowledged));finish_display_transactions(&mut lifecycle,None);assert_eq!(boundary::get().acks,1);
}

#[test]fn actual_finish_uses_full_receipt_captured_bits_only_after_scene_release(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;
 let mut lifecycle=surface_lifecycle::Lifecycle::new();assert!(lifecycle.register(Catalog{slot:40,target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}));assert!(lifecycle.request_present(15,300,7,0));
 let bit=surface_lifecycle::native_pixel_bit(15,300);assert_ne!(bit,0);
 let absent=rendered_receipt(Plan::full(),0,&bar).unwrap();unsafe{APP_PRESENT_BITS=bit;}finish_display_transactions(&mut lifecycle,Some(&absent));assert_eq!(boundary::get().replies,0);assert!(lifecycle.needs_presentation());
 let partial=rendered_receipt(Plan{content:Content::Area,strip:true},bit,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&partial));assert_eq!(boundary::get().replies,0);assert!(lifecycle.needs_presentation());
 let full=rendered_receipt(Plan::full(),bit,&bar).unwrap();unsafe{APP_PRESENT_BITS=0;}let lease=SceneLease::capture(1024,768,&bar).unwrap();let before=boundary::get().renders;finish_display_transactions(&mut lifecycle,Some(&full));assert_eq!(boundary::get().replies,0);assert_eq!(boundary::get().renders,before);drop(lease);scene_scrubbed();
 finish_display_transactions(&mut lifecycle,Some(&full));assert_eq!(boundary::get().replies,1);assert!(!lifecycle.needs_presentation());assert_eq!(boundary::get().renders,before);finish_display_transactions(&mut lifecycle,None);assert_eq!(boundary::get().replies,1);
}


// These direct Stage calls represent already-admitted stamped sender tuples.
// The fixture does not replace or claim Main's separate caller authority checks.
fn typed_text(index:usize,owner:u32,task:u32,sid:u64,sequence:u32,bytes:&[u8]){
 unsafe{
  let stage=&mut TEXT_STAGES[index];assert!(stage.receive(owner,task,sid,text_batch::Request::Begin{sequence,len:bytes.len(),color:0x123456}).is_none());
  for(offset,chunk)in bytes.chunks(8).enumerate(){let mut data=[0;8];data[..chunk.len()].copy_from_slice(chunk);assert!(stage.receive(owner,task,sid,text_batch::Request::Chunk{sequence,offset:offset*8,count:chunk.len(),bytes:data}).is_none());}
  assert!(stage.receive(owner,task,sid,text_batch::Request::Commit{sequence,hash:text_batch::hash(bytes)}).is_some());
 }
}
fn typed_poll(mut send:impl FnMut(u32,u64)->u64){unsafe{for stage in &mut TEXT_STAGES{stage.poll_reply(&mut send);}BITMAP_STORES.poll_reply(&mut send);}}
fn no_applied(){typed_poll(|_,_|panic!("Applied escaped before its full publication release"));}
fn typed_bitmap_clear(sequence:u32){unsafe{assert!(BITMAP_STORES.receive(15,150,300,Request::Clear{sequence}).is_some());}}
#[test]fn actual_typed_text_noop_empty_and_hidden_completion_force_publication(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;let mut lifecycle=surface_lifecycle::Lifecycle::new();
 unsafe{SURFACES[0]=Surface{owner_pd:9,surface_id:156,active:true,w:200,h:100,..SURFACE_EMPTY};}
 for(sequence,bytes)in[(1u32,&b""[..]),(2,&b""[..]),(3,&b"hidden"[..])]{
  assert!(!typed_completion_needs_publication());typed_text(0,9,90,156,sequence,bytes);
  assert!(damage::Damage::new(28).rect().is_none());assert!(typed_completion_needs_publication());no_applied();
  let receipt=rendered_receipt(Plan::full(),0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&receipt));assert!(!typed_completion_needs_publication());
  let mut got=Vec::new();typed_poll(|owner,word|{got.push((owner,word));0});assert_eq!(got,[(9,text_batch::Applied{surface:156,sequence}.encode().unwrap())]);no_applied();
 }
}
#[test]fn actual_typed_bitmap_empty_and_hidden_completion_force_publication(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;let mut lifecycle=surface_lifecycle::Lifecycle::new();unsafe{assert!(BITMAP_STORES.initialize_once(bind(15,150)));}
 for sequence in[1,2]{typed_bitmap_clear(sequence);assert!(typed_completion_needs_publication());no_applied();let receipt=rendered_receipt(Plan::full(),0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&receipt));assert!(!typed_completion_needs_publication());typed_poll(|owner,word|{assert_eq!((owner,word),(15,bitmap_batch::Applied{surface:300,sequence}.encode().unwrap()));0});}
 unsafe{publish(&mut BITMAP_STORES,bind(15,150),3,0xffabcdef);BITMAP_STORES.reset_surface(15,300);}assert!(typed_completion_needs_publication());no_applied();let receipt=rendered_receipt(Plan::full(),0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&receipt));let mut words=Vec::new();typed_poll(|p,w|{words.push((p,w));0});assert_eq!(words,[(15,bitmap_batch::Applied{surface:300,sequence:3}.encode().unwrap())]);
}
#[test]fn actual_typed_completions_wait_for_full_scene_release_then_retry_independently(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;let mut lifecycle=surface_lifecycle::Lifecycle::new();unsafe{assert!(BITMAP_STORES.initialize_once(bind(15,150)));}
 unsafe{SURFACES[0]=Surface{owner_pd:9,surface_id:156,active:true,y:100,w:200,h:100,..SURFACE_EMPTY};}
 typed_text(0,9,90,156,7,b"same admitted text");typed_bitmap_clear(11);assert!(typed_completion_needs_publication());no_applied();
 finish_display_transactions(&mut lifecycle,None);no_applied();let partial=rendered_receipt(Plan{content:Content::Cursor,strip:false},0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&partial));no_applied();assert!(typed_completion_needs_publication());
 boundary::fail_render(true);let failed=rendered_receipt(Plan::full(),0,&bar);assert!(failed.is_none());finish_display_transactions(&mut lifecycle,failed.as_ref());no_applied();boundary::fail_render(false);unsafe{assert_eq!(SURFACES[0].text_len,0);assert!(SURFACES[0].text_buf.iter().all(|b|*b==0));}
 let full=rendered_receipt(Plan::full(),0,&bar).unwrap();let lease=SceneLease::capture(1024,768,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&full));no_applied();assert!(typed_completion_needs_publication());drop(lease);scene_scrubbed();
 finish_display_transactions(&mut lifecycle,Some(&full));assert!(!typed_completion_needs_publication());unsafe{assert_eq!(&SURFACES[0].text_buf[..18],b"same admitted text");assert_eq!(SURFACES[0].text_len,18);}let renders=boundary::get().renders;let expected=[(9,text_batch::Applied{surface:156,sequence:7}.encode().unwrap()),(15,bitmap_batch::Applied{surface:300,sequence:11}.encode().unwrap())];
 let mut attempts=Vec::new();typed_poll(|p,w|{attempts.push((p,w));if p==9{99}else{0}});assert_eq!(attempts,expected);assert!(!typed_completion_needs_publication());
 for status in[1,u64::MAX,0]{let mut retry=Vec::new();finish_display_transactions(&mut lifecycle,None);typed_poll(|p,w|{retry.push((p,w));status});assert_eq!(retry,[expected[0]]);assert_eq!(boundary::get().renders,renders);}no_applied();
}
#[test]fn actual_terminal_cleanup_cancels_unreleased_typed_effects_before_ack(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();boundary::terminal();let mut bar=silkbar_model::DEFAULT_SILK_BAR;let mut lifecycle=surface_lifecycle::Lifecycle::new();assert!(lifecycle.register(Catalog{slot:40,target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}));unsafe{assert!(BITMAP_STORES.initialize_once(bind(15,150)));SURFACES[0]=Surface{owner_pd:15,surface_id:300,active:true,..SURFACE_EMPTY};}
 typed_text(0,15,150,300,7,b"terminal private text");typed_bitmap_clear(11);assert!(typed_completion_needs_publication());no_applied();assert!(prepare_app_cleanup(&mut lifecycle,&mut bar));assert!(!typed_completion_needs_publication());no_applied();finish_display_transactions(&mut lifecycle,None);assert_eq!(boundary::get().acks,0);
 let full=rendered_receipt(Plan::full(),0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&full));assert_eq!(boundary::get().acks,1);no_applied();assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Acknowledged));
}

fn pending_text(stage:&mut text_transaction::Stage,owner:u32,task:u32,sid:u64,sequence:u32,bytes:&[u8],selection:(u16,u16)){
 assert!(stage.receive(owner,task,sid,text_batch::Request::Begin{sequence,len:bytes.len(),color:0xff123456}).is_none());
 for(offset,chunk)in bytes.chunks(8).enumerate(){let mut data=[0;8];data[..chunk.len()].copy_from_slice(chunk);assert!(stage.receive(owner,task,sid,text_batch::Request::Chunk{sequence,offset:offset*8,count:chunk.len(),bytes:data}).is_none());}
 assert!(stage.receive(owner,task,sid,text_batch::Request::Select{sequence,start:selection.0 as usize,end:selection.1 as usize}).is_none());
 assert!(stage.receive(owner,task,sid,text_batch::Request::Commit{sequence,hash:text_batch::selected_hash(bytes,selection.0 as usize,selection.1 as usize).unwrap()}).is_some());
}
fn capture_pending(scene:&mut SceneSnapshot,surfaces:&[Surface;MAX_SURFACES],texts:&[text_transaction::Stage;MAX_SURFACES])->bool{
 scene.capture(surfaces,&FrameGroups::new(),&Set::UNBOUND,texts,&silkbar_model::DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,0,23,1024,768)
}
fn text_surface(owner:u32,sid:u64)->Surface{
 let mut s=Surface{owner_pd:owner,surface_id:sid,active:true,y:100,w:200,h:100,text_len:3,..SURFACE_EMPTY};s.text_buf[..3].copy_from_slice(b"old");s
}
#[test]fn pending_text_projection_owns_utf8_selection_layout_without_canonical_mutation(){
 let mut surfaces=[SURFACE_EMPTY;MAX_SURFACES];surfaces[0]=text_surface(9,156);
 let mut texts=[text_transaction::Stage::EMPTY;MAX_SURFACES];pending_text(&mut texts[0],9,90,156,7,"éA\nß".as_bytes(),(0,3));
 let mut scene=SceneSnapshot::EMPTY;assert!(capture_pending(&mut scene,&surfaces,&texts));
 assert_eq!(&surfaces[0].text_buf[..3],b"old");assert_eq!(surfaces[0].text_len,3);assert_eq!(surfaces[0].cell_count,0);
 let s=&scene.surfaces[0];assert_eq!(&s.text_buf[..6],"éA\nß".as_bytes());assert_eq!(s.text_len,6);assert!(s.text_buf[6..].iter().all(|b|*b==0));assert_eq!((s.selection_start,s.selection_end),(0,3));assert_eq!(s.text_color,0xff123456);
 assert_eq!((s.cell_count,s.line_count,s.wrap_cols),(4,2,30));assert_eq!(&s.cell_starts[..4],&[0,2,3,4]);assert_eq!(&s.line_starts[..2],&[0,4]);assert_eq!(&s.line_cells[..2],&[0,3]);
 let projection=scene.text_projections[0].unwrap();let c=projection.context;assert_eq!((c.owner(),c.task(),c.surface(),c.sequence()),(9,90,156,7));assert!(scene.text_projections[1..].iter().all(Option::is_none));
 texts[0].reset();surfaces[0].text_buf.fill(b'X');assert_eq!(&scene.surfaces[0].text_buf[..6],"éA\nß".as_bytes());assert_eq!(scene.text_projections[0],Some(projection));scene.scrub();assert_scrubbed(&scene);
}
#[test]fn pending_text_projection_covers_first_and_last_slots_in_one_owned_scene(){
 let mut surfaces=[SURFACE_EMPTY;MAX_SURFACES];let mut texts=[text_transaction::Stage::EMPTY;MAX_SURFACES];let last=MAX_SURFACES-1;
 for(index,owner,task,sid,bytes)in[(0,9,90,156,&b"first"[..]),(last,15,150,300,&b"last"[..])]{surfaces[index]=text_surface(owner,sid);pending_text(&mut texts[index],owner,task,sid,11+index as u32,bytes,(0,0));}
 let mut scene=SceneSnapshot::EMPTY;assert!(capture_pending(&mut scene,&surfaces,&texts));assert_eq!(&scene.surfaces[0].text_buf[..5],b"first");assert_eq!(&scene.surfaces[last].text_buf[..4],b"last");assert_eq!(scene.text_projections.iter().flatten().count(),2);
 for index in[0,last]{assert_eq!(scene.text_projections[index].unwrap().context,texts[index].pending().unwrap().context());assert_eq!(&surfaces[index].text_buf[..3],b"old");texts[index].poll_reply(|_,_|panic!("capture is not publication"));}
 // Equal canonical bytes still need a context and publication, not a changed-only result.
 surfaces[0]=scene.surfaces[0];assert!(capture_pending(&mut scene,&surfaces,&texts));assert!(scene.text_projections[0].is_some());
}
#[test]fn unmatched_pending_text_fails_closed_and_scrubs_prior_and_partial_projection(){
 for mismatch in 0..4{
  let mut surfaces=[SURFACE_EMPTY;MAX_SURFACES];surfaces[0]=text_surface(9,156);surfaces[1]=text_surface(15,300);
  let mut texts=[text_transaction::Stage::EMPTY;MAX_SURFACES];pending_text(&mut texts[0],9,90,156,1,b"first",(0,0));pending_text(&mut texts[1],15,150,300,2,b"last",(0,0));
  let mut scene=SceneSnapshot::EMPTY;assert!(capture_pending(&mut scene,&surfaces,&texts));
  match mismatch{0=>surfaces[1].active=false,1=>surfaces[1].owner_pd=30,2=>surfaces[1].surface_id=343,_=>texts.swap(1,2)}
  assert!(!capture_pending(&mut scene,&surfaces,&texts));assert_scrubbed(&scene);assert!(texts[0].needs_publication());
 }
}
#[test]fn reset_exact_retirement_and_released_receipts_leave_no_pending_projection(){
 for cancel in 0..3{
  let mut surfaces=[SURFACE_EMPTY;MAX_SURFACES];surfaces[0]=text_surface(9,156);let mut texts=[text_transaction::Stage::EMPTY;MAX_SURFACES];pending_text(&mut texts[0],9,90,156,7,b"candidate",(0,0));
  texts[0].retire(9,9);assert!(texts[0].pending().is_some());
  match cancel{0=>texts[0].reset(),1=>texts[0].retire(9,90),_=>{let context=texts[0].pending().unwrap().context();assert!(texts[0].release_matching(context));}}
  assert!(texts[0].pending().is_none());let mut scene=SceneSnapshot::EMPTY;assert!(capture_pending(&mut scene,&surfaces,&texts));assert!(scene.text_projections.iter().all(Option::is_none));assert_eq!(&scene.surfaces[0].text_buf[..3],b"old");
 }
}

#[test]fn actual_finish_rejects_crossed_or_cancelled_captured_text_contexts(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();
 for case in 0..5{
  reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;let mut lifecycle=surface_lifecycle::Lifecycle::new();unsafe{SURFACES[0]=text_surface(9,156);}
  typed_text(0,9,90,156,7,b"candidate");let mut stale=rendered_receipt(Plan::full(),0,&bar).unwrap();
  let expected:&[u8]=if case<3{
   let mut other=text_transaction::Stage::EMPTY;pending_text(&mut other,9,if case==0{91}else{90},156,if case==1{8}else{7},b"foreign",(0,0));
   let context=other.pending().unwrap().context();
   let mut projection=stale.text_projections[0].unwrap();projection.context=context;
   if case==2{stale.text_projections[0]=None;stale.text_projections[1]=Some(projection);}else{stale.text_projections[0]=Some(projection);}
   b"candidate"
  }else{
   unsafe{if case==3{TEXT_STAGES[0].reset();}else{TEXT_STAGES[0].retire(9,90);}}
   // A new sequence is intentionally used; Context is not a reset-resistant
   // token for arbitrary reuse of the identical multipart tuple.
   typed_text(0,9,90,156,8,b"replacement");b"replacement"
  };
  finish_display_transactions(&mut lifecycle,Some(&stale));
  unsafe{assert_eq!(SURFACES[0].text_len,3);assert_eq!(&SURFACES[0].text_buf[..3],b"old");assert!(TEXT_STAGES[0].pending().is_some());}
  no_applied();let actual=rendered_receipt(Plan::full(),0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&actual));
  unsafe{assert_eq!(SURFACES[0].text_len as usize,expected.len());assert_eq!(&SURFACES[0].text_buf[..expected.len()],expected);assert!(TEXT_STAGES[0].pending().is_none());}
  let mut replies=0;typed_poll(|owner,word|{assert_eq!((owner,word),(9,text_batch::Applied{surface:156,sequence:if case<3{7}else{8}}.encode().unwrap()));replies+=1;0});assert_eq!(replies,1);
 }
}

#[test]fn actual_partial_text_publication_covers_shortened_old_bounds_before_commit(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();
 for(original,replacement)in[(&b"a\nb\nc\nd\ne\nf"[..],&b"x"[..]),(&b"a\nb\nc\nd\ne\nf"[..],&b""[..]),(&b"x"[..],&b"a\nb\nc\nd\ne\nf"[..])]{
  reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;let mut lifecycle=surface_lifecycle::Lifecycle::new();
  unsafe{SURFACES[0]=Surface{owner_pd:9,surface_id:156,active:true,x:20,y:100,w:200,h:100,text_len:original.len()as u16,text_color:0x123456,..SURFACE_EMPTY};SURFACES[0].text_buf[..original.len()].copy_from_slice(original);recompute_text_lines(&mut SURFACES[0]);}
  typed_text(0,9,90,156,41,replacement);
  // One-row-only damage misses either the old erased rows or the new grown rows.
  let insufficient=rendered_receipt(Plan{content:Content::Damage(damage::Rect{x:20,y:112,right:220,bottom:121}),strip:false},0,&bar).unwrap();
  assert_eq!(insufficient.text_projections[0].unwrap().coverage,scene::TextCoverage::Body(damage::Rect{x:20,y:112,right:220,bottom:166}));
  finish_display_transactions(&mut lifecycle,Some(&insufficient));no_applied();
  unsafe{assert_eq!(&SURFACES[0].text_buf[..original.len()],original);assert!(TEXT_STAGES[0].needs_publication());}
  let complete=rendered_receipt(Plan{content:Content::Damage(damage::Rect{x:20,y:112,right:220,bottom:166}),strip:false},0,&bar).unwrap();
  finish_display_transactions(&mut lifecycle,Some(&complete));
  unsafe{assert_eq!(SURFACES[0].text_len as usize,replacement.len());assert_eq!(&SURFACES[0].text_buf[..replacement.len()],replacement);assert!(SURFACES[0].text_buf[replacement.len()..].iter().all(|b|*b==0));}
  let mut replies=0;typed_poll(|owner,word|{assert_eq!((owner,word),(9,text_batch::Applied{surface:156,sequence:41}.encode().unwrap()));replies+=1;0});assert_eq!(replies,1);
 }
}

#[test]fn actual_settle_requires_explicit_matching_no_pixels_projection_and_keeps_bitmap_pending(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();let bar=silkbar_model::DEFAULT_SILK_BAR;let mut lifecycle=surface_lifecycle::Lifecycle::new();
 unsafe{SURFACES[0]=text_surface(9,156);SURFACES[0].text_color=0x123456;recompute_text_lines(&mut SURFACES[0]);assert!(BITMAP_STORES.initialize_once(bind(15,150)));}
 typed_text(0,9,90,156,51,b"old");typed_bitmap_clear(61);
 let plan=Plan{content:Content::Settle,strip:false};
 let mut absent=rendered_receipt(plan,0,&bar).unwrap();assert_eq!(absent.text_projections[0].unwrap().coverage,scene::TextCoverage::NoPixels);absent.text_projections[0]=None;
 finish_display_transactions(&mut lifecycle,Some(&absent));no_applied();unsafe{assert!(TEXT_STAGES[0].needs_publication());}
 let mut crossed=rendered_receipt(plan,0,&bar).unwrap();let mut other=text_transaction::Stage::EMPTY;pending_text(&mut other,9,91,156,51,b"old",(0,0));crossed.text_projections[0].as_mut().unwrap().context=other.pending().unwrap().context();
 finish_display_transactions(&mut lifecycle,Some(&crossed));no_applied();unsafe{assert!(TEXT_STAGES[0].needs_publication());}
 let exact=rendered_receipt(plan,0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&exact));
 unsafe{assert!(!TEXT_STAGES[0].needs_publication());assert_eq!(SURFACES[0].text_len,3);assert_eq!(&SURFACES[0].text_buf[..3],b"old");}
 let mut replies=Vec::new();typed_poll(|owner,word|{replies.push((owner,word));0});assert_eq!(replies,[(9,text_batch::Applied{surface:156,sequence:51}.encode().unwrap())]);
 assert!(typed_completion_needs_publication());no_applied();
 let full=rendered_receipt(Plan::full(),0,&bar).unwrap();finish_display_transactions(&mut lifecycle,Some(&full));let mut bitmap_replies=Vec::new();typed_poll(|owner,word|{bitmap_replies.push((owner,word));0});assert_eq!(bitmap_replies,[(15,bitmap_batch::Applied{surface:300,sequence:61}.encode().unwrap())]);assert!(!typed_completion_needs_publication());
}
