// SPDX-License-Identifier: GPL-3.0-only
//! Actual extracted Shell routing/launcher functions with scripted held-query
//! and desktop-effect callbacks. Future rows and unequal tasks are host data;
//! this does not claim kernel readiness, activation, grants, or native pixels.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=2;pub const EV_KEY:u64=1;pub const EV_WHEEL:u64=5;
#[cfg(not(browser_launch_catalog))]
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[cfg(browser_launch_catalog)]pub mod app_catalog{include!(env!("BROWSER_LAUNCH_CATALOG"));}
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod lifecycle_types;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_selection.rs"]pub mod text_selection;
#[path="../crates/sex-pdx/src/clipboard.rs"]pub mod clipboard;
#[path="../crates/sex-pdx/src/app_key.rs"]pub mod app_key;
#[path="../crates/sex-pdx/src/app_scroll.rs"]pub mod app_scroll;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/document_view.rs"]pub mod document_view;
#[path="../servers/silk-shell/src/app_input.rs"]mod app_input;
#[path="../servers/silk-shell/src/app_custody.rs"]mod app_custody;
#[path="../servers/silk-shell/src/launcher.rs"]mod launcher;
/// Voyeur frecency double: the launcher records a run; ranking is tests/voyeur_search.rs.
mod voyeur{pub unsafe fn launched(_:usize){}}
pub mod app_lifecycle{
 pub use crate::lifecycle_types::{CatalogInfo,LifecycleInfo,ActivationInfo,DENIED,LIVE,RETRY};
 pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{unsafe{crate::HELD.iter().copied().find(|c|c.as_ref().is_ok_and(|c|crate::slot(c.target_pd)==slot)).unwrap_or(Err(DENIED))}}
 pub fn info(slot:usize)->Result<LifecycleInfo,u64>{let index=match slot{40=>0,154=>1,158=>2,_=>return Err(DENIED)};unsafe{crate::LIVE[index]}}
 pub fn activation(slot:usize,commit:bool)->Result<ActivationInfo,u64>{unsafe{
  crate::CALLS.push((slot,commit));let index=match slot{156=>0,160=>1,_=>return Err(DENIED)};
  if commit{let result=crate::COMMIT[index];if result.is_ok(){crate::ACTIVATION[index]=result;}result}else{crate::ACTIVATION[index]}
 }}
}
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{{unsafe{crate::LOGS.push(format!($($arg)*));}}};}
#[macro_export]macro_rules! serial_println{($($arg:tt)*)=>{{unsafe{crate::LOGS.push(format!($($arg)*));}}};}
use app_lifecycle as a;
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[None;3];
static mut HELD:[Result<a::CatalogInfo,u64>;3]=[Err(3);3];
static mut LIVE:[Result<a::LifecycleInfo,u64>;3]=[Err(a::LIVE);3];
static mut ACTIVATION:[Result<a::ActivationInfo,u64>;2]=[Err(3);2];
static mut COMMIT:[Result<a::ActivationInfo,u64>;2]=[Err(2);2];
static mut CALLS:Vec<(usize,bool)>=Vec::new();static mut LOGS:Vec<String>=Vec::new();
#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum Effect{Focus(u64),Reparent(u32,u8),Restore(u32),Queued(u32,u32,u64),Activated(u32,u64)}
static mut EFFECTS:Vec<Effect>=Vec::new();static mut FRAMES:[Option<ShellFrame>;3]=[None;3];
static mut ACTIVE_SCENE_IDX:u8=0;static mut FOCUSED_SURFACE_ID:u64=300;
static mut VISIBLE:[bool;3]=[true;3];static mut CONTENT_READY:bool=true;
static mut FOCUS_STATUS:u64=0;static mut LAST_DISPLAY_FOCUS_RESULT:Option<(u64,u64)>=None;
static mut COMMAND_PALETTE_SELECTED:u8=10;static mut ATLAS_MODE_ENABLED:bool=false;
const SLOT_DISPLAY:u64=4;const SURFACE_ID_QUIL:u64=201;const SURFACE_ID_SPINDLE:u64=153;
const SURFACE_ID_LINEN:u64=200;const SURFACE_ID_ATLAS_OVERLAY:u64=151;
const SURFACE_ID_BELL_PLACEHOLDER:u64=204;const SURFACE_ID_COLLAR:u64=203;const SURFACE_ID_MESH:u64=202;
#[derive(Clone,Copy)]enum SurfaceAction{RestoreMinimized,AccessZoomToggle,AccessActivate}
// The included definitions/functions are extracted verbatim from the Main
// draft/canonical source, not a second implementation of launcher policy.
include!(env!("BROWSER_LAUNCH_PRODUCTION"));
fn slot(target:u32)->usize{match target{15=>40,30=>154,31=>158,_=>usize::MAX}}
fn held(target:u32,task:u32,surface:u64)->a::CatalogInfo{a::CatalogInfo{target_pd:target,task_id:task,surface_id:surface,coordinator_pd:3}}
fn receipt(target:u32,task:u32,surface:u64,state:u64)->Result<a::ActivationInfo,u64>{Ok(a::ActivationInfo{successor:target,task,surface,state})}
unsafe fn reset(){
 APP_CUSTODY=[Some(app_custody::Entry::new(40,15,150,300)),Some(app_custody::Entry::new(154,30,300,343)),Some(app_custody::Entry::new(158,31,310,344))];
 HELD=[Ok(held(15,150,300)),Ok(held(30,300,343)),Ok(held(31,310,344))];LIVE=[Err(a::LIVE);3];
 ACTIVATION=[receipt(30,300,343,0),receipt(31,310,344,0)];COMMIT=[Err(a::RETRY);2];
 CALLS.clear();LOGS.clear();EFFECTS.clear();FRAMES=[None;3];VISIBLE=[true;3];CONTENT_READY=true;FOCUS_STATUS=0;LAST_DISPLAY_FOCUS_RESULT=None;
 ACTIVE_SCENE_IDX=0;FOCUSED_SURFACE_ID=300;COMMAND_PALETTE_SELECTED=10;ATLAS_MODE_ENABLED=false;
 install(0,300);
}
unsafe fn install(index:usize,sid:u64){let mut tabs=[None;MAX_TABS_PER_FRAME as usize];tabs[0]=Some(ShellTab{surface_id:sid,title_id:300,flags:0});
 FRAMES[index]=Some(ShellFrame{frame_id:10+index as u32,active_tab:0,tab_count:1,tabs,scene_id:0,flags:0,normal_x:0,normal_y:50,normal_w:512,normal_h:359});
}
unsafe fn dead(index:usize,acked:u64){let e=APP_CUSTODY[index].as_mut().unwrap();e.dead=true;e.retired=true;e.acked=acked&1!=0;e.close_pending=false;
 LIVE[index]=Ok(a::LifecycleInfo{target_pd:e.target,task_id:e.task,required_mask:31,acked_mask:acked});
}
unsafe fn frame_for_surface(sid:u64)->Option<u32>{FRAMES.iter().flatten().find(|f|f.tabs.iter().flatten().any(|t|t.surface_id==sid)).map(|f|f.frame_id)}
unsafe fn frame_is_minimized(fid:u32)->bool{FRAMES.iter().flatten().find(|f|f.frame_id==fid).is_some_and(|f|f.flags&FRAME_FLAG_MINIMIZED!=0)}
unsafe fn lifecycle_state(sid:u64)->Option<LifecycleState>{let i=FRAMES.iter().position(|f|f.is_some_and(|f|f.tabs.iter().flatten().any(|t|t.surface_id==sid)))?;Some(if VISIBLE[i]{LifecycleState::Visible}else{LifecycleState::Hidden})}
unsafe fn set_lifecycle_state(sid:u64,state:LifecycleState)->bool{let Some(i)=FRAMES.iter().position(|f|f.is_some_and(|f|f.tabs.iter().flatten().any(|t|t.surface_id==sid)))else{return false};VISIBLE[i]=state==LifecycleState::Visible;true}
unsafe fn reparent_frame_to_scene(fid:u32,scene:u8,_:&str)->bool{let Some(f)=FRAMES.iter_mut().flatten().find(|f|f.frame_id==fid)else{return false};f.scene_id=scene;EFFECTS.push(Effect::Reparent(fid,scene));true}
unsafe fn restore_minimized_frame_inner(fid:u32,_:bool)->bool{let Some(i)=FRAMES.iter().position(|f|f.is_some_and(|f|f.frame_id==fid))else{return false};FRAMES[i].as_mut().unwrap().flags&=!FRAME_FLAG_MINIMIZED;VISIBLE[i]=true;EFFECTS.push(Effect::Restore(fid));true}
unsafe fn get_surface_bounds(sid:u64)->Option<(i32,i32,u32,u32)>{frame_for_surface(sid).map(|_|(0,50,512,359))}
unsafe fn update_local_geometry(_:u64,_:i32,_:i32,_:u32,_:u32){}unsafe fn send_frame_tab_info(_:u32){}unsafe fn tile_active_scene_frames(){}
unsafe fn pdx_call(_:u64,_:u64,_:u64,_:u64,_:u64)->(u64,u64){(0,0)}
unsafe fn desktop_content_ready(_:u64)->bool{CONTENT_READY}
unsafe fn try_set_focus_inner(sid:u64,_:bool)->bool{FOCUSED_SURFACE_ID=sid;LAST_DISPLAY_FOCUS_RESULT=Some((sid,FOCUS_STATUS));EFFECTS.push(Effect::Focus(sid));FOCUS_STATUS==0||FOCUS_STATUS==ERR_SERVICE_NOT_READY}
unsafe fn try_set_focus(sid:u64)->bool{try_set_focus_inner(sid,true)}
unsafe fn launcher_focus_discard(){}
unsafe fn launcher_queue_focus(target:u32,frame:u64,_:launcher::LiveAction,_:u8,_:bool)->bool{let Some(e)=APP_CUSTODY.iter().flatten().find(|e|e.target==target)else{return false};EFFECTS.push(Effect::Queued(target,e.task,frame));true}
unsafe fn open_program_in_active_scene()->bool{false}
fn program_launcher_status()->(bool,&'static str,&'static str){(false,"","")}
fn power_launcher_status(_:Command)->(bool,&'static str,&'static str){(false,"","")}
fn palette_power(_:Command)->bool{false}
static mut PALETTE_FROM_KEY:bool=false;
unsafe fn launcher_action_record(target:u32,frame:u64,_:launcher::LiveAction,_:u8,activated:bool,_:bool){if activated{EFFECTS.push(Effect::Activated(target,frame));}}
unsafe fn first_minimized_frame_id()->Option<u32>{None}
unsafe fn open_spindle_in_active_scene()->bool{false}unsafe fn open_quil_in_active_scene()->bool{false}unsafe fn open_linen_in_active_scene()->bool{false}
unsafe fn open_bell_in_active_scene()->bool{false}unsafe fn open_collar_in_active_scene()->bool{false}unsafe fn open_mesh_in_active_scene()->bool{false}
unsafe fn atlas_toggle(){ATLAS_MODE_ENABLED=!ATLAS_MODE_ENABLED;}
unsafe fn access_handle_keyboard_action(_:SurfaceAction)->bool{false}
unsafe fn palette_render_list(){}
fn quil_launcher_status()->(bool,&'static str,&'static str){(false,"unused","unused")}
fn spindle_launcher_status()->(bool,&'static str,&'static str){(false,"unused","unused")}
fn spindle_active_frame()->Option<(u32,u64)>{None}

#[test]fn browser_row_preserves_existing_indices_and_webstub_identity(){let _l=LOCK.lock().unwrap();unsafe{reset();
 assert_eq!(COMMAND_LIST.len(),14);assert_eq!((COMMAND_LIST[0].command,COMMAND_LIST[1].command),(Command::FocusSpindle,Command::FocusQuil));
 assert_eq!(COMMAND_LIST[8].command,Command::ZoomToggle);assert_eq!(COMMAND_LIST[9].command,Command::MinimizeFocused);
 assert_eq!((COMMAND_LIST[10].command,COMMAND_LIST[10].name),(Command::FocusBrowser,"Open Browser"));assert_eq!(Command::FocusBrowser as u8,10);
 assert_eq!((SURFACE_ID_BROWSER,BROWSER_FRAME_ID),(205,8));assert!(![300,343,344].contains(&SURFACE_ID_BROWSER));
 assert!(palette_execute_selected());assert_eq!(FOCUSED_SURFACE_ID,300);assert!(EFFECTS.contains(&Effect::Focus(300)));assert!(!CALLS.iter().any(|c|c.1));
 assert_eq!(launcher_row_title(Command::FocusBrowser,"Open Browser"),"Focus Browser");
}}
#[test]fn live_or_closing_predecessor_never_activates_or_focuses_future_sibling(){let _l=LOCK.lock().unwrap();unsafe{reset();
 #[cfg(browser_launch_catalog)]{ACTIVATION[1]=receipt(31,310,344,1);}
 assert!(open_browser_in_active_scene());assert!(!CALLS.iter().any(|c|c.1));assert!(!EFFECTS.iter().any(|e|matches!(e,Effect::Focus(343|344))));
 APP_CUSTODY[0].as_mut().unwrap().close_pending=true;EFFECTS.clear();CALLS.clear();assert!(!open_browser_in_active_scene());assert!(!browser_launcher_status().0);assert!(!CALLS.iter().any(|c|c.1));assert!(EFFECTS.is_empty());
}}
#[test]fn held_exact_catalog_is_required_before_browser_focus_restore_or_bring_here(){let _l=LOCK.lock().unwrap();unsafe{reset();
 FRAMES[0].as_mut().unwrap().scene_id=1;assert_eq!(launcher_row_title(Command::FocusBrowser,"Open Browser"),"Bring Here Browser");assert!(open_browser_in_active_scene());assert!(EFFECTS.contains(&Effect::Reparent(10,0)));assert_eq!(FOCUSED_SURFACE_ID,300);
 FRAMES[0].as_mut().unwrap().flags|=FRAME_FLAG_MINIMIZED;assert_eq!(launcher_row_title(Command::FocusBrowser,"Open Browser"),"Restore Browser");assert!(open_browser_in_active_scene());assert!(EFFECTS.contains(&Effect::Restore(10)));
 for bad in[held(15,15,300),held(15,0,300),held(15,150,343),a::CatalogInfo{coordinator_pd:7,..held(15,150,300)}]{reset();HELD[0]=Ok(bad);assert!(!open_browser_in_active_scene());assert!(EFFECTS.is_empty());}
 reset();APP_CUSTODY[0].as_mut().unwrap().slot=154;assert_eq!(launcher_focus_existing(15,300,None),Some(false));assert!(EFFECTS.is_empty());
}}
#[cfg(browser_launch_catalog)]#[test]fn ordered_ready_grants_activate_30_then_31_and_exhaust_honestly(){let _l=LOCK.lock().unwrap();unsafe{reset();
 dead(0,30);assert_eq!(browser_launcher_status().2,"predecessor_cleanup_pending");assert!(!open_browser_in_active_scene());assert!(!CALLS.iter().any(|c|c.1));
 // Ready is a scripted kernel-grant receipt. This fixture does not reimplement
 // or claim the kernel's all-ACK/retired-pages readiness mechanism.
 dead(0,31);ACTIVATION[0]=receipt(30,300,343,1);COMMIT[0]=receipt(30,300,343,2);CALLS.clear();
 assert_eq!(browser_launcher_status().2,"boot_successor_ready");assert_eq!(browser_reserve_label("REOPEN / RESERVED INSTANCE",true,"boot_successor_ready"),"REOPEN / 2 RESERVED LEFT");
 assert!(open_browser_in_active_scene());assert_eq!(CALLS.iter().filter(|c|c.1).copied().collect::<Vec<_>>(),[(156,true),(156,true)]);assert!(EFFECTS.contains(&Effect::Activated(30,343)));
 install(1,343);assert!(open_browser_in_active_scene());assert_eq!(FOCUSED_SURFACE_ID,343);assert_eq!(desktop_style::native_label(343),Some("Browser"));
 APP_CUSTODY[1].as_mut().unwrap().close_pending=true;CALLS.clear();assert!(!open_browser_in_active_scene());assert!(!CALLS.iter().any(|c|c.1));
 dead(1,31);ACTIVATION[0]=receipt(30,300,343,3);assert_eq!(browser_launcher_status().2,"predecessor_cleanup_pending");assert!(!open_browser_in_active_scene());assert_eq!(browser_active_frame(),None);
 ACTIVATION[1]=receipt(31,310,344,1);COMMIT[1]=receipt(31,310,344,2);CALLS.clear();assert!(open_browser_in_active_scene());assert_eq!(CALLS.iter().filter(|c|c.1).copied().collect::<Vec<_>>(),[(160,true),(160,true)]);
 install(2,344);assert!(open_browser_in_active_scene());assert_eq!(FOCUSED_SURFACE_ID,344);assert_eq!(desktop_style::native_label(344),Some("Browser"));
 dead(2,31);ACTIVATION[1]=receipt(31,310,344,3);CALLS.clear();assert_eq!(browser_launcher_status(),(false,"CLOSED / RESERVE EXHAUSTED","boot_reserve_exhausted"));assert_eq!(app_reserve_remaining(app_catalog::App::Browser),Some(0));assert!(!palette_execute_selected());assert!(browser_active_frame().is_none());assert!(!CALLS.iter().any(|c|c.1));
}}
#[cfg(browser_launch_catalog)]#[test]fn wrong_task_slot_surface_and_retry_cannot_skip_or_commit_an_edge(){let _l=LOCK.lock().unwrap();unsafe{
 for bad in[receipt(30,30,343,1),receipt(30,310,343,1),receipt(31,300,343,1),receipt(30,300,300,1),receipt(30,300,343,4),Err(a::RETRY),Err(a::DENIED)]{reset();ACTIVATION[0]=bad;ACTIVATION[1]=receipt(31,310,344,1);assert!(!open_browser_in_active_scene());assert!(!CALLS.iter().any(|c|c.1));assert!(EFFECTS.is_empty());}
 for code in[a::RETRY,a::DENIED,u64::MAX]{reset();dead(0,31);ACTIVATION[0]=receipt(30,300,343,1);COMMIT[0]=Err(code);assert!(!open_browser_in_active_scene());assert!(!EFFECTS.iter().any(|e|matches!(e,Effect::Activated(..))));}
 for state in[0,1,3]{reset();dead(0,31);ACTIVATION[0]=receipt(30,300,343,1);COMMIT[0]=receipt(30,300,343,state);assert!(!open_browser_in_active_scene());assert!(!EFFECTS.iter().any(|e|matches!(e,Effect::Activated(..))));}
}}
#[cfg(browser_launch_catalog)]#[test]fn full_focus_and_old_input_close_view_custody_never_retarget_to_new_instance(){let _l=LOCK.lock().unwrap();unsafe{reset();
 let old=clipboard::Participant::from_catalog(held(15,150,300)).unwrap();let next=clipboard::Participant::from_catalog(held(30,300,343)).unwrap();assert_eq!((old.route,next.route),(43,155));
 let mut input=app_input::Queue::new();let key=app_key::Key::new(0x22,false,false);
 assert!(input.push(app_input::Item{destination:old,input:app_input::Input::Key(key)}));let front=input.front();
 dead(0,31);ACTIVATION[0]=receipt(30,300,343,2);install(1,343);FOCUS_STATUS=ERR_SERVICE_NOT_READY;assert!(open_browser_in_active_scene());assert!(EFFECTS.contains(&Effect::Queued(30,300,343)));assert_eq!(input.front(),front);
 assert!(input.push(app_input::Item{destination:next,input:app_input::Input::Key(key)}));assert_eq!(input.retire(old.subject),1);assert_eq!(input.front().unwrap().destination,next);
 assert_eq!(APP_CUSTODY[0].unwrap().close_slot(),Some(43));assert_eq!(APP_CUSTODY[1].unwrap().close_slot(),Some(155));assert!(!APP_CUSTODY[0].as_mut().unwrap().request_close());assert!(!APP_CUSTODY[1].unwrap().close_pending);
 let mut views=[document_view::Delivery::new(),document_view::Delivery::new()];views[0].queue(15,150,512,359);views[1].queue(30,300,512,359);
 let mut sent=Vec::new();for _ in 0..2{views[0].poll(true,true,|n|{sent.push(n);Err(ERR_SERVICE_NOT_READY)});}assert_eq!(sent[0],sent[1]);assert_eq!((sent[0].target,sent[0].task),(15,150));
 views[0].retire();let mut delivered=None;views[1].poll(true,true,|n|{delivered=Some(n);Ok(())});let n=delivered.unwrap();assert_eq!(document_view::Notice::decode(3,30,300,n.args(),0),Some(n));assert!(document_view::Notice::decode(3,15,150,n.args(),0).is_none());
}}
