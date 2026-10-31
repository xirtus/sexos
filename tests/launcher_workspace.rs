// SPDX-License-Identifier: GPL-3.0-only
//! Actual Shell launcher/reparent/scene functions; held queries and display
//! admission are finite host callbacks, not native authority or pixel proof.
#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../servers/silk-shell/src/launcher.rs"]mod launcher;
#[path="../servers/silk-shell/src/app_custody.rs"]mod app_custody;
use launcher::{Evidence,LiveAction};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]pub struct Operation{pub opcode:u64,pub surface:u64,pub packed:u64,pub control:u64}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]pub enum Admission{Accepted,Full,Rejected(u64)}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]pub enum LatestPoll{Idle,Deferred,Applied(Operation),Rejected(u64)}
mod linen_text_delivery{pub use crate::{Admission,LatestPoll};}
pub const ERR_SERVICE_NOT_READY:u64=9;
static mut NOW:u64=0;
pub fn get_ticks()->u64{unsafe{NOW}}
static mut LAUNCHER_FOCUS:Option<LauncherFocus>=None;
static mut DISPLAY_FOCUS:FocusDelivery=FocusDelivery::new();
static mut ACTUAL_DESKTOP:Option<AdmissionStates>=None;
#[derive(Clone,Copy)]enum Command{FocusQuil,FocusSpindle,Other}
struct Activation{state:u8,successor:u32}
static mut ACTIVATION_STATE:u8=0;
fn quil_activation_info(_:bool)->Result<Activation,u64>{unsafe{Ok(Activation{state:ACTIVATION_STATE,successor:16})}}
fn spindle_activation_info(_:bool)->Result<Activation,u64>{unsafe{Ok(Activation{state:ACTIVATION_STATE,successor:17})}}
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
#[derive(Clone,Copy)]struct Tab{surface_id:u64}
#[derive(Clone,Copy)]struct Frame{frame_id:u32,scene_id:u8,flags:u32,tabs:[Option<Tab>;1]}
const FRAME_FLAG_MINIMIZED:u32=1;
const ATLAS_MAX_SCENES:usize=5;
#[derive(Clone,Copy,PartialEq,Eq)]enum LifecycleState{Visible,Hidden,Minimized,Closing,Tombstoned,Destroyed}
static mut FRAMES:[Option<Frame>;6]=[None;6];
static mut APP_CUSTODY:[Option<app_custody::Entry>;6]=[None;6];
static mut ACTIVE_SCENE_IDX:u8=0;
static mut FOCUSED_SURFACE_ID:u64=0;
static mut LAST_DISPLAY_FOCUS_RESULT:Option<(u64,u64)>=None;
static mut STATE:[LifecycleState;6]=[LifecycleState::Hidden;6];
static mut READY:[bool;6]=[true;6];
static mut CATALOG_BAD_TASK:bool=false;
static mut CATALOG_ERROR:Option<u64>=None;
static mut EVIDENCE:Evidence=Evidence::Live;
static mut STATUS:u64=0;
static mut OPS:Vec<String>=Vec::new();
static mut LOG:Vec<String>=Vec::new();
struct Drag{active:bool}static mut ATLAS_DRAG_INTENT:Drag=Drag{active:false};
struct DesktopStates;static DESKTOP_STATES:DesktopStates=DesktopStates;
impl DesktopStates{
 fn ready(&self,sid:u64)->bool{unsafe{ACTUAL_DESKTOP.as_ref().map_or_else(||READY[index(sid)],|d|d.ready(sid))}}
 fn pending_surface(&self,sid:u64)->bool{!self.ready(sid)}
 fn poll(&self,budget:usize,validate:impl FnMut(Operation)->Option<Operation>,send:impl FnMut(Operation)->Admission){unsafe{if let Some(d)=ACTUAL_DESKTOP.as_mut(){d.poll(budget,validate,send)}}}
}
fn desktop_state_validate(op:Operation)->Option<Operation>{Some(op)}
mod desktop_content{
 pub struct Child{pub sid:u64,pub w:u32,pub h:u32}
 pub fn children(_:u64,_:i32,_:i32,_:u32,_:u32)->[Option<Child>;1]{[Some(Child{sid:unsafe{if crate::ACTUAL_DESKTOP.is_some(){154}else{999}},w:10,h:10})]}
}
const SLOT_DISPLAY:u64=1;
const SURFACE_ID_COMMAND_PALETTE:u64=888;
fn index(sid:u64)->usize{match sid{201=>0,153=>1,301=>2,306=>3,307=>4,999=>5,_=>panic!("unknown sid {sid}")}}
fn log(s:String){unsafe{LOG.push(s)}}
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{crate::log(format!($($arg)*))}}
macro_rules! serial_println{($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
pub mod app_lifecycle{
 pub const LIVE:u64=5;pub const RETRY:u64=2;
 pub struct Catalog{pub target_pd:u32,pub task_id:u32,pub surface_id:u64,pub coordinator_pd:u32}
 pub struct Info{pub target_pd:u32,pub task_id:u32,pub required_mask:u64,pub acked_mask:u64}
 pub fn catalog(slot:usize)->Result<Catalog,u64>{unsafe{if let Some(error)=crate::CATALOG_ERROR{return Err(error)}let e=crate::APP_CUSTODY.iter().flatten().find(|e|e.slot==slot).unwrap();Ok(Catalog{target_pd:e.target,task_id:e.task+crate::CATALOG_BAD_TASK as u32,surface_id:e.surface,coordinator_pd:3})}}
 pub fn info(_:usize)->Result<Info,u64>{unsafe{match crate::EVIDENCE{crate::Evidence::Live=>Err(LIVE),crate::Evidence::Retry=>Err(RETRY),crate::Evidence::Invalid=>Err(1),crate::Evidence::Dead{domain,task,required,acked}=>Ok(Info{target_pd:domain,task_id:task,required_mask:required,acked_mask:acked})}}}
}
unsafe fn frame_for_surface(sid:u64)->Option<u32>{FRAMES.iter().flatten().find(|f|f.tabs.iter().flatten().any(|t|t.surface_id==sid)).map(|f|f.frame_id)}
unsafe fn frame_is_minimized(fid:u32)->bool{FRAMES.iter().flatten().find(|f|f.frame_id==fid).unwrap().flags&FRAME_FLAG_MINIMIZED!=0}
unsafe fn lifecycle_state(sid:u64)->Option<LifecycleState>{Some(STATE[index(sid)])}
unsafe fn set_lifecycle_state(sid:u64,next:LifecycleState)->bool{STATE[index(sid)]=next;true}
unsafe fn surface_is_alive(_:u64)->bool{true}
unsafe fn surface_is_lifecycle_focusable(sid:u64)->bool{STATE[index(sid)]==LifecycleState::Visible}
unsafe fn is_tombstoned(_:u64)->bool{false}
unsafe fn get_surface_bounds(_:u64)->Option<(i32,i32,u32,u32)>{Some((10,60,500,300))}
unsafe fn raw_pdx_call(_:u64,op:u64,sid:u64,_:u64,_:u64)->(u64,u64){OPS.push(format!("send:{op:x}:{sid}"));if op==0xEC&&ACTUAL_DESKTOP.is_none(){READY[index(sid)]=STATUS==0;}(STATUS,0)}
unsafe fn pdx_call(slot:u64,op:u64,sid:u64,a:u64,b:u64)->(u64,u64){let result=raw_pdx_call(slot,op,sid,a,b);if let Some(d)=ACTUAL_DESKTOP.as_mut(){d.observe(Operation{opcode:op,surface:sid,packed:a,control:b},match result.0{0=>Admission::Accepted,9=>Admission::Full,e=>Admission::Rejected(e)});}if op==0xED{LAST_DISPLAY_FOCUS_RESULT=Some((sid,result.0));DISPLAY_FOCUS.observe(sid,match result.0{0=>Admission::Accepted,9=>Admission::Full,e=>Admission::Rejected(e)});}result}
unsafe fn update_local_geometry(_:u64,_:i32,_:i32,_:u32,_:u32){}
unsafe fn send_frame_tab_info(_:u32){}
unsafe fn tile_active_scene_frames(){OPS.push("tile".into());}
unsafe fn try_set_focus(sid:u64)->bool{try_set_focus_inner(sid,true)}
unsafe fn try_set_focus_inner(sid:u64,refresh_children:bool)->bool{launcher_focus_discard();if refresh_children&&ACTUAL_DESKTOP.is_some(){ACTUAL_DESKTOP.as_mut().unwrap().observe(Operation{opcode:0xEC,surface:154,packed:1,control:1},Admission::Accepted);}let f=FRAMES.iter().flatten().find(|f|f.frame_id==sid as u32).unwrap();if f.scene_id!=ACTIVE_SCENE_IDX||frame_is_minimized(f.frame_id)||STATE[index(sid)]!=LifecycleState::Visible{return false}FOCUSED_SURFACE_ID=sid;pdx_call(SLOT_DISPLAY,0xED,sid,0,0);true}
unsafe fn restore_minimized_frame_inner(fid:u32,focus:bool)->bool{OPS.push(format!("restore:{fid}"));let f=FRAMES.iter_mut().flatten().find(|f|f.frame_id==fid).unwrap();f.flags&=!FRAME_FLAG_MINIMIZED;STATE[index(fid as u64)]=LifecycleState::Visible;pdx_call(SLOT_DISPLAY,0xEC,fid as u64,0,0);if focus&&DESKTOP_STATES.ready(fid as u64){try_set_focus(fid as u64);}true}
unsafe fn clear_focus_if_wrong_scene(){}unsafe fn clear_drag_if_wrong_scene(){}unsafe fn clear_hover_if_wrong_scene(){}
unsafe fn atlas_capture_snapshot(){}unsafe fn atlas_same_scene_drop_noop(_:u32,_:u8,_:u8)->bool{true}
unsafe fn surface_in_active_scene(sid:u64)->bool{FRAMES.iter().flatten().any(|f|f.frame_id==sid as u32&&f.scene_id==ACTIVE_SCENE_IDX)}
unsafe fn sync_lifecycle_scene_visibility(){for f in FRAMES.iter().flatten(){let sid=f.frame_id as u64;if f.flags&FRAME_FLAG_MINIMIZED==0{STATE[index(sid)]=if f.scene_id==ACTIVE_SCENE_IDX{LifecycleState::Visible}else{LifecycleState::Hidden};}}}
include!(env!("LAUNCHER_WORKSPACE_PRODUCTION"));
unsafe fn reset(target:u32,sid:u64,scene:u8,minimized:bool,visible:bool){
 APP_CUSTODY=[None;6];FRAMES=[None;6];STATE=[LifecycleState::Hidden;6];READY=[true;6];ACTIVE_SCENE_IDX=0;FOCUSED_SURFACE_ID=0;LAST_DISPLAY_FOCUS_RESULT=None;CATALOG_BAD_TASK=false;CATALOG_ERROR=None;LAUNCHER_FOCUS=None;DISPLAY_FOCUS=FocusDelivery::new();ACTUAL_DESKTOP=None;NOW=0;ACTIVATION_STATE=0;EVIDENCE=Evidence::Live;STATUS=0;OPS.clear();LOG.clear();ATLAS_DRAG_INTENT.active=false;
 let content=match target{9=>156,12=>154,16=>301,17=>302,18=>307,_=>panic!()};APP_CUSTODY[0]=Some(app_custody::Entry::new(38,target,target*100+1,content));FRAMES[0]=Some(Frame{frame_id:sid as u32,scene_id:scene,flags:if minimized{1}else{0},tabs:[Some(Tab{surface_id:sid})]});STATE[index(sid)]=if minimized{LifecycleState::Minimized}else if visible{LifecycleState::Visible}else{LifecycleState::Hidden};
}
#[test]fn original_and_reserved_live_instances_move_without_activation_and_focus_after_visibility(){let _l=LOCK.lock().unwrap();unsafe{
 for(target,sid)in[(9,201),(12,153),(16,301),(17,306),(18,307)]{reset(target,sid,2,false,false);assert_eq!(launcher_live_action(target),Some(LiveAction::BringHere));assert_eq!(launcher_focus_existing(target,sid,None),Some(true));assert_eq!(FRAMES[0].unwrap().scene_id,0);assert_eq!(FOCUSED_SURFACE_ID,sid);let create=OPS.iter().position(|s|s==&format!("send:ec:{sid}")).unwrap();let focus=OPS.iter().position(|s|s==&format!("send:ed:{sid}")).unwrap();assert!(create<focus);assert!(LOG.last().unwrap().contains("action=BringHere from=2 to=0 visible=1 focused=1 activated=0 queued=0 accepted=1 ok=1"));assert_eq!(APP_CUSTODY[0].unwrap().task,target*100+1);}
}}
#[test]fn minimized_remote_frame_stays_hidden_during_move_then_restores_before_focus(){let _l=LOCK.lock().unwrap();unsafe{reset(17,306,3,true,false);assert_eq!(launcher_focus_existing(17,306,None),Some(true));let hidden=OPS.iter().position(|s|s=="send:ee:306").unwrap();let restore=OPS.iter().position(|s|s=="restore:306").unwrap();let create=OPS.iter().position(|s|s=="send:ec:306").unwrap();let focus=OPS.iter().position(|s|s=="send:ed:306").unwrap();assert!(hidden<restore&&restore<create&&create<focus);assert!(!frame_is_minimized(306));assert!(LOG.last().unwrap().contains("action=BringHere"));}}
#[test]fn same_workspace_minimized_restore_and_visible_focus_use_exact_current_frame(){let _l=LOCK.lock().unwrap();unsafe{reset(12,153,0,true,false);assert_eq!(launcher_live_action(12),Some(LiveAction::Restore));assert_eq!(launcher_focus_existing(12,153,None),Some(true));assert!(LOG.last().unwrap().contains("action=Restore"));OPS.clear();assert_eq!(launcher_live_action(12),Some(LiveAction::Focus));assert_eq!(launcher_focus_existing(12,153,None),Some(true));assert_eq!(OPS,["send:ed:153"]);assert!(LOG.last().unwrap().contains("action=Focus"));}}
#[test]fn stale_catalog_closing_terminal_and_retry_never_move_restore_or_focus(){let _l=LOCK.lock().unwrap();unsafe{for bad in 0..5{reset(12,153,2,true,false);match bad{0=>CATALOG_BAD_TASK=true,1=>APP_CUSTODY[0].as_mut().unwrap().close_pending=true,2=>APP_CUSTODY[0].as_mut().unwrap().dead=true,3=>EVIDENCE=Evidence::Retry,_=>EVIDENCE=Evidence::Invalid}assert_eq!(launcher_focus_existing(12,153,None),Some(false));assert!(OPS.is_empty());assert_eq!(FRAMES[0].unwrap().scene_id,2);assert_eq!(FOCUSED_SURFACE_ID,0);}}}
#[test]fn missing_frame_is_initial_open_only_and_wrong_frame_cannot_alias_live_subject(){let _l=LOCK.lock().unwrap();unsafe{reset(12,153,0,false,true);FRAMES[0]=None;assert_eq!(launcher_live_action(12),Some(LiveAction::Open));assert_eq!(launcher_focus_existing(12,153,None),None);assert_eq!(launcher_focus_existing(12,306,None),Some(false));assert!(OPS.is_empty());}}
#[test]fn child_and_parent_admission_delay_queue_without_early_focus_or_scene_replay(){let _l=LOCK.lock().unwrap();unsafe{
 reset(12,153,2,false,false);READY[index(999)]=false;
 assert_eq!(launcher_focus_existing(12,153,None),Some(true));assert_eq!(FOCUSED_SURFACE_ID,0);
 assert!(LAUNCHER_FOCUS.is_some());assert!(LOG.last().unwrap().contains("visible=0 focused=0 activated=0 queued=1 accepted=1 ok=0"));
 let effects=OPS.clone();for _ in 0..4{launcher_focus_poll()}assert_eq!(OPS,effects);
 READY[index(999)]=true;launcher_focus_poll();assert!(LAUNCHER_FOCUS.is_none());assert_eq!(FOCUSED_SURFACE_ID,153);
 assert_eq!(OPS.iter().filter(|op|op.as_str()=="send:ec:153").count(),1);
 assert_eq!(OPS.iter().filter(|op|op.as_str()=="send:ed:153").count(),1);
 assert!(LOG.last().unwrap().contains("queued=0 accepted=1 ok=1"));
}}
#[test]fn display_full_focus_has_one_actual_retry_then_no_accepted_replay(){let _l=LOCK.lock().unwrap();unsafe{
 reset(12,153,0,false,true);STATUS=9;assert_eq!(launcher_focus_existing(12,153,None),Some(true));
 assert!(LAUNCHER_FOCUS.unwrap().requested);assert!(LOG.last().unwrap().contains("focused=0 activated=0 queued=1 accepted=1 ok=0"));
 desktop_state_poll();launcher_focus_poll();assert!(LAUNCHER_FOCUS.is_some());
 STATUS=0;desktop_state_poll();assert_eq!(LAST_DISPLAY_FOCUS_RESULT,Some((153,0)));launcher_focus_poll();assert!(LAUNCHER_FOCUS.is_none());
 let n=OPS.len();for _ in 0..8{desktop_state_poll();launcher_focus_poll()}assert_eq!(OPS.len(),n);
 assert_eq!(OPS.iter().filter(|op|op.as_str()=="send:ed:153").count(),3); // two Full, one Accepted
 assert!(LOG.last().unwrap().contains("queued=0 accepted=1 ok=1"));
}}
#[test]fn hard_focus_denial_is_not_accepted_or_queued(){let _l=LOCK.lock().unwrap();unsafe{
 reset(12,153,0,false,true);STATUS=u64::MAX;assert_eq!(launcher_focus_existing(12,153,None),Some(false));assert!(LAUNCHER_FOCUS.is_none());
 assert!(LOG.last().unwrap().contains("focused=0 activated=0 queued=0 accepted=0 ok=0"));
}}
#[test]fn existing_minimized_unrelated_frame_is_not_reactivated_during_bring_here(){let _l=LOCK.lock().unwrap();unsafe{reset(12,153,2,false,false);FRAMES[1]=Some(Frame{frame_id:201,scene_id:0,flags:1,tabs:[Some(Tab{surface_id:201})]});STATE[index(201)]=LifecycleState::Minimized;assert_eq!(launcher_focus_existing(12,153,None),Some(true));assert!(OPS.iter().any(|s|s=="send:ee:201"));assert!(!OPS.iter().any(|s|s=="send:ec:201"));}}

#[test]fn transient_held_queries_preserve_intent_until_exact_live_without_replaying_effects(){let _l=LOCK.lock().unwrap();unsafe{
 for phase in 0..4{
  reset(12,153,2,false,false);READY[index(999)]=false;assert_eq!(launcher_focus_existing(12,153,None),Some(true));
  READY[index(999)]=true;let before=OPS.clone();
  match phase{0=>CATALOG_ERROR=Some(2),1=>CATALOG_ERROR=Some(9),2=>EVIDENCE=Evidence::Retry,_=>EVIDENCE=Evidence::Invalid}
  launcher_focus_poll();
  if phase==3{assert!(LAUNCHER_FOCUS.is_none());assert_eq!(OPS,before);continue}
  assert!(LAUNCHER_FOCUS.is_some());assert_eq!(OPS,before);CATALOG_ERROR=None;EVIDENCE=Evidence::Live;launcher_focus_poll();
  assert!(LAUNCHER_FOCUS.is_none());assert_eq!(FOCUSED_SURFACE_ID,153);assert_eq!(OPS.len(),before.len()+1);
 }
}}
#[test]fn pending_identity_fences_superseding_focus_scene_minimize_and_deadlines_cancel(){let _l=LOCK.lock().unwrap();unsafe{
 for cause in 0..9{
  reset(12,153,0,false,true);STATUS=9;assert_eq!(launcher_focus_existing(12,153,None),Some(true));let n=OPS.len();
  match cause{0=>CATALOG_BAD_TASK=true,1=>APP_CUSTODY[0].as_mut().unwrap().close_pending=true,2=>APP_CUSTODY[0].as_mut().unwrap().dead=true,
   3=>FOCUSED_SURFACE_ID=0,4=>ACTIVE_SCENE_IDX=1,5=>FRAMES[0].as_mut().unwrap().flags=1,6=>NOW=2000,
   7=>LAUNCHER_FOCUS.as_mut().unwrap().turns=4095,_=>{LAUNCHER_FOCUS.as_mut().unwrap().last=1;NOW=0;}}
  launcher_focus_poll();assert!(LAUNCHER_FOCUS.is_none(),"cause {cause}");STATUS=0;desktop_state_poll();assert_eq!(OPS.len(),n,"cause {cause}");
 }
}}
#[test]fn focus_retry_waits_for_fresh_live_and_cancels_before_any_postdeadline_send(){let _l=LOCK.lock().unwrap();unsafe{
 reset(12,153,0,false,true);STATUS=9;assert_eq!(launcher_focus_existing(12,153,None),Some(true));let n=OPS.len();STATUS=0;CATALOG_ERROR=Some(2);
 desktop_state_poll();launcher_focus_poll();assert_eq!(OPS.len(),n);assert!(LAUNCHER_FOCUS.is_some());
 NOW=2000;desktop_state_poll();launcher_focus_poll();assert_eq!(OPS.len(),n);assert!(LAUNCHER_FOCUS.is_none());
 CATALOG_ERROR=None;desktop_state_poll();assert_eq!(OPS.len(),n);
}}
#[test]fn reserved_ready_titles_truthfully_offer_open_and_live_titles_follow_frame(){let _l=LOCK.lock().unwrap();unsafe{
 reset(12,153,0,false,true);ACTIVATION_STATE=1;assert_eq!(launcher_row_title(Command::FocusSpindle,"fallback"),"Open Spindle");
 assert_eq!(launcher_row_title(Command::FocusQuil,"fallback"),"Open Quil");ACTIVATION_STATE=0;
 assert_eq!(launcher_row_title(Command::FocusSpindle,"fallback"),"Focus Spindle");FRAMES[0].as_mut().unwrap().scene_id=1;
 assert_eq!(launcher_row_title(Command::FocusSpindle,"fallback"),"Bring Here Spindle");
}}

#[test]fn actual_focus_retry_never_sends_on_live_deadline_or_terminal_or_child_denial(){let _l=LOCK.lock().unwrap();unsafe{
 for cause in 0..5{
  reset(12,153,0,false,true);STATUS=9;assert_eq!(launcher_focus_existing(12,153,None),Some(true));let n=OPS.len();STATUS=0;
  match cause{0=>NOW=2000,1=>LAUNCHER_FOCUS.as_mut().unwrap().turns=4095,2=>APP_CUSTODY[0].as_mut().unwrap().dead=true,3=>CATALOG_BAD_TASK=true,_=>READY[index(999)]=false}
  desktop_state_poll();assert_eq!(OPS.len(),n,"cause {cause}");launcher_focus_poll();
  if cause<4{assert!(LAUNCHER_FOCUS.is_none());desktop_state_poll();assert_eq!(OPS.len(),n)}else{assert!(LAUNCHER_FOCUS.is_some());READY[index(999)]=true;desktop_state_poll();launcher_focus_poll();assert_eq!(OPS.len(),n+1)}
 }
}}
#[test]fn superseding_same_focus_is_explicit_and_does_not_leave_old_retry(){let _l=LOCK.lock().unwrap();unsafe{
 reset(12,153,0,false,true);STATUS=9;assert_eq!(launcher_focus_existing(12,153,None),Some(true));STATUS=0;
 try_set_focus(153);let n=OPS.len();assert!(LAUNCHER_FOCUS.is_none());desktop_state_poll();assert_eq!(OPS.len(),n);
}}

#[test]fn actual_desktop_child_full_and_rejection_block_focus_until_real_admission(){let _l=LOCK.lock().unwrap();unsafe{
 for status in [Admission::Full,Admission::Rejected(7)]{
  reset(12,153,2,false,false);ACTUAL_DESKTOP=Some(AdmissionStates::new());
  let child=Operation{opcode:0xEC,surface:154,packed:1,control:1};ACTUAL_DESKTOP.as_mut().unwrap().observe(child,status);
  assert_eq!(launcher_focus_existing(12,153,None),Some(true));assert!(LAUNCHER_FOCUS.is_some());assert_eq!(FOCUSED_SURFACE_ID,0);
  STATUS=9;for _ in 0..4{desktop_state_poll();launcher_focus_poll();}assert!(!OPS.iter().any(|s|s=="send:ed:153"));
  STATUS=0;
  if status==Admission::Rejected(7){
   for _ in 0..4{desktop_state_poll();launcher_focus_poll();}assert!(!OPS.iter().any(|s|s=="send:ed:153"));
   assert!(!ACTUAL_DESKTOP.as_ref().unwrap().ready(154));ACTUAL_DESKTOP.as_mut().unwrap().observe(child,Admission::Accepted);
  }
  for _ in 0..12{desktop_state_poll();launcher_focus_poll();}
  assert!(ACTUAL_DESKTOP.as_ref().unwrap().ready(154));assert!(LAUNCHER_FOCUS.is_none());assert_eq!(FOCUSED_SURFACE_ID,153);
  assert_eq!(OPS.iter().filter(|s|s.as_str()=="send:ed:153").count(),1);
  assert!(LOG.last().unwrap().contains("queued=0 accepted=1 ok=1"));
 }
}}

#[test]fn palette_close_handoff_reconciles_its_actual_target_focus_without_duplicate_admission(){let _l=LOCK.lock().unwrap();unsafe{
 for first in [0,9]{
  reset(12,153,0,false,true);FOCUSED_SURFACE_ID=SURFACE_ID_COMMAND_PALETTE;READY[index(999)]=false;
  assert_eq!(launcher_focus_existing(12,153,None),Some(true));let pending=LAUNCHER_FOCUS;assert!(!pending.unwrap().requested);
  STATUS=first;assert!(try_set_focus_inner(153,false));launcher_palette_closed(pending);assert!(LAUNCHER_FOCUS.unwrap().requested);
  let n=OPS.len();READY[index(999)]=true;STATUS=0;desktop_state_poll();launcher_focus_poll();
  assert!(LAUNCHER_FOCUS.is_none());assert_eq!(OPS.len(),n+(first==9)as usize);
  let accepted=OPS.len();for _ in 0..4{desktop_state_poll();launcher_focus_poll();}assert_eq!(OPS.len(),accepted);
  assert!(LOG.last().unwrap().contains("queued=0 accepted=1 ok=1"));
 }
}}
#[test]fn palette_close_other_fallback_retains_only_desired_target_and_denied_target_cancels(){let _l=LOCK.lock().unwrap();unsafe{
 reset(12,153,0,false,true);FOCUSED_SURFACE_ID=SURFACE_ID_COMMAND_PALETTE;READY[index(999)]=false;
 assert_eq!(launcher_focus_existing(12,153,None),Some(true));let pending=LAUNCHER_FOCUS;FOCUSED_SURFACE_ID=0;LAST_DISPLAY_FOCUS_RESULT=Some((0,0));launcher_palette_closed(pending);
 assert!(!LAUNCHER_FOCUS.unwrap().requested);READY[index(999)]=true;launcher_focus_poll();assert!(LAUNCHER_FOCUS.is_none());assert_eq!(OPS,["send:ed:153"]);
 reset(12,153,0,false,true);FOCUSED_SURFACE_ID=SURFACE_ID_COMMAND_PALETTE;READY[index(999)]=false;
 assert_eq!(launcher_focus_existing(12,153,None),Some(true));let pending=LAUNCHER_FOCUS;STATUS=7;try_set_focus_inner(153,false);launcher_palette_closed(pending);
 assert!(LAUNCHER_FOCUS.is_none());let n=OPS.len();STATUS=0;READY[index(999)]=true;desktop_state_poll();launcher_focus_poll();assert_eq!(OPS.len(),n);
}}

#[test]fn slow_live_content_with_an_advancing_clock_waits_until_the_deadline_not_a_poll_count(){let _l=LOCK.lock().unwrap();unsafe{
 // The 1800-second compact session: Spindle content took ~1100-1600 ticks
 // to become ready and 4096 polls cancelled the restore focus first.
 reset(12,153,0,false,true);READY[index(999)]=false;assert_eq!(launcher_focus_existing(12,153,None),Some(true));
 // 15,992 polls, far beyond the old 4096 bound, as ticks advance to 1999.
 for poll in 0..15_992u64{NOW=poll/8;launcher_focus_poll();assert!(LAUNCHER_FOCUS.is_some(),"poll {poll}");}
 // A clock that stops advancing is still bounded by polls.
 for _ in 0..4096{launcher_focus_poll();}assert!(LAUNCHER_FOCUS.is_none());
}}
