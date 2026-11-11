// SPDX-License-Identifier: GPL-3.0-only
#![allow(dead_code, static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/desktop_stack.rs"]pub mod desktop_stack;
#[path="../servers/silk-shell/src/app_custody.rs"]mod app_custody;
use desktop_stack::{FrameFacts,InputFacts,input_allowed};

#[test]fn canonical_inventory_is_complete_unique_and_literal_front_order_is_stable(){
 assert_eq!(&desktop_stack::ORDER[..4],&[200,201,153,300]);
 for(i,&sid)in desktop_stack::ORDER.iter().enumerate(){
  assert!(!desktop_stack::ORDER[..i].contains(&sid));
  assert_eq!(desktop_stack::rank(sid),Some(i as u16));
 }
 for row in app_catalog::INCARNATIONS{
  assert!(desktop_stack::ORDER.contains(&row.frame()));
  assert_eq!(desktop_stack::rank(row.surface),desktop_stack::rank(row.frame()));
  for &child in row.children{assert_eq!(desktop_stack::rank(child),desktop_stack::rank(row.frame()));}
 }
 assert_eq!(desktop_stack::rank(u64::MAX),None);
 assert!(desktop_stack::key(201,3,152)<desktop_stack::key(201,3,201));
 assert!(desktop_stack::key(201,3,201)<desktop_stack::key(201,3,200));
 assert!(desktop_stack::key(0,3,200)<desktop_stack::key(0,3,202));
 assert!(desktop_stack::key(0,10,156)>desktop_stack::key(0,3,201));
 assert!(desktop_stack::key(0,8,200)>desktop_stack::key(0,3,202));
 assert!(desktop_stack::key(0,8,800)<desktop_stack::key(0,8,801));
 assert!(desktop_stack::key(201,8,152)>desktop_stack::key(201,3,201));
}
#[test]fn lifecycle_visible_placeholder_is_insufficient_but_owned_live_standalone_remains(){
 let good=InputFacts{registered:true,alive:true,lifecycle_ready:true,in_scene:true,frame:Some(FrameFacts{active_tab:true,accepts_input:true}),standalone_owned:false};
 assert!(input_allowed(good));
 assert!(!input_allowed(InputFacts{frame:None,..good}));
 assert!(!input_allowed(InputFacts{frame:Some(FrameFacts{active_tab:false,accepts_input:true}),..good}));
 assert!(!input_allowed(InputFacts{frame:Some(FrameFacts{active_tab:true,accepts_input:false}),..good}));
 assert!(!input_allowed(InputFacts{registered:false,..good}));
 assert!(!input_allowed(InputFacts{alive:false,..good}));
 assert!(!input_allowed(InputFacts{lifecycle_ready:false,..good}));
 assert!(!input_allowed(InputFacts{in_scene:false,..good}));
 assert!(input_allowed(InputFacts{frame:None,standalone_owned:true,..good}));
 assert!(!input_allowed(InputFacts{frame:None,standalone_owned:true,lifecycle_ready:false,..good}));
}

#[derive(Clone,Copy)]struct Case{sid:u64,registered:bool,alive:bool,lifecycle:bool,scene:bool,owned:bool,frame:Option<(u32,u64,bool)>}
const NONE:Case=Case{sid:0,registered:false,alive:false,lifecycle:false,scene:false,owned:false,frame:None};
static mut CASES:[Case;4]=[NONE;4];
static mut APP_CUSTODY:[Option<app_custody::Entry>;2]=[None;2];
unsafe fn facts(sid:u64)->Case{CASES.iter().find(|case|case.sid==sid).copied().unwrap_or(NONE)}
fn is_focusable_surface(sid:u64)->bool{unsafe{facts(sid).registered}}
fn surface_is_alive(sid:u64)->bool{unsafe{facts(sid).alive}}
unsafe fn surface_is_lifecycle_focusable(sid:u64)->bool{facts(sid).lifecycle}
unsafe fn surface_in_active_scene(sid:u64)->bool{facts(sid).scene}
fn is_shell_surface(sid:u64)->bool{unsafe{facts(sid).owned}}
unsafe fn frame_for_surface(sid:u64)->Option<u32>{facts(sid).frame.map(|frame|frame.0)}
unsafe fn active_surface_for_frame(id:u32)->Option<u64>{CASES.iter().find_map(|case|case.frame.filter(|frame|frame.0==id).map(|frame|frame.1))}
unsafe fn frame_accepts_input(id:u32)->bool{CASES.iter().find_map(|case|case.frame.filter(|frame|frame.0==id).map(|frame|frame.2)).unwrap_or(false)}
include!(env!("SHELL_STACK_SOURCE"));
#[test]fn actual_shell_wrapper_refuses_uncreated_and_inactive_tabs_without_erasing_owned_path(){unsafe{
 let good=Case{sid:201,registered:true,alive:true,lifecycle:true,scene:true,owned:false,frame:Some((3,201,true))};
 CASES=[good,Case{sid:202,frame:None,..good},Case{sid:100,frame:None,owned:true,..good},Case{sid:101,frame:Some((1,100,true)),..good}];
 assert!(input_surface_eligible(201));
 assert!(!input_surface_eligible(202)); // misleading Visible label, no real frame
 assert!(input_surface_eligible(100)); // explicit mapped standalone ownership
 assert!(!input_surface_eligible(101)); // tab exists but another tab is active
 CASES[2].lifecycle=false;assert!(!input_surface_eligible(100));
 CASES[0].scene=false;assert!(!input_surface_eligible(201));
 CASES=[NONE;4];APP_CUSTODY=[None;2];
}}
#[test]fn actual_focus_alias_needs_exact_live_custody_and_real_bound_parent(){unsafe{
 CASES=[Case{sid:201,frame:Some((3,201,true)),..NONE},Case{sid:153,frame:Some((4,153,true)),..NONE},NONE,NONE];
 APP_CUSTODY=[Some(app_custody::Entry::new(9,9,9,156)),Some(app_custody::Entry::new(12,12,12,154))];
 assert_eq!(input_focus_surface(156),201);assert_eq!(input_focus_surface(160),153);assert_eq!(input_focus_surface(161),153);
 APP_CUSTODY[0].as_mut().unwrap().dead=true;assert_eq!(input_focus_surface(156),156);
 APP_CUSTODY[1].as_mut().unwrap().close_pending=true;assert_eq!(input_focus_surface(160),160);
 APP_CUSTODY[0]=Some(app_custody::Entry::new(9,8,8,156));assert_eq!(input_focus_surface(156),156);
 APP_CUSTODY[0]=Some(app_custody::Entry::new(9,9,9,156));CASES[0].frame=None;assert_eq!(input_focus_surface(156),156);
 assert_eq!(input_focus_surface(u64::MAX),u64::MAX);
 CASES=[NONE;4];APP_CUSTODY=[None;2];
}}
