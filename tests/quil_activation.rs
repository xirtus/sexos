#![allow(dead_code,static_mut_refs)]
#[path="../servers/silk-shell/src/launcher.rs"] mod launcher;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
use sex_pdx::app_lifecycle as a;
static mut INFO:[Result<a::ActivationInfo,u64>;2]=[Err(1);2];
static mut APP_CUSTODY:[Option<app_custody::Entry>;2]=[
 Some(app_custody::Entry::new(45,16,160,301)),Some(app_custody::Entry::new(54,18,180,307))];
static mut ORIGINAL:(bool,&str,&str)=(true,"RUNNING / FOCUS","original_incarnation_live");
static mut SUCCESSOR:(bool,&str,&str)=(true,"RUNNING / FOCUS","original_incarnation_live");
static mut FOCUSED_SURFACE_ID:u64=201;
const SURFACE_ID_QUIL:u64=201;const SLOT_QUIL:u64=12;
mod sex_pdx{pub mod app_lifecycle{
 pub const ACTIVATION_SLOT:usize=44;pub const DENIED:u64=3;pub const SUCCESSOR_ROUTE_SLOT:u64=47;
 pub const QUIL_THIRD_ACTIVATION_SLOT:usize=55;pub const QUIL_THIRD_LIFECYCLE_SLOT:usize=54;
 pub const QUIL_THIRD_ROUTE_SLOT:u64=57;pub const ACTIVATION_EXHAUSTED:u64=3;
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct ActivationInfo{pub successor:u32,pub task:u32,pub surface:u64,pub state:u64}
 pub fn activation(slot:usize,_:bool)->Result<ActivationInfo,u64>{unsafe{crate::INFO[match slot{44=>0,55=>1,_=>panic!()}]}}
}}
fn native_launcher_status(t:u32)->(bool,&'static str,&'static str){unsafe{if t==9{ORIGINAL}else{assert!(matches!(t,16|18));SUCCESSOR}}}
include!(env!("QUIL_ACTIVATION_PRODUCTION"));
unsafe fn info(state:u64){INFO=[Ok(a::ActivationInfo{successor:16,task:160,surface:301,state}),Ok(a::ActivationInfo{successor:18,task:180,surface:307,state:0})];}
#[test]fn immutable_tuple_and_state_must_match(){let _l=LOCK.lock().unwrap();unsafe{info(1);assert!(quil_activation_info(false).is_ok());
 for i in [a::ActivationInfo{successor:9,task:160,surface:301,state:1},a::ActivationInfo{successor:16,task:16,surface:301,state:1},a::ActivationInfo{successor:16,task:160,surface:156,state:1},a::ActivationInfo{successor:16,task:160,surface:301,state:4}]{INFO[0]=Ok(i);assert_eq!(quil_activation_info(true),Err(3));}}}
#[test]fn only_ready_reservation_can_offer_reopen(){let _l=LOCK.lock().unwrap();unsafe{info(1);assert_eq!(quil_launcher_status(),(true,"REOPEN / RESERVED INSTANCE","boot_successor_ready"));info(0);ORIGINAL=(false,"closed","no_boot_successor");assert_eq!(quil_launcher_status().2,"predecessor_cleanup_pending");}}
#[test]fn active_uses_exact_lifecycle_and_final_exhaustion_stays_disabled(){let _l=LOCK.lock().unwrap();unsafe{info(2);SUCCESSOR=(false,"CLOSING / PLEASE WAIT","incarnation_closing");assert_eq!(quil_launcher_status(),SUCCESSOR);INFO[0].as_mut().unwrap().state=3;INFO[1].as_mut().unwrap().state=3;assert_eq!(quil_launcher_status(),(false,"CLOSED / RESERVE EXHAUSTED","boot_reserve_exhausted"));}}
#[test]fn activation_errors_never_skip_grants_or_fall_back(){let _l=LOCK.lock().unwrap();unsafe{info(0);INFO[0]=Err(2);INFO[1].as_mut().unwrap().state=1;ORIGINAL=(true,"running","live");assert!(!quil_launcher_status().0);assert_eq!(quil_route_slot(),SLOT_QUIL);FOCUSED_SURFACE_ID=301;assert!(!quil_has_focus());info(3);INFO[1]=Err(2);assert!(!quil_launcher_status().0);}}
#[test]fn successor_input_route_requires_active_receipt(){let _l=LOCK.lock().unwrap();unsafe{FOCUSED_SURFACE_ID=301;for state in [0,1,3]{info(state);assert!(!quil_has_focus());assert_eq!(quil_route_slot(),SLOT_QUIL);}info(2);assert!(quil_has_focus());assert_eq!(quil_route_slot(),47);}}
#[test]fn third_reserve_is_reachable_only_after_second_terminal(){let _l=LOCK.lock().unwrap();unsafe{info(0);INFO[1].as_mut().unwrap().state=1;assert_eq!(quil_activation_info(false).unwrap().successor,16);info(3);INFO[1].as_mut().unwrap().state=1;assert!(quil_launcher_status().0);assert_eq!(quil_activation_info(true).unwrap().task,180);INFO[1].as_mut().unwrap().state=2;FOCUSED_SURFACE_ID=307;assert!(quil_has_focus());assert_eq!(quil_route_slot(),57);FOCUSED_SURFACE_ID=301;assert!(!quil_has_focus());}}
#[test]fn third_catalog_task_cannot_be_inferred_from_domain(){let _l=LOCK.lock().unwrap();unsafe{info(3);INFO[1].as_mut().unwrap().state=1;INFO[1].as_mut().unwrap().task=18;assert_eq!(quil_activation_info(true),Err(3));info(3);INFO[1].as_mut().unwrap().state=4;assert_eq!(quil_activation_info(true),Err(3));}}
