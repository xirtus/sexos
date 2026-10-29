#![allow(dead_code,static_mut_refs)]
#[path="../servers/silk-shell/src/launcher.rs"] mod launcher;
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut INFO:Result<sex_pdx::app_lifecycle::ActivationInfo,u64>=Err(1);
static mut FOCUSED_SURFACE_ID:u64=306;
const SURFACE_ID_SPINDLE:u64=153;const SLOT_SPINDLE:u64=14;
mod sex_pdx{pub mod app_lifecycle{
 pub const SPINDLE_ACTIVATION_SLOT:usize=49;pub const DENIED:u64=3;pub const SPINDLE_ROUTE_SLOT:u64=52;
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct ActivationInfo{pub successor:u32,pub task:u32,pub surface:u64,pub state:u64}
 pub fn activation(slot:usize,_:bool)->Result<ActivationInfo,u64>{assert_eq!(slot,49);unsafe{crate::INFO}}
}}
fn native_launcher_status(t:u32)->(bool,&'static str,&'static str){match t {12=>(false,"closed","no_boot_successor"),17=>(true,"RUNNING / FOCUS","original_incarnation_live"),_=>panic!("wrong logical target")}}
include!(env!("SPINDLE_ACTIVATION_PRODUCTION"));
#[test]fn native_printable_keys_reach_spindle_without_stealing_launcher_or_function_keys(){
 for code in 0..=127 {
  if let Some(ch)=scancode_to_ascii(code){
   if ch!=b'`'{assert!(is_spindle_text_key(code),"native ASCII {ch} scancode {code} swallowed");}
  }
 }
 for code in [0x29,0x3B,0x42,0x57,0x58]{assert!(!is_spindle_text_key(code));}
}
unsafe fn info(state:u64){INFO=Ok(sex_pdx::app_lifecycle::ActivationInfo{successor:17,task:17,surface:302,state});}
#[test]fn no_quil_or_stale_tuple_can_resolve_as_spindle(){let _l=LOCK.lock().unwrap();unsafe{
 for i in [(16,16,301,2),(17,12,302,2),(12,17,302,2),(17,17,154,2),(17,17,302,4)]{
  INFO=Ok(sex_pdx::app_lifecycle::ActivationInfo{successor:i.0,task:i.1,surface:i.2,state:i.3});assert_eq!(spindle_activation_info(true),Err(3));assert!(!spindle_launcher_status().0);
 }}}
#[test]fn availability_tracks_cleanup_ready_active_and_permanent_exhaustion(){let _l=LOCK.lock().unwrap();unsafe{
 info(0);assert_eq!(spindle_launcher_status().2,"predecessor_cleanup_pending");
 info(1);assert_eq!(spindle_launcher_status(),(true,"REOPEN / RESERVED INSTANCE","boot_successor_ready"));
 info(2);assert_eq!(spindle_launcher_status().2,"original_incarnation_live");
 info(3);assert_eq!(spindle_launcher_status(),(false,"CLOSED / RESERVE EXHAUSTED","boot_reserve_exhausted"));
 }}
#[test]fn new_frame_input_requires_only_its_active_receipt(){let _l=LOCK.lock().unwrap();unsafe{
 FOCUSED_SURFACE_ID=306;for state in [0,1,3]{info(state);assert!(!spindle_has_focus());assert_eq!(spindle_route_slot(),14);}
 info(2);assert!(spindle_has_focus());assert_eq!(spindle_route_slot(),52);
 FOCUSED_SURFACE_ID=301;assert!(!spindle_has_focus());
 }}
#[test]fn unavailable_or_retry_authority_never_enables_execution(){let _l=LOCK.lock().unwrap();unsafe{
 for code in [1,2,3,u64::MAX]{INFO=Err(code);FOCUSED_SURFACE_ID=306;assert!(!spindle_launcher_status().0);assert!(!spindle_has_focus());}
 }}
