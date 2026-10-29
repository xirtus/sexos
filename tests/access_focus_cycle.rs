//! Actual semantic focus traversal, with authority/scene/focus gates modeled.
#![allow(static_mut_refs,dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_record{($($x:tt)*)=>{{let _=format_args!($($x)*);}};}
const MAX_ACCESS_NODES:usize=64;
const ACCESS_VISIBLE:u32=1;
const ACT_FOCUS:u32=1;
#[derive(Clone,Copy)]struct AccessTargetRef{surface_id:u64}
#[derive(Clone,Copy)]struct AccessNode{target:AccessTargetRef,state:u32,actions:u32}
static mut FOCUSED_SURFACE_ID:u64=0;
static mut ACTIVE_SCENE_IDX:u8=0;
static mut DEAD:Vec<u64>=Vec::new();
static mut TOMBSTONED:Vec<u64>=Vec::new();
static mut HIDDEN:Vec<u64>=Vec::new();
static mut INACTIVE:Vec<u64>=Vec::new();
static mut DENY:Vec<u64>=Vec::new();
static mut ATTEMPTS:Vec<u64>=Vec::new();
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
fn surface_is_alive(sid:u64)->bool{unsafe{!DEAD.contains(&sid)}}
unsafe fn is_tombstoned(sid:u64)->bool{TOMBSTONED.contains(&sid)}
unsafe fn surface_is_lifecycle_focusable(sid:u64)->bool{!HIDDEN.contains(&sid)}
unsafe fn surface_scene_id(sid:u64)->Option<u8>{Some(if INACTIVE.contains(&sid){1}else{0})}
unsafe fn try_set_focus(sid:u64)->bool{ATTEMPTS.push(sid);if DENY.contains(&sid){false}else{FOCUSED_SURFACE_ID=sid;true}}
include!(env!("ACCESS_FOCUS_SOURCE"));
fn node(sid:u64)->Option<AccessNode>{Some(AccessNode{target:AccessTargetRef{surface_id:sid},state:ACCESS_VISIBLE,actions:ACT_FOCUS})}
unsafe fn reset(current:u64){FOCUSED_SURFACE_ID=current;ACTIVE_SCENE_IDX=0;DEAD.clear();TOMBSTONED.clear();HIDDEN.clear();INACTIVE.clear();DENY.clear();ATTEMPTS.clear();}
#[test]fn duplicate_linen_annotation_cannot_trap_fresh_quil_before_wrap(){let _g=LOCK.lock().unwrap();unsafe{
 for fresh in [301,307]{reset(fresh);let nodes=[node(fresh),node(200),node(300),node(200)];
  for expected in [200,300,fresh,200,300,fresh]{assert!(access_move_focus(&nodes,true));assert_eq!(FOCUSED_SURFACE_ID,expected);}
  for expected in [300,200,fresh]{assert!(access_move_focus(&nodes,false));assert_eq!(FOCUSED_SURFACE_ID,expected);}
 }
}}
#[test]fn hidden_inactive_terminal_and_nonfocus_nodes_are_excluded(){let _g=LOCK.lock().unwrap();unsafe{
 reset(301);DEAD.push(201);TOMBSTONED.push(156);HIDDEN.push(153);INACTIVE.push(306);
 let mut invisible=node(202).unwrap();invisible.state=0;let mut no_action=node(203).unwrap();no_action.actions=0;
 let nodes=[node(0),node(201),node(156),node(153),node(306),Some(invisible),Some(no_action),node(301),node(200)];
 assert!(access_move_focus(&nodes,true));assert_eq!(ATTEMPTS,vec![200]);
}}
#[test]fn final_focus_denial_skips_to_another_candidate_without_authorizing_it(){let _g=LOCK.lock().unwrap();unsafe{
 reset(301);DENY.push(200);assert!(access_move_focus(&[node(301),node(200),node(300)],true));assert_eq!(ATTEMPTS,vec![200,300]);assert_eq!(FOCUSED_SURFACE_ID,300);
}}
#[test]fn absent_current_starts_at_first_or_last_unique_target(){let _g=LOCK.lock().unwrap();unsafe{
 reset(999);let nodes=[node(301),node(200),node(300),node(200)];assert!(access_move_focus(&nodes,true));assert_eq!(FOCUSED_SURFACE_ID,301);
 reset(999);assert!(access_move_focus(&nodes,false));assert_eq!(FOCUSED_SURFACE_ID,300);
}}
#[test]fn empty_or_all_denied_keeps_current_focus_and_bounds_attempts(){let _g=LOCK.lock().unwrap();unsafe{
 reset(301);assert!(!access_move_focus(&[],true));assert!(ATTEMPTS.is_empty());
 DENY.extend([301,200]);assert!(!access_move_focus(&[node(301),node(200),node(200)],true));assert_eq!(ATTEMPTS,vec![200,301]);assert_eq!(FOCUSED_SURFACE_ID,301);
}}
