// Host fixture uses actual SDK, Surface, FrameGroups, bitmap publication and Scene.
// Controlled held metadata is not a runtime grant or native composition proof.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod app_lifecycle_sdk;
pub mod app_lifecycle{
 pub use crate::app_lifecycle_sdk::*;
 pub fn info(slot:usize)->Result<LifecycleInfo,u64>{crate::boundary::info(slot)}
 pub fn ack(slot:usize,task:u32)->Result<(),u64>{crate::boundary::ack(slot,task)}
}
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
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
 scene.capture(surfaces,groups,stores,&silkbar_model::DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,153,19,1024,768)
}
fn assert_scrubbed(scene:&SceneSnapshot){
 assert!(!scene.is_valid());assert_eq!((scene.focus,scene.frame,scene.width,scene.height),(0,0,0,0));assert!(scene.blur_rows.iter().all(|row|*row==0));
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
 assert!(scene.capture(&surfaces,&groups,&Set::UNBOUND,&bar,tokens,153,frame.load(core::sync::atomic::Ordering::Relaxed),1280,800));
 let parent=Surface{owner_pd:3,surface_id:153,active:true,..SURFACE_EMPTY};groups.observe(&[parent]);bar.civil_clock=silkbar_model::civil_clock::CivilClock::new();tokens.frame_rim_color=0;frame.store(92,core::sync::atomic::Ordering::Relaxed);assert_eq!(bar.civil_clock.seconds(),0);assert_eq!(tokens.frame_rim_color,0);
 assert_eq!(groups.awaiting_parent(&child),None);assert_eq!(scene.groups.awaiting_parent(&child),Some(153));assert_eq!(scene.bar.civil_clock.seconds(),1793491200);assert_eq!(scene.tokens.frame_rim_color,0x123456);assert_eq!((scene.width,scene.height,scene.focus,scene.frame),(1280,800,153,91));assert_eq!(frame.load(core::sync::atomic::Ordering::Relaxed),92);let retained=scene.blur_rows;let next=scene_bar_blur_rows(768);assert_eq!(retained,scene_bar_blur_rows(800));assert_ne!(retained,next);
}
#[test]fn pending_bitmap_protocol_is_excluded_and_committed_pixels_are_owned(){
 let mut stores=Set::UNBOUND;let b=bind(15,150);assert!(stores.initialize_once(b));publish(&mut stores,b,1,0xff123456);
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
  assert!(!scene.capture(&source,&FrameGroups::new(),&Set::UNBOUND,&silkbar_model::DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,153,19,width,height));assert_scrubbed(&scene);
 }
 for(width,height)in[(1,1),(MAX_FB_W,MAX_FB_H)]{assert!(scene.capture(&source,&FrameGroups::new(),&Set::UNBOUND,&silkbar_model::DEFAULT_SILK_BAR,DEFAULT_RENDER_TOKENS,153,19,width,height));assert_eq!((scene.width,scene.height),(width,height));}
 // Compare the actual pure capture helper with the actual legacy blur
 // implementation, not a fixture-only copy of its arithmetic.
 for height in [1usize,2,49,50,768,800]{
  let rows=scene_bar_blur_rows(height);assert!(fill_bar_bg_buffer(1280,height as u32));assert!(blur_bar_bg_buffer_radius1(1280,height as u32));
  for y in 0..BAR_BG_H.min(height){for x in [0,1,640,1279]{assert_eq!(rows[y],unsafe{BAR_BLUR_BUF[y*BAR_BG_W_CAP+x]},"height{height} y{y} x{x}");}}
  assert!(rows[BAR_BG_H.min(height)..].iter().all(|row|*row==0));
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
 #[derive(Default,Clone,Copy)]pub struct Counts{pub queries:usize,pub acks:usize,pub replies:usize,pub renders:usize,pub cursor_scrubs:usize,pub terminal:bool,pub fail_render:bool}
 static STATE:std::sync::Mutex<Counts>=std::sync::Mutex::new(Counts{queries:0,acks:0,replies:0,renders:0,cursor_scrubs:0,terminal:false,fail_render:false});
 pub fn reset(){*STATE.lock().unwrap()=Counts::default();}
 pub fn get()->Counts{*STATE.lock().unwrap()}
 pub fn terminal(){STATE.lock().unwrap().terminal=true;}
 pub fn fail_render(value:bool){STATE.lock().unwrap().fail_render=value;}
 pub fn rendering()->bool{let mut s=STATE.lock().unwrap();s.renders+=1;s.fail_render}
 pub fn cursor(){STATE.lock().unwrap().cursor_scrubs+=1;}
 pub fn info(slot:usize)->Result<app_lifecycle::LifecycleInfo,u64>{let mut s=STATE.lock().unwrap();s.queries+=1;assert_eq!(slot,40);if !s.terminal{return Err(app_lifecycle::LIVE)}Ok(app_lifecycle::LifecycleInfo{target_pd:15,task_id:150,required_mask:app_lifecycle::ROLE_DISPLAY,acked_mask:0})}
 pub fn ack(slot:usize,task:u32)->Result<(),u64>{assert_eq!((slot,task),(40,150));unsafe{assert_scrubbed(&*core::ptr::addr_of!(RENDER_SCENE));}STATE.lock().unwrap().acks+=1;Ok(())}
 pub fn reply(_target:u32,_word:u64)->u64{unsafe{assert!(!(*core::ptr::addr_of!(RENDER_SCENE)).is_valid());}STATE.lock().unwrap().replies+=1;0}
}
pub fn pdx_reply(target:u32,word:u64)->u64{boundary::reply(target,word)}
fn scrub_cursor_under(){boundary::cursor();}
// Controlled renderer callback: execute the actual scoped lease and its Drop,
// but no framebuffer/device access. Actual Main rendering is checked by gate.
fn render(_fb:*mut u32,w:usize,h:usize,bar:&SilkBar)->bool{
 let fail=boundary::rendering();let Some(lease)=SceneLease::capture(w,h,bar)else{return false;};assert!(lease.view().is_valid());if fail{return false;}true
}
fn reset_host_inputs(){
 boundary::reset();unsafe{assert!(!(*core::ptr::addr_of!(RENDER_SCENE)).is_valid());(*core::ptr::addr_of_mut!(RENDER_SCENE)).scrub();SURFACES.fill(SURFACE_EMPTY);FRAME_GROUPS=FrameGroups::new();DISPLAY_TOKENS=DEFAULT_RENDER_TOKENS;FOCUSED_SURFACE_ID=0;TOKEN_BUF_CALL1_RECEIVED=false;TOKEN_BUF_OWNER=0;CURSOR_UNDER_VALID=false;FB_W=1024;FB_H=768;}
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
#[test]fn actual_cleanup_functions_withhold_effects_and_ack_until_render_lease_drops(){
 let _single=HOST_LEASE_LOCK.lock().unwrap();reset_host_inputs();boundary::terminal();let mut bar=silkbar_model::DEFAULT_SILK_BAR;
 let mut lifecycle=surface_lifecycle::Lifecycle::new();assert!(lifecycle.register(Catalog{slot:40,target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}));
 unsafe{SURFACES[0]=Surface{owner_pd:15,surface_id:300,active:true,text_len:1,..SURFACE_EMPTY};SURFACES[0].text_buf.fill(0x73);}
 let lease=SceneLease::capture(1024,768,&bar).unwrap();assert!(!prepare_app_cleanup(&mut lifecycle,&mut bar));assert_eq!(boundary::get().queries,0);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Live));assert_eq!(lease.view().surfaces[0].text_buf[511],0x73);drop(lease);scene_scrubbed();
 assert!(prepare_app_cleanup(&mut lifecycle,&mut bar));assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Prepared));assert_eq!(boundary::get().queries,1);
 let lease=SceneLease::capture(1024,768,&bar).unwrap();finish_display_transactions(&mut lifecycle,&bar,true);assert_eq!(boundary::get().renders,0);assert_eq!(boundary::get().acks,0);drop(lease);scene_scrubbed();
 finish_display_transactions(&mut lifecycle,&bar,false);assert_eq!(boundary::get().acks,0);boundary::fail_render(true);finish_display_transactions(&mut lifecycle,&bar,true);scene_scrubbed();assert_eq!(boundary::get().renders,1);assert_eq!(boundary::get().acks,0);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Prepared));
 boundary::fail_render(false);finish_display_transactions(&mut lifecycle,&bar,true);scene_scrubbed();assert_eq!(boundary::get().renders,2);assert_eq!(boundary::get().acks,1);assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Acknowledged));finish_display_transactions(&mut lifecycle,&bar,true);assert_eq!(boundary::get().acks,1);
}
