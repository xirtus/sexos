#![allow(dead_code,static_mut_refs)]
#[path="../servers/silk-shell/src/launcher.rs"] mod launcher;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
#[path="../crates/sex-pdx/src/app_catalog.rs"] mod app_catalog;
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
use sex_pdx::app_lifecycle as a;
static mut INFO:[Result<a::ActivationInfo,u64>;15]=[Err(1);15];
static mut APP_CUSTODY:[Option<app_custody::Entry>;15]=[
 Some(app_custody::Entry::new(45,16,160,301)),Some(app_custody::Entry::new(54,18,180,307)),
 Some(app_custody::Entry::new(110,19,190,308)),Some(app_custody::Entry::new(114,20,200,309)),
 Some(app_custody::Entry::new(118,21,210,310)),Some(app_custody::Entry::new(122,22,220,311)),
 Some(app_custody::Entry::new(126,23,230,312)),
 Some(app_custody::Entry::new(162,32,320,345)),Some(app_custody::Entry::new(166,33,330,346)),Some(app_custody::Entry::new(170,34,340,347)),Some(app_custody::Entry::new(174,35,350,348)),Some(app_custody::Entry::new(178,36,360,349)),Some(app_custody::Entry::new(182,37,370,350)),Some(app_custody::Entry::new(186,38,380,351)),Some(app_custody::Entry::new(190,39,390,352))];
static mut COMMIT:[Option<Result<a::ActivationInfo,u64>>;15]=[None;15];
static mut CALLS:Vec<(usize,bool)>=Vec::new();
static mut ORIGINAL:(bool,&str,&str)=(true,"RUNNING / FOCUS","original_incarnation_live");
static mut SUCCESSOR:(bool,&str,&str)=(true,"RUNNING / FOCUS","original_incarnation_live");
static mut FOCUSED_SURFACE_ID:u64=201;
const SURFACE_ID_QUIL:u64=201;const SLOT_QUIL:u64=12;
mod sex_pdx{pub mod app_lifecycle{
 pub const ACTIVATION_SLOT:usize=44;pub const DENIED:u64=3;pub const SUCCESSOR_ROUTE_SLOT:u64=47;
 pub const QUIL_THIRD_ACTIVATION_SLOT:usize=55;pub const QUIL_THIRD_LIFECYCLE_SLOT:usize=54;
 pub const QUIL_THIRD_ROUTE_SLOT:u64=57;pub const ACTIVATION_EXHAUSTED:u64=3;
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct ActivationInfo{pub successor:u32,pub task:u32,pub surface:u64,pub state:u64}
 pub fn activation(slot:usize,commit:bool)->Result<ActivationInfo,u64>{unsafe{crate::CALLS.push((slot,commit));let index=match slot{44=>0,55=>1,112=>2,116=>3,120=>4,124=>5,128=>6,164=>7,168=>8,172=>9,176=>10,180=>11,184=>12,188=>13,192=>14,_=>panic!()};if commit{crate::COMMIT[index].unwrap_or(crate::INFO[index])}else{crate::INFO[index]}}}
}}
fn native_launcher_status(t:u32)->(bool,&'static str,&'static str){unsafe{if t==9{ORIGINAL}else{assert!(matches!(t,16|18|19|20|21|22|23|32|33|34|35|36|37|38|39));SUCCESSOR}}}
include!(env!("QUIL_ACTIVATION_PRODUCTION"));
unsafe fn info(state:u64){
 INFO=[(16,160,301),(18,180,307),(19,190,308),(20,200,309),(21,210,310),(22,220,311),(23,230,312),(32,320,345),(33,330,346),(34,340,347),(35,350,348),(36,360,349),(37,370,350),(38,380,351),(39,390,352)]
  .map(|(successor,task,surface)|Ok(a::ActivationInfo{successor,task,surface,state:0}));
 INFO[0].as_mut().unwrap().state=state;COMMIT=[None;15];CALLS.clear();
 ORIGINAL=(true,"RUNNING / FOCUS","original_incarnation_live");
 SUCCESSOR=ORIGINAL;FOCUSED_SURFACE_ID=201;
}
#[test]fn immutable_tuple_and_state_must_match(){let _l=LOCK.lock().unwrap();unsafe{info(1);assert!(quil_activation_info(false).is_ok());
 for i in [a::ActivationInfo{successor:9,task:160,surface:301,state:1},a::ActivationInfo{successor:16,task:16,surface:301,state:1},a::ActivationInfo{successor:16,task:160,surface:156,state:1},a::ActivationInfo{successor:16,task:160,surface:301,state:4}]{INFO[0]=Ok(i);assert_eq!(quil_activation_info(true),Err(3));}}}
#[test]fn only_ready_reservation_can_offer_reopen(){let _l=LOCK.lock().unwrap();unsafe{info(1);assert_eq!(quil_launcher_status(),(true,"REOPEN / RESERVED INSTANCE","boot_successor_ready"));info(0);ORIGINAL=(false,"closed","no_boot_successor");assert_eq!(quil_launcher_status().2,"predecessor_cleanup_pending");}}
#[test]fn active_uses_exact_lifecycle_and_final_exhaustion_stays_disabled(){let _l=LOCK.lock().unwrap();unsafe{info(2);SUCCESSOR=(false,"CLOSING / PLEASE WAIT","incarnation_closing");assert_eq!(quil_launcher_status(),SUCCESSOR);for i in &mut INFO{i.as_mut().unwrap().state=3;}assert_eq!(quil_launcher_status(),(false,"CLOSED / RESERVE EXHAUSTED","boot_reserve_exhausted"));}}
#[test]fn activation_errors_never_skip_grants_or_fall_back(){let _l=LOCK.lock().unwrap();unsafe{info(0);INFO[0]=Err(2);INFO[1].as_mut().unwrap().state=1;ORIGINAL=(true,"running","live");assert!(!quil_launcher_status().0);assert_eq!(quil_route_slot(),SLOT_QUIL);FOCUSED_SURFACE_ID=301;assert!(!quil_has_focus());info(3);INFO[1]=Err(2);assert!(!quil_launcher_status().0);}}
#[test]fn successor_input_route_requires_active_receipt(){let _l=LOCK.lock().unwrap();unsafe{for state in [0,1,3]{info(state);FOCUSED_SURFACE_ID=301;assert!(!quil_has_focus());assert_eq!(quil_route_slot(),SLOT_QUIL);}info(2);FOCUSED_SURFACE_ID=301;assert!(quil_has_focus());assert_eq!(quil_route_slot(),47);}}
#[test]fn third_reserve_is_reachable_only_after_second_terminal(){let _l=LOCK.lock().unwrap();unsafe{info(0);INFO[1].as_mut().unwrap().state=1;assert_eq!(quil_activation_info(false).unwrap().successor,16);info(3);INFO[1].as_mut().unwrap().state=1;assert!(quil_launcher_status().0);assert_eq!(quil_activation_info(true).unwrap().task,180);INFO[1].as_mut().unwrap().state=2;FOCUSED_SURFACE_ID=307;assert!(quil_has_focus());assert_eq!(quil_route_slot(),57);FOCUSED_SURFACE_ID=301;assert!(!quil_has_focus());}}
#[test]fn third_catalog_task_cannot_be_inferred_from_domain(){let _l=LOCK.lock().unwrap();unsafe{info(3);INFO[1].as_mut().unwrap().state=1;INFO[1].as_mut().unwrap().task=18;assert_eq!(quil_activation_info(true),Err(3));info(3);INFO[1].as_mut().unwrap().state=4;assert_eq!(quil_activation_info(true),Err(3));}}

#[test]fn remaining_reserve_queries_all_fifteen_exact_grants_and_never_enables_action(){let _l=LOCK.lock().unwrap();unsafe{
 info(0);assert_eq!(quil_reserve_remaining(),Some(15));
 info(2);assert_eq!(quil_reserve_remaining(),Some(14));
 info(3);INFO[1].as_mut().unwrap().state=1;assert_eq!(quil_reserve_remaining(),Some(14));
 INFO[1].as_mut().unwrap().state=2;assert_eq!(quil_reserve_remaining(),Some(13));
 INFO[1]=Err(2);assert_eq!(quil_reserve_remaining(),None);
 info(0);INFO[1].as_mut().unwrap().task=18;assert_eq!(quil_reserve_remaining(),None);
 assert_eq!(quil_reserve_label("CLOSING",false,"incarnation_closing"),"CLOSING");
 info(2);INFO[1].as_mut().unwrap().state=1;assert_eq!(quil_reserve_remaining(),None);
}}

#[test]fn full_chain_selects_last_exact_route_and_exhausts_only_after_all_fifteen(){let _l=LOCK.lock().unwrap();unsafe{
 info(0);
 let slots=[44,55,112,116,120,124,128,164,168,172,176,180,184,188,192];
 let targets=[16,18,19,20,21,22,23,32,33,34,35,36,37,38,39];let surfaces=[301,307,308,309,310,311,312,345,346,347,348,349,350,351,352];let routes=[47,57,113,117,121,125,129,165,169,173,177,181,185,189,193];
 for index in 0..15{
  INFO[index].as_mut().unwrap().state=1;
  CALLS.clear();assert_eq!(quil_activation_info(true).unwrap().successor,targets[index]);
  let mut expected:Vec<_>=slots[..=index].iter().map(|slot|(*slot,false)).collect();expected.push((slots[index],true));assert_eq!(CALLS,expected);
  assert_eq!(quil_reserve_remaining(),Some((15-index)as u8));
  INFO[index].as_mut().unwrap().state=2;FOCUSED_SURFACE_ID=surfaces[index];
  assert!(quil_has_focus());assert_eq!(quil_route_slot(),routes[index]);
  INFO[index].as_mut().unwrap().state=3;
 }
 assert_eq!(quil_reserve_remaining(),Some(0));
 assert_eq!(quil_launcher_status(),(false,"CLOSED / RESERVE EXHAUSTED","boot_reserve_exhausted"));
 assert!(!quil_has_focus());assert_eq!(quil_route_slot(),SLOT_QUIL);
}}
#[test]fn middle_error_or_blocked_state_never_skips_to_ready_last_reserve(){let _l=LOCK.lock().unwrap();unsafe{
 for middle in [Err(2),Ok(a::ActivationInfo{successor:20,task:200,surface:309,state:0}),Ok(a::ActivationInfo{successor:20,task:200,surface:309,state:4})]{
  info(0);for i in 0..3{INFO[i].as_mut().unwrap().state=3;}INFO[3]=middle;INFO[6].as_mut().unwrap().state=1;
  let result=quil_activation_info(true);
  if middle.is_ok_and(|i|i.state==0){assert_eq!(result.unwrap().successor,20);}else{assert!(result.is_err());}
  assert!(!CALLS.iter().any(|(slot,_)|[120,124,128].contains(slot)));
  assert_eq!(quil_reserve_remaining(),None);
 }
}}
#[test]fn last_reserve_task_and_surface_cannot_be_inferred_or_borrowed(){let _l=LOCK.lock().unwrap();unsafe{
 for bad in [a::ActivationInfo{successor:23,task:23,surface:312,state:1},a::ActivationInfo{successor:23,task:220,surface:312,state:1},a::ActivationInfo{successor:23,task:230,surface:311,state:1},a::ActivationInfo{successor:23,task:230,surface:312,state:4}]{
  info(0);for i in 0..6{INFO[i].as_mut().unwrap().state=3;}INFO[6]=Ok(bad);
  assert_eq!(quil_activation_info(true),Err(a::DENIED));assert_eq!(quil_reserve_remaining(),None);
  assert!(!CALLS.iter().any(|(_,commit)|*commit));
 }
}}

#[test]fn sixteenth_reserve_task_and_surface_cannot_be_inferred_or_borrowed(){let _l=LOCK.lock().unwrap();unsafe{
 for bad in [a::ActivationInfo{successor:39,task:39,surface:352,state:1},a::ActivationInfo{successor:39,task:380,surface:352,state:1},a::ActivationInfo{successor:39,task:390,surface:351,state:1},a::ActivationInfo{successor:39,task:390,surface:352,state:4}]{
  info(0);for i in 0..14{INFO[i].as_mut().unwrap().state=3;}INFO[14]=Ok(bad);
  assert_eq!(quil_activation_info(true),Err(a::DENIED));assert_eq!(quil_reserve_remaining(),None);
  assert!(!CALLS.iter().any(|(_,commit)|*commit));
 }
}}

#[test]fn every_new_edge_retains_ready_on_full_or_denial_and_rejects_changed_commit_tuple(){let _l=LOCK.lock().unwrap();unsafe{
 for index in 7..15{for code in [1,2,3,u64::MAX]{
  info(0);for entry in &mut INFO[..index]{entry.as_mut().unwrap().state=3;}INFO[index].as_mut().unwrap().state=1;
  COMMIT[index]=Some(Err(code));assert_eq!(quil_activation_info(true),Err(code));assert_eq!(INFO[index].unwrap().state,1);assert_eq!(quil_route_slot(),SLOT_QUIL);
  assert!(!CALLS.iter().any(|(slot,_)|*slot>164+4*(index-7)));
 }
 info(0);for entry in &mut INFO[..index]{entry.as_mut().unwrap().state=3;}INFO[index].as_mut().unwrap().state=1;
 let mut wrong=INFO[index].unwrap();wrong.task=wrong.successor;wrong.state=2;COMMIT[index]=Some(Ok(wrong));assert_eq!(quil_activation_info(true),Err(a::DENIED));assert_eq!(INFO[index].unwrap().state,1);
 }
}}
