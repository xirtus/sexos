#![allow(dead_code,static_mut_refs)]
#[path="../servers/silk-shell/src/launcher.rs"] mod launcher;
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut INFO:Result<sex_pdx::app_lifecycle::ActivationInfo,u64>=Err(1);
static mut ORIGINAL:(bool,&str,&str)=(true,"RUNNING / FOCUS","original_incarnation_live");
static mut SUCCESSOR:(bool,&str,&str)=(true,"RUNNING / FOCUS","original_incarnation_live");
static mut FOCUSED_SURFACE_ID:u64=201;
const SURFACE_ID_QUIL:u64=201;const SLOT_QUIL:u64=12;
mod sex_pdx{pub mod app_lifecycle{
 pub const ACTIVATION_SLOT:usize=44;pub const DENIED:u64=3;pub const SUCCESSOR_ROUTE_SLOT:u64=47;
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct ActivationInfo{pub successor:u32,pub task:u32,pub surface:u64,pub state:u64}
 pub fn activation(_:usize,_:bool)->Result<ActivationInfo,u64>{unsafe{crate::INFO}}
}}
fn native_launcher_status(t:u32)->(bool,&'static str,&'static str){unsafe{if t==9{ORIGINAL}else{assert_eq!(t,16);SUCCESSOR}}}
include!(env!("QUIL_ACTIVATION_PRODUCTION"));
unsafe fn info(state:u64){INFO=Ok(sex_pdx::app_lifecycle::ActivationInfo{successor:16,task:16,surface:301,state});}
#[test]fn immutable_tuple_and_state_must_match(){let _l=LOCK.lock().unwrap();unsafe{info(1);assert!(quil_activation_info(false).is_ok());
 for i in [sex_pdx::app_lifecycle::ActivationInfo{successor:9,task:16,surface:301,state:1},sex_pdx::app_lifecycle::ActivationInfo{successor:16,task:17,surface:301,state:1},sex_pdx::app_lifecycle::ActivationInfo{successor:16,task:16,surface:156,state:1},sex_pdx::app_lifecycle::ActivationInfo{successor:16,task:16,surface:301,state:4}]{INFO=Ok(i);assert_eq!(quil_activation_info(true),Err(3));}}}
#[test]fn only_ready_reservation_can_offer_reopen(){let _l=LOCK.lock().unwrap();unsafe{info(1);assert_eq!(quil_launcher_status(),(true,"REOPEN / RESERVED INSTANCE","boot_successor_ready"));info(0);ORIGINAL=(false,"closed","no_boot_successor");assert_eq!(quil_launcher_status().2,"predecessor_cleanup_pending");}}
#[test]fn active_uses_new_lifecycle_and_exhaustion_stays_disabled(){let _l=LOCK.lock().unwrap();unsafe{info(2);SUCCESSOR=(false,"CLOSING / PLEASE WAIT","incarnation_closing");assert_eq!(quil_launcher_status(),SUCCESSOR);info(3);assert_eq!(quil_launcher_status(),(false,"CLOSED / RESERVE EXHAUSTED","boot_reserve_exhausted"));}}
#[test]fn activation_errors_never_fall_back_to_old_live_identity(){let _l=LOCK.lock().unwrap();unsafe{INFO=Err(2);ORIGINAL=(true,"running","live");assert!(!quil_launcher_status().0);assert_eq!(quil_route_slot(),SLOT_QUIL);FOCUSED_SURFACE_ID=301;assert!(!quil_has_focus());}}
#[test]fn successor_input_route_requires_active_receipt(){let _l=LOCK.lock().unwrap();unsafe{FOCUSED_SURFACE_ID=301;for state in [0,1,3]{info(state);assert!(!quil_has_focus());assert_eq!(quil_route_slot(),SLOT_QUIL);}info(2);assert!(quil_has_focus());assert_eq!(quil_route_slot(),47);}}
