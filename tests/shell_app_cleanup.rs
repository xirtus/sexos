#![allow(dead_code,static_mut_refs)]
extern crate self as sex_pdx;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
#[macro_export] macro_rules! serial_record {($($t:tt)*)=>{let _=format_args!($($t)*);}}
#[derive(Clone,Copy)] pub struct PdxMessage {type_id:u64,caller_pd:u32,arg0:u64,arg1:u64,arg2:u64}
pub mod app_lifecycle {
 pub const OP_APP_EXIT:u64=0x107;
 pub const OP_APP_CLOSE:u64=0x108;
 pub struct Info {pub target_pd:u32,pub task_id:u32,pub required_mask:u64}
 pub fn info(_:usize)->Result<Info,u64> {unsafe {if super::INFO_STATUS!=0{return Err(super::INFO_STATUS)} Ok(Info{target_pd:super::INFO_TARGET,task_id:super::INFO_TASK,required_mask:31})}}
 pub fn ack(_:usize,_:u32)->Result<(),u64>{unsafe{assert!(super::PUBLISHED);assert!(super::APP_CUSTODY[0].unwrap().dead);assert!(super::APP_CUSTODY[0].unwrap().retired);super::ACKS+=1;if super::ACK_STATUS==0{Ok(())}else{Err(super::ACK_STATUS)}}}
}
#[derive(Clone,Copy)]struct Tab{surface_id:u64}
#[derive(Clone,Copy)]struct Frame{frame_id:u32,tabs:[Option<Tab>;4],active_tab:u8,tab_count:u8,scene_id:u8}
#[derive(Clone,Copy)]struct Grant{subject_id:u64}
struct Desc{window_id:u64}struct Window{desc:Desc}
#[derive(Clone,Copy,PartialEq,Eq)]enum LifecycleState{Visible,Closing,Tombstoned,Destroyed}
#[derive(Clone,Copy,PartialEq,Eq,Debug)]enum InteractionState{Idle,ClickPending,Dragging{surface_id:u64},Resizing{surface_id:u64},TabDragging{frame_id:u32}}
enum UpdateKind{SetActiveApp}
const MAX_TABS_PER_FRAME:u8=4;const HOVER_NONE:u32=0;const FRAME_LIGHT_NONE:u8=0;
static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[None;3];
static mut FRAMES:[Option<Frame>;3]=[None;3];static mut COLLAR_GRANTS:[Option<Grant>;3]=[None;3];
static mut WINDOWS:Vec<Window>=Vec::new();
static mut HOVERED_FRAME_ID:u32=0;static mut HOVER_KIND:u32=0;static mut HOVERED_FRAME_LIGHT:u8=0;
static mut TAB_DRAG_PENDING_FRAME_ID:u32=0;static mut DRAG_PENDING_ACTIVE:bool=false;static mut DRAG_PENDING_TARGET:u64=0;static mut DRAG_PENDING_KIND:u8=0;
static mut INTERACTION:InteractionState=InteractionState::Idle;static mut FOCUSED_SURFACE_ID:u64=0;static mut ACTIVE_SCENE_IDX:u8=0;
static mut INFO_STATUS:u64=0;static mut INFO_TARGET:u32=15;static mut INFO_TASK:u32=80;static mut ACK_STATUS:u64=0;static mut ACKS:u32=0;
static mut PUBLISHED:bool=false;static mut RETIRES:u32=0;static mut FOCUS_REF:u64=0;static mut BAR:u64=0;
fn lifecycle_state(_:u64)->Option<LifecycleState>{Some(LifecycleState::Visible)}
fn set_lifecycle_state(_:u64,_:LifecycleState){}
fn try_set_focus(sid:u64)->bool {unsafe{if sid==300{return false}FOCUSED_SURFACE_ID=sid;}true}
fn sync_focus_ref(){unsafe{FOCUS_REF=FOCUSED_SURFACE_ID}}
fn send_silkbar_phase2_update(_:u32,sid:u64,_:u64){unsafe{BAR=sid}}
fn tile_active_scene_frames(){}
fn snap_capture_layout(){unsafe{RETIRES+=1}}
fn emit_snapshot(){unsafe{assert!(!FRAMES.iter().flatten().flat_map(|f|f.tabs.iter().flatten()).any(|t|t.surface_id==300));PUBLISHED=true}}
include!(env!("SHELL_CLEANUP_SOURCE"));
unsafe fn reset(){
 APP_CUSTODY=[Some(app_custody::Entry::new(38,15,80,300)),None,None];
 FRAMES=[Some(Frame{frame_id:10,tabs:[Some(Tab{surface_id:300}),Some(Tab{surface_id:205}),Some(Tab{surface_id:300}),None],active_tab:1,tab_count:3,scene_id:0}),Some(Frame{frame_id:11,tabs:[Some(Tab{surface_id:300}),None,None,None],active_tab:0,tab_count:1,scene_id:0}),None];
 COLLAR_GRANTS=[Some(Grant{subject_id:300}),Some(Grant{subject_id:205}),None];
 WINDOWS=vec![Window{desc:Desc{window_id:300}},Window{desc:Desc{window_id:2}}];
 HOVERED_FRAME_ID=10;HOVER_KIND=1;HOVERED_FRAME_LIGHT=1;TAB_DRAG_PENDING_FRAME_ID=10;DRAG_PENDING_TARGET=300;DRAG_PENDING_ACTIVE=true;DRAG_PENDING_KIND=3;
 INTERACTION=InteractionState::Dragging{surface_id:300};FOCUSED_SURFACE_ID=300;ACTIVE_SCENE_IDX=0;
 INFO_STATUS=0;INFO_TARGET=15;INFO_TASK=80;ACK_STATUS=0;ACKS=0;PUBLISHED=false;RETIRES=0;FOCUS_REF=300;BAR=300;
}
fn notice(caller:u32,target:u64,task:u64)->PdxMessage{PdxMessage{type_id:0x107,caller_pd:caller,arg0:target,arg1:task,arg2:0}}
#[test]fn untrusted_hints_and_full_width_aliases_do_not_change_state(){unsafe{reset();for m in [notice(15,15,80),notice(0,15+(1<<32),80),notice(0,15,80+(1<<32)),notice(0,10,80)]{assert!(shell_lifecycle_hint(&m));assert!(!APP_CUSTODY[0].unwrap().pending);assert!(!PUBLISHED);assert_eq!(ACKS,0);}}}
#[test]fn hint_is_not_death_attestation(){unsafe{reset();INFO_STATUS=5;shell_lifecycle_hint(&notice(0,15,80));assert!(APP_CUSTODY[0].unwrap().pending);assert!(!APP_CUSTODY[0].unwrap().dead);assert!(!PUBLISHED);INFO_STATUS=0;INFO_TASK=81;shell_lifecycle_poll();assert!(!PUBLISHED);INFO_TASK=80;shell_lifecycle_poll();assert!(PUBLISHED);}}
#[test]fn cleanup_removes_every_owned_tab_preserving_unrelated_frame_and_prototypes(){unsafe{reset();shell_lifecycle_hint(&notice(0,15,80));let f=FRAMES[0].unwrap();assert_eq!(f.tab_count,1);assert_eq!(f.active_tab,0);assert_eq!(f.tabs[0].unwrap().surface_id,205);assert!(f.tabs[1..].iter().all(Option::is_none));assert!(FRAMES[1].is_none());assert!(COLLAR_GRANTS[0].is_none());assert_eq!(COLLAR_GRANTS[1].unwrap().subject_id,205);assert_eq!(WINDOWS.len(),1);assert_eq!(WINDOWS[0].desc.window_id,2);assert_eq!(FOCUSED_SURFACE_ID,205);assert_eq!(FOCUS_REF,205);assert_eq!(BAR,205);assert_eq!(HOVERED_FRAME_ID,0);assert!(!DRAG_PENDING_ACTIVE);assert_eq!(INTERACTION,InteractionState::Idle);assert_eq!(ACKS,1);}}
#[test]fn ack_retry_never_retires_or_frees_again(){unsafe{reset();ACK_STATUS=2;shell_lifecycle_hint(&notice(0,15,80));assert!(PUBLISHED);assert!(!APP_CUSTODY[0].unwrap().acked);assert_eq!(RETIRES,1);shell_lifecycle_poll();assert_eq!(RETIRES,1);ACK_STATUS=0;shell_lifecycle_poll();assert!(APP_CUSTODY[0].unwrap().acked);shell_lifecycle_hint(&notice(0,15,80));assert_eq!(ACKS,3);assert_eq!(RETIRES,1);}}
#[test]fn permanent_fence_rejects_queued_registration_after_retirement(){unsafe{reset();assert!(APP_CUSTODY[0].unwrap().allows(15,300,15));shell_lifecycle_hint(&notice(0,15,80));let e=APP_CUSTODY[0].unwrap();assert!(!e.allows(15,300,15));assert!(!e.allows(3,300,15));}}
#[test]fn wrong_owner_surface_and_manifest_identity_denied(){let e=app_custody::Entry::new(38,15,80,300);assert!(!e.allows(9,300,15));assert!(!e.allows(15,205,15));assert!(!e.allows(15,300,9));assert!(!e.clone().finish_ack());}
#[test]fn all_interaction_variants_and_no_focus_fallback(){unsafe{for state in [InteractionState::Resizing{surface_id:300},InteractionState::TabDragging{frame_id:10},InteractionState::ClickPending]{reset();INTERACTION=state;FRAMES[0].as_mut().unwrap().tabs[1]=None;shell_lifecycle_hint(&notice(0,15,80));assert_eq!(INTERACTION,InteractionState::Idle);assert_eq!(FOCUSED_SURFACE_ID,0);assert_eq!(FOCUS_REF,0);assert_eq!(BAR,0);}}}
#[test]fn native_shell_aliases_are_exact_catalog_tuples(){assert_eq!(app_custody::Entry::new(38,9,80,156).shell_surface(),Some(201));assert_eq!(app_custody::Entry::new(39,12,81,154).shell_surface(),Some(153));assert_eq!(app_custody::Entry::new(39,15,81,154).shell_surface(),None);assert_eq!(app_custody::Entry::new(39,12,81,157).shell_surface(),None);assert!(!app_custody::Entry::new(40,15,82,300).owns_surface(205));}
#[test]fn endpoint_does_not_authorize_input_pointer_or_other_shell_operations(){for target in [9,12,15]{let e=app_custody::Entry::new(38,target,80,300);for op in [0x14,0x202,0x203,0xfb,1,0xffff]{assert!(!e.allows_operation(op,[300,1,0]));}assert!(e.allows_operation(0xfa,[300,300,(target as u64)<<8]));assert!(!e.allows_operation(0xfa,[301,300,0]));}assert!(app_custody::buffer_sender(2));for caller in [0,3,9,12,15,u32::MAX]{assert!(!app_custody::buffer_sender(caller));}}
#[test]fn spindle_preserves_only_bounded_existing_launcher_route(){let mut e=app_custody::Entry::new(39,12,81,154);for action in 0..8{assert!(e.allows_operation(0x15,[action,0,0]));}for args in [[8,0,0],[1<<32,0,0],[1,1,0],[1,0,1]]{assert!(!e.allows_operation(0x15,args));}assert!(!app_custody::Entry::new(40,15,82,300).allows_operation(0x15,[1,0,0]));e.dead=true;assert!(!e.allows_operation(0xfa,[154,154,12<<8]));assert!(!e.allows_operation(0x15,[1,0,0]));}

static mut TICK:u64=0;static mut CLOSE_FAIL:bool=false;static mut CLOSE_SENDS:Vec<(u64,u64,u64,u64,u64)>=Vec::new();
pub fn get_ticks()->u64{unsafe{TICK}}
fn pdx_call_checked(slot:u64,op:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{unsafe{CLOSE_SENDS.push((slot,op,a,b,c));if CLOSE_FAIL{Err(2)}else{Ok(0)}}}
#[test]fn successful_close_enqueue_is_not_exit_and_retries_until_attested_dead(){unsafe{reset();TICK=100;CLOSE_FAIL=false;CLOSE_SENDS.clear();shell_request_app_close(300);assert_eq!(CLOSE_SENDS,vec![(43,0x108,15,80,0)]);assert!(APP_CUSTODY[0].unwrap().close_pending);assert!(!APP_CUSTODY[0].unwrap().dead);assert_eq!(ACKS,0);for tick in 101..116{TICK=tick;shell_close_poll();}assert_eq!(CLOSE_SENDS.len(),1);TICK=116;shell_close_poll();assert_eq!(CLOSE_SENDS.len(),2);shell_lifecycle_hint(&notice(0,15,80));TICK=1000;shell_close_poll();assert_eq!(CLOSE_SENDS.len(),2);assert!(!APP_CUSTODY[0].unwrap().close_pending);}}
#[test]fn full_close_queue_retains_intent_and_duplicate_ui_requests_do_not_flood(){unsafe{reset();TICK=0;CLOSE_FAIL=true;CLOSE_SENDS.clear();shell_request_app_close(300);for _ in 0..5{shell_request_app_close(300);}assert_eq!(CLOSE_SENDS.len(),1);TICK=16;CLOSE_FAIL=false;shell_close_poll();assert_eq!(CLOSE_SENDS.len(),2);assert!(APP_CUSTODY[0].unwrap().close_pending);assert!(!APP_CUSTODY[0].unwrap().allows_operation(0xfa,[300,300,15<<8]));assert_eq!(ACKS,0);}}
#[test]fn close_routes_derive_only_exact_boot_native_associations(){for (target,sid,slot) in [(9,156,41),(12,154,42),(15,300,43)]{let mut e=app_custody::Entry::new(38,target,80,sid);assert_eq!(e.close_slot(),Some(slot));assert!(e.request_close());}let mut wrong=app_custody::Entry::new(38,15,80,205);assert_eq!(wrong.close_slot(),None);assert!(!wrong.request_close());}

static mut REG_REPLY_FAIL:bool=false;static mut REG_REPLIES:Vec<(u32,u64)>=Vec::new();
fn pdx_reply(target:u32,value:u64)->u64{unsafe{REG_REPLIES.push((target,value));if REG_REPLY_FAIL{2}else{0}}}
#[test]fn registration_reply_retains_exact_identity_on_full_mailbox(){unsafe{reset();REG_REPLIES.clear();REG_REPLY_FAIL=true;APP_CUSTODY[0].as_mut().unwrap().registration_reply=true;shell_registration_reply_poll();assert!(APP_CUSTODY[0].unwrap().registration_reply);assert_eq!(REG_REPLIES,vec![(15,300)]);REG_REPLY_FAIL=false;shell_registration_reply_poll();assert!(!APP_CUSTODY[0].unwrap().registration_reply);shell_registration_reply_poll();assert_eq!(REG_REPLIES,vec![(15,300),(15,300)]);}}
#[test]fn terminal_registration_never_replies_to_reclaimed_client(){unsafe{reset();APP_CUSTODY[0].as_mut().unwrap().registration_reply=true;shell_lifecycle_hint(&notice(0,15,80));REG_REPLIES.clear();shell_registration_reply_poll();assert!(REG_REPLIES.is_empty());}}

#[test]fn quil_controls_require_exact_live_catalog_subject_and_arguments(){for target in[9,16]{let mut e=app_custody::Entry::new(38,target,target,if target==9{156}else{301});for op in[0x274,0x275,0x279]{assert!(e.allows_operation(op,[target as u64,target as u64,0]));for args in[[12,target as u64,0],[target as u64,99,0],[target as u64,target as u64,1]]{assert!(!e.allows_operation(op,args));}e.close_pending=true;assert!(!e.allows_operation(op,[target as u64,target as u64,0]));e.close_pending=false;}e.dead=true;assert!(!e.allows_operation(0x274,[target as u64,target as u64,0]));}assert!(!app_custody::Entry::new(39,12,12,154).allows_operation(0x274,[12,12,0]));}

#[test]
fn browser_ready_is_exact_geometry_intent_without_document_control_authority() {
 let mut e=app_custody::Entry::new(40,15,150,300);
 assert!(e.allows_operation(0x275,[15,150,0]));
 for args in [[15,15,0],[16,150,0],[15,0,0],[15,150,1]] { assert!(!e.allows_operation(0x275,args)); }
 for op in [0x274,0x279,0x273,0x202] { assert!(!e.allows_operation(op,[15,150,0])); }
 e.close_pending=true;assert!(!e.allows_operation(0x275,[15,150,0]));
 e.close_pending=false;e.dead=true;assert!(!e.allows_operation(0x275,[15,150,0]));
}
