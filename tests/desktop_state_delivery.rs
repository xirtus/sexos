// SPDX-License-Identifier: GPL-3.0-only
//! Exact production wrapper, lifecycle validator and poll bodies; PDX and shell
//! model queries are mocked. No guest rendering is claimed by this harness.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] pub mod app_catalog;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../crates/sex-pdx/src/spindle_view.rs"]pub mod spindle_view;
#[path="../servers/silk-shell/src/desktop_content.rs"]mod desktop_content;
pub struct PdxMessage{pub type_id:u64,pub caller_pd:u32,pub arg0:u64}
#[path="../servers/silk-shell/src/linen_text_delivery.rs"] mod linen_text_delivery;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
use std::sync::Mutex;
use std::collections::VecDeque;
static LOCK:Mutex<()>=Mutex::new(());
static CALLS:Mutex<Vec<(u64,u64,u64,u64,u64)>>=Mutex::new(Vec::new());
static RESULTS:Mutex<VecDeque<(u64,u64)>>=Mutex::new(VecDeque::new());
static LOG:Mutex<Vec<String>>=Mutex::new(Vec::new());
const SLOT_SPINDLE:u64=14;fn spindle_route_slot()->u64{52}
const SLOT_QUIL:u64=10;fn quil_route_slot()->u64{47}
const SLOT_DISPLAY:u64=5;const SURFACE_ID_LINEN:u64=200;
#[macro_export]macro_rules! record {($($a:tt)*)=>{{$crate::LOG.lock().unwrap().push(format!($($a)*));}}}
pub use crate::record as serial_record;pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
static mut FOCUSED_SURFACE_ID:u64=0;
static mut LAST_DISPLAY_FOCUS_RESULT:Option<(u64,u64)>=None;
// This wrapper harness has no pending launcher request. Its retry predicate is
// true in that state; actual pending-launcher guards run in launcher_workspace.
unsafe fn launcher_focus_retry_ready(_:u64)->bool{true}
static mut DISPLAY_FOCUS:linen_text_delivery::FocusDelivery=linen_text_delivery::FocusDelivery::new();
static mut DESKTOP_STATES:linen_text_delivery::DesktopStates=linen_text_delivery::DesktopStates::new();
static mut LINEN_TEXT:linen_text_delivery::Outbox=linen_text_delivery::Outbox::new(200);
static mut PALETTE_TEXT:linen_text_delivery::Outbox=linen_text_delivery::Outbox::atomic(152);
static mut LINEN_REPAINT_AFTER_CREATE:bool=false;
static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[None;3];
#[derive(Clone,Copy)]enum LifecycleState{Visible,Hidden,Minimized,Closing,Tombstoned,Destroyed}
static mut STATE:LifecycleState=LifecycleState::Visible;
static mut ALIVE:bool=true;static mut MINIMIZED:bool=false;static mut FRAME:bool=true;
static mut BOUNDS:Option<(i32,i32,u32,u32)>=Some((10,50,640,400));
static mut SELECTION:u64=3;
fn surface_is_alive(_:u64)->bool{unsafe{ALIVE}}
unsafe fn surface_is_lifecycle_focusable(_:u64)->bool{matches!(STATE,LifecycleState::Visible)}
unsafe fn is_tombstoned(_:u64)->bool{matches!(STATE,LifecycleState::Tombstoned|LifecycleState::Destroyed)}
unsafe fn lifecycle_state(_:u64)->Option<LifecycleState>{Some(STATE)}
unsafe fn frame_for_surface(sid:u64)->Option<u32>{if FRAME{Some(sid as u32)}else{None}}
unsafe fn frame_is_minimized(_:u32)->bool{MINIMIZED}
unsafe fn get_surface_bounds(_:u64)->Option<(i32,i32,u32,u32)>{BOUNDS}
unsafe fn frame_chrome_operation(frame:u32)->Option<linen_text_delivery::Operation>{Some(linen_text_delivery::Operation{opcode:0xFD,surface:frame as u64,packed:1,control:0x2300})}
fn raw_pdx_call(slot:u64,op:u64,sid:u64,a:u64,b:u64)->(u64,u64){CALLS.lock().unwrap().push((slot,op,sid,a,b));RESULTS.lock().unwrap().pop_front().unwrap_or((0,0))}
unsafe fn linen_paint_surface_fast(){linen_queue_text(b"latest complete view",0xabcdef,linen_text_delivery::Metadata{count:2,first:0,rows:2,selected:SELECTION});}
include!(env!("DESKTOP_DELIVERY_PRODUCTION"));
unsafe fn reset(){FOCUSED_SURFACE_ID=0;LAST_DISPLAY_FOCUS_RESULT=None;DISPLAY_FOCUS=linen_text_delivery::FocusDelivery::new();DESKTOP_STATES=linen_text_delivery::DesktopStates::new();LINEN_TEXT=linen_text_delivery::Outbox::new(200);LINEN_REPAINT_AFTER_CREATE=false;PALETTE_TEXT=linen_text_delivery::Outbox::atomic(152);APP_CUSTODY=[None;3];STATE=LifecycleState::Visible;ALIVE=true;MINIMIZED=false;FRAME=true;BOUNDS=Some((10,50,640,400));SELECTION=3;CALLS.lock().unwrap().clear();RESULTS.lock().unwrap().clear();LOG.lock().unwrap().clear();}
fn op(code:u64,sid:u64)->linen_text_delivery::Operation{linen_text_delivery::Operation{opcode:code,surface:sid,packed:0,control:0}}
#[test]fn actual_wrapper_preserves_status_and_retains_only_selected_display_states(){let _l=LOCK.lock().unwrap();unsafe{reset();}
 RESULTS.lock().unwrap().push_back((sex_pdx::ERR_SERVICE_NOT_READY,77));assert_eq!(pdx_call(5,0xEC,201,1,2),(sex_pdx::ERR_SERVICE_NOT_READY,77));assert!(unsafe{DESKTOP_STATES.pending_surface(201)});
 RESULTS.lock().unwrap().push_back((sex_pdx::ERR_SERVICE_NOT_READY,88));assert_eq!(pdx_call(5,0xEC,999,1,2).1,88);assert!(!unsafe{DESKTOP_STATES.pending_surface(999)});
 RESULTS.lock().unwrap().push_back((123,456));assert_eq!(pdx_call(7,0xEC,300,1,2),(123,456));assert!(!unsafe{DESKTOP_STATES.pending_surface(300)});
}
#[test]fn actual_validator_fences_dead_close_minimize_and_refreshes_current_geometry(){let _l=LOCK.lock().unwrap();unsafe{reset();APP_CUSTODY[0]=Some(app_custody::Entry::new(38,9,9,156));
 let create=op(0xEC,201);assert_eq!(desktop_state_validate(create).unwrap().packed,(50<<32)|10);
 APP_CUSTODY[0].as_mut().unwrap().close_pending=true;assert!(desktop_state_validate(create).is_none());assert!(desktop_state_validate(op(0xEE,201)).is_some());
 APP_CUSTODY[0].as_mut().unwrap().close_pending=false;APP_CUSTODY[0].as_mut().unwrap().dead=true;assert!(desktop_state_validate(create).is_none());APP_CUSTODY=[None;3];
 STATE=LifecycleState::Minimized;assert!(desktop_state_validate(create).is_none());STATE=LifecycleState::Visible;MINIMIZED=true;assert!(desktop_state_validate(create).is_none());MINIMIZED=false;BOUNDS=Some((-20,90,800,600));assert_eq!(desktop_state_validate(create).unwrap().packed,(90<<32)|(-20i32 as u32 as u64));}}
#[test]fn actual_poll_admits_create_then_cached_boot_chrome_when_frame_metadata_is_late(){let _l=LOCK.lock().unwrap();unsafe{reset();FRAME=false;}
 RESULTS.lock().unwrap().push_back((sex_pdx::ERR_SERVICE_NOT_READY,0));pdx_call(5,0xEC,201,1,2);pdx_call(5,0xFD,201,1,0x2300);
 unsafe{desktop_state_poll();assert!(DESKTOP_STATES.pending_surface(201));desktop_state_poll();assert!(!DESKTOP_STATES.pending());}
 let c=CALLS.lock().unwrap();assert_eq!(c[c.len()-2].1,0xEC);assert_eq!(c.last().unwrap().1,0xFD);assert_eq!(c.last().unwrap().4,0x2300);
}
#[test]fn actual_text_poll_waits_for_chrome_and_suspends_hidden_without_false_receipt(){let _l=LOCK.lock().unwrap();unsafe{reset();}
 RESULTS.lock().unwrap().push_back((sex_pdx::ERR_SERVICE_NOT_READY,0));pdx_call(5,0xEC,200,1,2);
 unsafe{linen_queue_text(b"old",1,linen_text_delivery::Metadata{count:1,first:0,rows:1,selected:1});linen_text_poll();assert_eq!(CALLS.lock().unwrap().len(),1);
 desktop_state_poll();linen_text_poll();assert_eq!(CALLS.lock().unwrap().len(),2);desktop_state_poll();STATE=LifecycleState::Minimized;
 let before=CALLS.lock().unwrap().len();linen_text_poll();assert_eq!(CALLS.lock().unwrap().len(),before);assert!(!linen_text_can_progress());assert!(!LOG.lock().unwrap().iter().any(|s|s.contains("linen.text.present")));
 STATE=LifecycleState::Visible;SELECTION=9;linen_text_poll();assert!(LOG.lock().unwrap().iter().any(|s|s.contains("linen.text.present")&&s.contains("selected=9")&&s.contains("admitted=1")));}}
#[test]fn actual_text_rejection_has_no_success_receipt_and_next_repaint_recovers(){let _l=LOCK.lock().unwrap();unsafe{reset();linen_queue_text(b"old",1,linen_text_delivery::Metadata{count:1,first:0,rows:1,selected:1});}
 RESULTS.lock().unwrap().push_back((99,0));unsafe{linen_text_poll();assert!(!LINEN_TEXT.pending());linen_paint_surface_fast();linen_text_poll();}
 let logs=LOG.lock().unwrap();assert_eq!(logs.iter().filter(|s|s.contains("linen.text.rejected")).count(),1);assert_eq!(logs.iter().filter(|s|s.contains("linen.text.present")).count(),1);
}

#[test]fn actual_focus_wrapper_preserves_full_and_retries_after_create_and_chrome(){
 let _l=LOCK.lock().unwrap();unsafe{reset();FOCUSED_SURFACE_ID=153;}
 RESULTS.lock().unwrap().extend([(sex_pdx::ERR_SERVICE_NOT_READY,7),(sex_pdx::ERR_SERVICE_NOT_READY,8)]);
 assert_eq!(pdx_call(5,0xEC,153,1,2),(sex_pdx::ERR_SERVICE_NOT_READY,7));
 assert_eq!(pdx_call(5,0xED,153,0,0),(sex_pdx::ERR_SERVICE_NOT_READY,8));
 unsafe{desktop_state_poll();}assert_eq!(CALLS.lock().unwrap().last().unwrap().1,0xEC);
 unsafe{desktop_state_poll();}let calls=CALLS.lock().unwrap();assert_eq!(calls[calls.len()-2].1,0xFD);assert_eq!(calls.last().unwrap().1,0xED);
 assert!(LOG.lock().unwrap().iter().any(|r|r.contains("shell.desktop.focus.admitted")&&r.contains("sid=153")));
}
#[test]fn actual_focus_poll_discards_stale_and_terminal_targets_without_replay(){
 let _l=LOCK.lock().unwrap();for stale in [false,true]{unsafe{reset();FOCUSED_SURFACE_ID=153;}
 RESULTS.lock().unwrap().push_back((sex_pdx::ERR_SERVICE_NOT_READY,0));pdx_call(5,0xED,153,0,0);
 unsafe{if stale{FOCUSED_SURFACE_ID=306;}else{STATE=LifecycleState::Closing;}desktop_state_poll();}
 assert_eq!(CALLS.lock().unwrap().len(),1);assert!(LOG.lock().unwrap().is_empty());
 }
}
#[test]fn actual_atomic_poll_and_reply_publish_only_current_correlated_metadata(){let _l=LOCK.lock().unwrap();unsafe{
 reset();LINEN_TEXT=linen_text_delivery::Outbox::atomic(200);
 linen_queue_text(b"current",1,linen_text_delivery::Metadata{count:1,first:0,rows:1,selected:9});linen_text_poll();
 assert!(!LOG.lock().unwrap().iter().any(|s|s.contains("linen.text.present")));
 let value=text_batch::Applied{surface:200,sequence:1}.encode().unwrap();
 for msg in [PdxMessage{type_id:1,caller_pd:11,arg0:value},PdxMessage{type_id:2,caller_pd:1,arg0:value},PdxMessage{type_id:1,caller_pd:1,arg0:value|1<<56}]{assert!(!desktop_text_receive(&msg));}
 assert!(desktop_text_receive(&PdxMessage{type_id:1,caller_pd:1,arg0:value}));
 assert!(LOG.lock().unwrap().iter().any(|s|s.contains("selected=9")&&s.contains("applied=1 pixels=unconfirmed")));
 assert!(!desktop_text_receive(&PdxMessage{type_id:1,caller_pd:1,arg0:value}));
}}
#[test]fn palette_receipt_is_surface_correlated_and_closed_payload_cannot_escape(){let _l=LOCK.lock().unwrap();unsafe{
 reset();PALETTE_TEXT.queue(b"Open Quil",0xf0f8ff,linen_text_delivery::Metadata{count:10,first:0,rows:5,selected:1});
 for _ in 0..2{PALETTE_TEXT.poll(4,|_|linen_text_delivery::Admission::Accepted);}
 let value=text_batch::Applied{surface:152,sequence:1}.encode().unwrap();
 assert!(!desktop_text_receive(&PdxMessage{type_id:1,caller_pd:1,arg0:text_batch::Applied{surface:200,sequence:1}.encode().unwrap()}));
 assert!(desktop_text_receive(&PdxMessage{type_id:1,caller_pd:1,arg0:value}));
 assert!(LOG.lock().unwrap().iter().any(|s|s.contains("launcher.text.admitted")&&s.contains("selected=1")&&s.contains("applied=1")));
 PALETTE_TEXT.queue(b"must scrub",0,linen_text_delivery::Metadata{count:10,first:0,rows:5,selected:2});
 for _ in 0..2{PALETTE_TEXT.poll(4,|_|linen_text_delivery::Admission::Accepted);}PALETTE_TEXT.cancel();
 assert!(!desktop_text_receive(&PdxMessage{type_id:1,caller_pd:1,arg0:text_batch::Applied{surface:152,sequence:2}.encode().unwrap()}));
}}

#[test]fn retained_child_create_uses_exact_live_parent_latest_geometry_before_viewport_ready(){let _l=LOCK.lock().unwrap();unsafe{
 reset();APP_CUSTODY[0]=Some(app_custody::Entry::new(38,9,90,156));
 RESULTS.lock().unwrap().push_back((ERR_SERVICE_NOT_READY,0));pdx_call(SLOT_DISPLAY,0xEC,156,0,0);
 assert!(!desktop_content_ready(201));BOUNDS=Some((20,60,800,600));
 let op=desktop_state_validate(op(0xEC,156)).unwrap();assert_eq!((op.packed,op.control),((96<<32)|28,(556<<32)|784));
 desktop_state_poll();desktop_state_poll();assert!(desktop_content_ready(201));
 APP_CUSTODY[0].as_mut().unwrap().close_pending=true;assert!(desktop_state_validate(op).is_none());
 APP_CUSTODY[0].as_mut().unwrap().close_pending=false;STATE=LifecycleState::Minimized;assert!(desktop_state_validate(op).is_none());assert!(!desktop_content_ready(201));
 reset();assert!(desktop_state_validate(op).is_none());
}}
#[test]fn viewport_waits_for_each_spindle_child_and_hard_failure_cannot_be_readiness(){let _l=LOCK.lock().unwrap();unsafe{
 reset();APP_CUSTODY[0]=Some(app_custody::Entry::new(50,17,170,302));
 for sid in [302,303,304,305]{DESKTOP_STATES.observe(op(0xEC,sid),linen_text_delivery::Admission::Full);}
 assert!(!desktop_content_ready(306));desktop_state_poll();assert!(!desktop_content_ready(306));desktop_state_poll();desktop_state_poll();assert!(desktop_content_ready(306));
 DESKTOP_STATES.observe(op(0xEC,303),linen_text_delivery::Admission::Rejected(3));assert!(!desktop_content_ready(306));
 DESKTOP_STATES.observe(op(0xFD,303),linen_text_delivery::Admission::Accepted);assert!(!desktop_content_ready(306));
 DESKTOP_STATES.observe(op(0xEC,303),linen_text_delivery::Admission::Accepted);desktop_state_poll();assert!(desktop_content_ready(306));
}}
