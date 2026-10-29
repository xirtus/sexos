// SPDX-License-Identifier: GPL-3.0-only
//! Exact production wrapper, lifecycle validator and poll bodies; PDX and shell
//! model queries are mocked. No guest rendering is claimed by this harness.
#![allow(dead_code,static_mut_refs)]
#[path="../servers/silk-shell/src/linen_text_delivery.rs"] mod linen_text_delivery;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
use std::sync::Mutex;
use std::collections::VecDeque;
static LOCK:Mutex<()>=Mutex::new(());
static CALLS:Mutex<Vec<(u64,u64,u64,u64,u64)>>=Mutex::new(Vec::new());
static RESULTS:Mutex<VecDeque<(u64,u64)>>=Mutex::new(VecDeque::new());
static LOG:Mutex<Vec<String>>=Mutex::new(Vec::new());
const SLOT_QUIL:u64=10;fn quil_route_slot()->u64{47}
const SLOT_DISPLAY:u64=5;const SURFACE_ID_LINEN:u64=200;
#[macro_export]macro_rules! record {($($a:tt)*)=>{$crate::LOG.lock().unwrap().push(format!($($a)*));}}
mod sex_pdx{pub use crate::record as serial_record;pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;}
static mut DESKTOP_STATES:linen_text_delivery::DesktopStates=linen_text_delivery::DesktopStates::new();
static mut LINEN_TEXT:linen_text_delivery::Outbox=linen_text_delivery::Outbox::new(200);
static mut LINEN_REPAINT_AFTER_CREATE:bool=false;
static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[None;3];
#[derive(Clone,Copy)]enum LifecycleState{Visible,Hidden,Minimized,Closing,Tombstoned,Destroyed}
static mut STATE:LifecycleState=LifecycleState::Visible;
static mut ALIVE:bool=true;static mut MINIMIZED:bool=false;static mut FRAME:bool=true;
static mut BOUNDS:Option<(i32,i32,u32,u32)>=Some((10,50,640,400));
static mut SELECTION:u64=3;
fn surface_is_alive(_:u64)->bool{unsafe{ALIVE}}
unsafe fn lifecycle_state(_:u64)->Option<LifecycleState>{Some(STATE)}
unsafe fn frame_for_surface(sid:u64)->Option<u32>{if FRAME{Some(sid as u32)}else{None}}
unsafe fn frame_is_minimized(_:u32)->bool{MINIMIZED}
unsafe fn get_surface_bounds(_:u64)->Option<(i32,i32,u32,u32)>{BOUNDS}
unsafe fn frame_chrome_operation(frame:u32)->Option<linen_text_delivery::Operation>{Some(linen_text_delivery::Operation{opcode:0xFD,surface:frame as u64,packed:1,control:0x2300})}
fn raw_pdx_call(slot:u64,op:u64,sid:u64,a:u64,b:u64)->(u64,u64){CALLS.lock().unwrap().push((slot,op,sid,a,b));RESULTS.lock().unwrap().pop_front().unwrap_or((0,0))}
unsafe fn linen_paint_surface_fast(){linen_queue_text(b"latest complete view",0xabcdef,linen_text_delivery::Metadata{count:2,first:0,rows:2,selected:SELECTION});}
include!(env!("DESKTOP_DELIVERY_PRODUCTION"));
unsafe fn reset(){DESKTOP_STATES=linen_text_delivery::DesktopStates::new();LINEN_TEXT=linen_text_delivery::Outbox::new(200);LINEN_REPAINT_AFTER_CREATE=false;APP_CUSTODY=[None;3];STATE=LifecycleState::Visible;ALIVE=true;MINIMIZED=false;FRAME=true;BOUNDS=Some((10,50,640,400));SELECTION=3;CALLS.lock().unwrap().clear();RESULTS.lock().unwrap().clear();LOG.lock().unwrap().clear();}
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
