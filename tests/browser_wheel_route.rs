// SPDX-License-Identifier: GPL-3.0-only
//! Actual Shell wheel admission/main prefix/delivery and frame scene guard.
//! Hit winner, catalog and IPC callbacks are host observations, not native
//! routing/authority/pixel evidence. The real FIFO, ABI and broker are included.
#![allow(dead_code,static_mut_refs,unused_variables)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod lifecycle_abi;
#[path="../crates/sex-pdx/src/app_scroll.rs"]pub mod app_scroll;
#[path="../crates/sex-pdx/src/app_key.rs"]pub mod app_key;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_selection.rs"]pub mod text_selection;
#[path="../crates/sex-pdx/src/clipboard.rs"]pub mod clipboard;
#[path="../servers/silk-shell/src/app_custody.rs"]mod app_custody;
#[path="../servers/silk-shell/src/app_input.rs"]mod app_input;
#[path="../servers/silk-shell/src/clipboard.rs"]mod broker;
use clipboard::{Participant,Subject};
use app_input::{Input,Item,Queue};
use app_scroll::Scroll;
pub mod app_lifecycle{
 pub use crate::lifecycle_abi::*;
 pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{unsafe{crate::CATALOG_QUERIES.push(slot);crate::CATALOG}}
}
pub mod shell_ready{pub const INPUT_PD_ID:u32=4;}
#[derive(Clone,Copy)]pub struct PdxMessage{pub caller_pd:u32,pub caller_task:u32,pub arg0:u64,pub arg1:u64,pub arg2:u64}
const OP_HID_EVENT:u64=0x202;
const EV_KEY:u64=1;const EV_WHEEL:u64=app_scroll::CLASS;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
pub const ERR_CAP_INVALID:u64=u64::MAX-2;
pub fn get_ticks()->u64{1}
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{unsafe{crate::LOG.push(format!($($arg)*));}}}}
macro_rules! serial_println{($($arg:tt)*)=>{serial_record!($($arg)*)}}
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut APP_INPUT:Queue=Queue::new();
static mut CLIPBOARD:broker::Broker=broker::Broker::new();
static mut COMMAND_PALETTE_OPEN:bool=false;static mut ATLAS_MODE_ENABLED:bool=false;
static mut LAUNCHER_ACTIVE:bool=false;static mut CLOCK_ACTIVE:bool=false;static mut STATUS_ACTIVE:bool=false;
static mut BELL_ACTIVE:bool=false;static mut SCENE_SETTINGS_ACTIVE:bool=false;
#[derive(Clone,Copy)]enum InteractionState{Idle,ClickPending,Dragging,Resizing,TabDragging,PanelActive}
static mut INTERACTION:InteractionState=InteractionState::Idle;
static mut POINTER_BUTTONS:u8=0;static mut POINTER_X:i32=100;static mut POINTER_Y:i32=120;
static mut FOCUSED_SURFACE_ID:u64=201;
#[derive(Clone,Copy)]enum HitTarget{None,Surface(u64),FrameChrome}
static mut WINNER:HitTarget=HitTarget::Surface(300);
static mut BOUNDS:Option<(i32,i32,u32,u32)>=Some((0,50,640,400));
static mut FRAME_ID:Option<u32>=Some(30);
static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[None;3];
static mut CATALOG:Result<lifecycle_abi::CatalogInfo,u64>=Err(1);
static mut CATALOG_QUERIES:Vec<usize>=Vec::new();
#[derive(Clone,Copy)]struct Tab{surface_id:u64}
#[derive(Clone,Copy)]struct Frame{frame_id:u32,scene_id:u8,flags:u32,tabs:[Option<Tab>;1],active_tab:u8}
static mut FRAMES:[Option<Frame>;1]=[None];
static mut ACTIVE_SCENE_IDX:u8=0;
const FRAME_FLAG_MINIMIZED:u32=1;
static mut ALIVE:bool=true;static mut TOMBSTONED:bool=false;
static mut LIVE:Option<bool>=Some(true);static mut LIVE_QUERIES:Vec<Subject>=Vec::new();
static mut STATUS:u64=0;static mut SENT:Vec<(u64,u64,[u64;3])>=Vec::new();static mut LOG:Vec<String>=Vec::new();
fn desktop_width()->i32{1280}fn desktop_height()->i32{800}
unsafe fn hit_test_at(_:i32,_:i32)->HitTarget{WINNER}
unsafe fn frame_for_surface(sid:u64)->Option<u32>{assert_eq!(sid,300);FRAME_ID}
unsafe fn get_surface_bounds(sid:u64)->Option<(i32,i32,u32,u32)>{assert_eq!(sid,300);BOUNDS}
unsafe fn surface_is_alive(_:u64)->bool{ALIVE}
unsafe fn is_tombstoned(_:u64)->bool{TOMBSTONED}
unsafe fn clipboard_live(subject:Subject)->Option<bool>{LIVE_QUERIES.push(subject);LIVE}
unsafe fn clipboard_poll(){}
unsafe fn pdx_call(route:u64,opcode:u64,a:u64,b:u64,c:u64)->(u64,u64){SENT.push((route,opcode,[a,b,c]));(STATUS,0)}
include!(env!("BROWSER_WHEEL_PRODUCTION"));
fn browser()->Participant{Participant::from_catalog(lifecycle_abi::CatalogInfo{target_pd:15,task_id:15,surface_id:300,coordinator_pd:3}).unwrap()}
fn wheel(vertical:i8)->Item{Item{destination:browser(),input:Input::Scroll(Scroll::new(vertical).unwrap())}}
unsafe fn reset(){
 APP_INPUT=Queue::new();CLIPBOARD=broker::Broker::new();CLIPBOARD.register(lifecycle_abi::CatalogInfo{target_pd:15,task_id:15,surface_id:300,coordinator_pd:3}).unwrap();
 COMMAND_PALETTE_OPEN=false;ATLAS_MODE_ENABLED=false;LAUNCHER_ACTIVE=false;CLOCK_ACTIVE=false;STATUS_ACTIVE=false;BELL_ACTIVE=false;SCENE_SETTINGS_ACTIVE=false;
 INTERACTION=InteractionState::Idle;POINTER_BUTTONS=0;POINTER_X=100;POINTER_Y=120;FOCUSED_SURFACE_ID=201;WINNER=HitTarget::Surface(300);BOUNDS=Some((0,50,640,400));FRAME_ID=Some(30);
 APP_CUSTODY=[Some(app_custody::Entry::new(40,15,15,300)),None,None];CATALOG=Ok(lifecycle_abi::CatalogInfo{target_pd:15,task_id:15,surface_id:300,coordinator_pd:3});CATALOG_QUERIES.clear();
 FRAMES=[Some(Frame{frame_id:30,scene_id:0,flags:0,tabs:[Some(Tab{surface_id:300})],active_tab:0})];ACTIVE_SCENE_IDX=0;ALIVE=true;TOMBSTONED=false;LIVE=Some(true);LIVE_QUERIES.clear();STATUS=0;SENT.clear();LOG.clear();
}
unsafe fn refused(){browser_wheel_input(1,0);assert!(!APP_INPUT.pending());assert!(SENT.is_empty());assert_eq!(FOCUSED_SURFACE_ID,201);assert!(LOG.iter().any(|line|line.contains("[shell.browser.wheel.rejected]")));}
#[test]fn actual_main_authentication_precedes_scroll_decode_and_admission(){
 let _lock=LOCK.lock().unwrap();unsafe{for(pd,task)in[(0,0),(4,0),(0,4),(3,3),(4,3),(3,4),(u32::MAX,4),(4,u32::MAX)]{
  reset();actual_main_wheel_prefix(&PdxMessage{caller_pd:pd,caller_task:task,arg0:1,arg1:0,arg2:EV_WHEEL});
  assert!(SENT.is_empty());assert!(!APP_INPUT.pending());assert!(CATALOG_QUERIES.is_empty());assert!(LOG.is_empty());
 }
 reset();actual_main_wheel_prefix(&PdxMessage{caller_pd:4,caller_task:4,arg0:1,arg1:0,arg2:EV_WHEEL});
 assert_eq!(SENT,[(43,OP_HID_EVENT,[1,0,5])]);assert_eq!(FOCUSED_SURFACE_ID,201);
 for class in[0,1,2,3,4,6,u64::MAX]{reset();actual_main_wheel_prefix(&PdxMessage{caller_pd:4,caller_task:4,arg0:1,arg1:0,arg2:class});assert!(SENT.is_empty());}
 }}
#[test]fn all_canonical_wheels_send_exact_signed_packets_without_focus_mutation(){
 let _lock=LOCK.lock().unwrap();unsafe{for vertical in i8::MIN..=i8::MAX{
  if vertical==0{continue}reset();let scroll=Scroll::new(vertical).unwrap();
  actual_main_wheel_prefix(&PdxMessage{caller_pd:4,caller_task:4,arg0:scroll.wire(),arg1:0,arg2:5});
  assert_eq!(SENT,[(43,OP_HID_EVENT,[scroll.wire(),0,5])]);assert_eq!(FOCUSED_SURFACE_ID,201);assert!(!APP_INPUT.pending());
  assert_eq!(LIVE_QUERIES,[browser().subject]);assert_eq!(CATALOG_QUERIES,[40]);
  assert!(LOG[0].contains("pd=15 task=15 route=43"));assert!(LOG[0].contains("x=100 y=120 accepted=1"));assert!(LOG[1].contains("accepted=1 status=0"));
 }}}
#[test]fn malformed_zero_aliases_reserved_payloads_never_replace_a_queued_wheel(){
 let _lock=LOCK.lock().unwrap();unsafe{
 for word in[0,128,255,256,65535,u32::MAX as u64,(1<<32)|1]{reset();APP_INPUT.push(wheel(-1));browser_wheel_input(word,0);assert_eq!(APP_INPUT.front(),Some(wheel(-1)));assert!(SENT.is_empty());assert!(CATALOG_QUERIES.is_empty());}
 for reserved in[1,2,u64::MAX]{reset();APP_INPUT.push(wheel(-1));browser_wheel_input(1,reserved);assert_eq!(APP_INPUT.front(),Some(wheel(-1)));assert!(SENT.is_empty());assert!(CATALOG_QUERIES.is_empty());}
 for vertical in[1,-1,127,-128]{let word=Scroll::new(vertical).unwrap().wire();for bit in 8..64{reset();browser_wheel_input(word^(1<<bit),0);assert!(!APP_INPUT.pending());assert!(SENT.is_empty());}}
 }}
#[test]fn launcher_all_panels_atlas_drag_and_button_capture_refuse_without_effects(){
 let _lock=LOCK.lock().unwrap();unsafe{for kind in 0..7{
  reset();match kind{0=>COMMAND_PALETTE_OPEN=true,1=>ATLAS_MODE_ENABLED=true,2=>LAUNCHER_ACTIVE=true,3=>CLOCK_ACTIVE=true,4=>STATUS_ACTIVE=true,5=>BELL_ACTIVE=true,_=>SCENE_SETTINGS_ACTIVE=true}refused();
 }
 for interaction in[InteractionState::ClickPending,InteractionState::Dragging,InteractionState::Resizing,InteractionState::TabDragging,InteractionState::PanelActive]{reset();INTERACTION=interaction;refused();}
 for buttons in[1,2,4,7,255]{reset();POINTER_BUTTONS=buttons;refused();}
 }}
#[test]fn actual_scene_minimize_lifecycle_chrome_overlap_and_clipping_guards_refuse(){
 let _lock=LOCK.lock().unwrap();unsafe{
 for winner in[HitTarget::None,HitTarget::FrameChrome,HitTarget::Surface(201),HitTarget::Surface(152)]{reset();WINNER=winner;refused();}
 for point in[(-1,120),(1280,120),(100,-1),(100,800),(7,120),(632,120),(100,85),(100,442)]{reset();POINTER_X=point.0;POINTER_Y=point.1;refused();}
 for point in[(8,86),(631,441)]{reset();POINTER_X=point.0;POINTER_Y=point.1;assert_eq!(browser_wheel_destination(),Some(browser()));}
 reset();FRAMES[0].as_mut().unwrap().scene_id=1;refused();
 reset();FRAMES[0].as_mut().unwrap().flags=FRAME_FLAG_MINIMIZED;refused();
 reset();ALIVE=false;refused();reset();TOMBSTONED=true;refused();
 reset();FRAME_ID=None;refused();reset();FRAMES=[None];refused();reset();BOUNDS=None;refused();
 for bounds in[(-200,-50,640,400),(0,50,0,400),(0,50,640,0)]{reset();BOUNDS=Some(bounds);POINTER_X=if bounds.0<0{-1}else{100};refused();}
 }}
#[test]fn catalog_and_cached_incarnation_must_both_attest_the_same_live_browser(){
 let _lock=LOCK.lock().unwrap();unsafe{
 for state in 0..3{reset();let entry=APP_CUSTODY[0].as_mut().unwrap();match state{0=>entry.dead=true,1=>entry.retired=true,_=>entry.close_pending=true}refused();}
 reset();APP_CUSTODY=[None;3];refused();
 for field in 0..4{reset();if let Ok(c)=CATALOG.as_mut(){match field{0=>c.target_pd=9,1=>c.task_id=99,2=>c.surface_id=301,_=>c.coordinator_pd=15}}refused();}
 for error in[1,2,u64::MAX]{reset();CATALOG=Err(error);refused();}
 }}
#[test]fn fresh_live_retry_and_full_preserve_original_target_payload_and_fifo_across_focus_changes(){
 let _lock=LOCK.lock().unwrap();unsafe{
 reset();LIVE=None;browser_wheel_input(Scroll::new(-3).unwrap().wire(),0);assert_eq!(APP_INPUT.front(),Some(wheel(-3)));assert!(SENT.is_empty());
 FOCUSED_SURFACE_ID=307;WINNER=HitTarget::Surface(307);POINTER_X=700;POINTER_Y=300;
 for _ in 0..4{app_input_poll();assert_eq!(APP_INPUT.front(),Some(wheel(-3)));assert!(SENT.is_empty());}
 LIVE=Some(true);STATUS=ERR_SERVICE_NOT_READY;
 for _ in 0..4{app_input_poll();assert_eq!(APP_INPUT.front(),Some(wheel(-3)));assert_eq!(SENT.last().unwrap(),&(43,OP_HID_EVENT,[(-3i64)as u64,0,5]));}
 assert!(APP_INPUT.push(Item{destination:Participant::from_catalog(lifecycle_abi::CatalogInfo{target_pd:18,task_id:18,surface_id:307,coordinator_pd:3}).unwrap(),input:Input::Key(app_key::Key{code:0x1e,shift:true,control:false})}));
 STATUS=0;app_input_poll();assert!(matches!(APP_INPUT.front().unwrap().input,Input::Key(_)));app_input_poll();assert!(!APP_INPUT.pending());
 assert_eq!(SENT[4],(43,OP_HID_EVENT,[(-3i64)as u64,0,5]));assert_eq!(SENT[5].0,56);assert_eq!(FOCUSED_SURFACE_ID,307);
 }}
#[test]fn terminal_live_loss_hard_error_or_exact_retirement_never_redirect_a_queued_wheel(){
 let _lock=LOCK.lock().unwrap();unsafe{
 reset();APP_INPUT.push(wheel(1));LIVE=Some(false);app_input_poll();assert!(!APP_INPUT.pending());assert!(SENT.is_empty());
 reset();APP_INPUT.push(wheel(-1));STATUS=77;app_input_poll();assert!(!APP_INPUT.pending());assert_eq!(SENT,[(43,OP_HID_EVENT,[u64::MAX,0,5])]);assert!(LOG.last().unwrap().contains("accepted=0 status=77"));
 reset();APP_INPUT.push(wheel(1));APP_INPUT.push(wheel(-1));assert_eq!(APP_INPUT.retire(Subject{pd:15,task:99}),0);assert_eq!(APP_INPUT.front(),Some(wheel(1)));assert_eq!(APP_INPUT.retire(browser().subject),2);app_input_poll();assert!(SENT.is_empty());
 }}
#[test]fn queue_exhaustion_preserves_all_events_and_waits_only_for_prior_intent_admission(){
 let _lock=LOCK.lock().unwrap();unsafe{
 reset();LIVE=None;for _ in 0..app_input::CAPACITY{assert!(APP_INPUT.push(wheel(1)));}browser_wheel_input(Scroll::new(-1).unwrap().wire(),0);assert!(LOG[0].contains("accepted=0 capacity=32"));
 for _ in 0..app_input::CAPACITY{assert_eq!(APP_INPUT.front(),Some(wheel(1)));APP_INPUT.pop();}assert!(!APP_INPUT.pending());
 reset();CLIPBOARD.issue(browser().subject,clipboard::Action::Copy,true,1).unwrap();APP_INPUT.push(wheel(-1));app_input_poll();assert!(SENT.is_empty());assert_eq!(APP_INPUT.front(),Some(wheel(-1)));
 CLIPBOARD.poll(1,4,|_,_|broker::Admission::Accepted);assert!(CLIPBOARD.busy());assert!(!CLIPBOARD.outbox_pending());app_input_poll();assert!(!APP_INPUT.pending());assert_eq!(SENT.len(),1);
 }}
