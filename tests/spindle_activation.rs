#![allow(dead_code,static_mut_refs)]
#[path="../servers/silk-shell/src/launcher.rs"] mod launcher;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
#[path="../crates/sex-pdx/src/app_catalog.rs"] mod app_catalog;
use sex_pdx::app_lifecycle as a;
// Independent held-query fixtures use unequal PD/task IDs. This does not claim
// that the current BOOT allocator produces those unequal task IDs natively.
static mut APP_CUSTODY:[Option<app_custody::Entry>;9]=[
 Some(app_custody::Entry::new(39,12,120,154)),
 Some(app_custody::Entry::new(50,17,170,302)),
 Some(app_custody::Entry::new(130,24,240,313)),
 Some(app_custody::Entry::new(134,25,250,318)),
 Some(app_custody::Entry::new(138,26,260,323)),
 Some(app_custody::Entry::new(142,27,270,328)),
 Some(app_custody::Entry::new(146,28,280,333)),
 Some(app_custody::Entry::new(150,29,290,338)),
 Some(app_custody::Entry::new(194,40,400,353)),
];
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut INFO:[Result<a::ActivationInfo,u64>;8]=[Err(1);8];
static mut COMMIT:[Result<a::ActivationInfo,u64>;8]=[Err(2);8];
static mut CALLS:Vec<(usize,bool)>=Vec::new();
static mut FOCUSED_SURFACE_ID:u64=306;
const SURFACE_ID_SPINDLE:u64=153;const SLOT_SPINDLE:u64=14;
mod sex_pdx{pub mod app_lifecycle{
 pub const SPINDLE_ACTIVATION_SLOT:usize=49;pub const SPINDLE_LIFECYCLE_SLOT:usize=50;pub const DENIED:u64=3;pub const SPINDLE_ROUTE_SLOT:u64=52;
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct ActivationInfo{pub successor:u32,pub task:u32,pub surface:u64,pub state:u64}
 pub fn activation(slot:usize,commit:bool)->Result<ActivationInfo,u64>{unsafe{
  crate::CALLS.push((slot,commit));let index=match slot{49=>0,132=>1,136=>2,140=>3,144=>4,148=>5,152=>6,196=>7,_=>panic!("unexpected grant {slot}")};
  // Scripted syscall receipts: query and commit are distinct. Successful commit
  // publishes its returned state for later queries; a retry leaves it unchanged.
  if commit{let result=crate::COMMIT[index];if result.is_ok(){crate::INFO[index]=result;}result}else{crate::INFO[index]}
 }}
}}
fn native_launcher_status(t:u32)->(bool,&'static str,&'static str){
 if t==12{return(false,"closed","no_boot_successor")}
 let index=[17,24,25,26,27,28,29,40].iter().position(|&v|v==t).expect("wrong logical target");
 unsafe{if INFO[index].is_ok_and(|i|i.state==3){(false,"closed","no_boot_successor")}else{(true,"RUNNING / FOCUS","original_incarnation_live")}}
}
include!(env!("SPINDLE_ACTIVATION_PRODUCTION"));
unsafe fn info(state:u64){
 APP_CUSTODY=[Some(app_custody::Entry::new(39,12,120,154)),Some(app_custody::Entry::new(50,17,170,302)),Some(app_custody::Entry::new(130,24,240,313)),Some(app_custody::Entry::new(134,25,250,318)),Some(app_custody::Entry::new(138,26,260,323)),Some(app_custody::Entry::new(142,27,270,328)),Some(app_custody::Entry::new(146,28,280,333)),Some(app_custody::Entry::new(150,29,290,338)),Some(app_custody::Entry::new(194,40,400,353))];
 INFO=[Ok(a::ActivationInfo{successor:17,task:170,surface:302,state}),Ok(a::ActivationInfo{successor:24,task:240,surface:313,state:0}),Ok(a::ActivationInfo{successor:25,task:250,surface:318,state:0}),Ok(a::ActivationInfo{successor:26,task:260,surface:323,state:0}),Ok(a::ActivationInfo{successor:27,task:270,surface:328,state:0}),Ok(a::ActivationInfo{successor:28,task:280,surface:333,state:0}),Ok(a::ActivationInfo{successor:29,task:290,surface:338,state:0}),Ok(a::ActivationInfo{successor:40,task:400,surface:353,state:0})];
 COMMIT=[Err(2);8];CALLS.clear();FOCUSED_SURFACE_ID=306;
}
#[test]fn native_printable_keys_reach_spindle_without_stealing_launcher_or_function_keys(){
 for code in 0..=127 {if let Some(ch)=scancode_to_ascii(code){if ch!=b'`'{assert!(is_spindle_text_key(code),"native ASCII {ch} scancode {code} swallowed");}}}
 for code in [0x29,0x3B,0x42,0x57,0x58]{assert!(!is_spindle_text_key(code));}
}
#[test]fn no_quil_or_stale_tuple_can_resolve_as_spindle(){let _l=LOCK.lock().unwrap();unsafe{
 for i in [(16,16,301,2),(17,12,302,2),(12,17,302,2),(17,170,154,2),(17,170,302,4),(17,17,302,2)]{
  info(0);INFO[0]=Ok(a::ActivationInfo{successor:i.0,task:i.1,surface:i.2,state:i.3});assert_eq!(spindle_activation_info(true),Err(3));assert!(!spindle_launcher_status().0);
  assert!(CALLS.iter().all(|&(slot,commit)|slot==49&&!commit));
 }}}
#[test]fn availability_tracks_cleanup_ready_active_and_permanent_exhaustion(){let _l=LOCK.lock().unwrap();unsafe{
 info(0);assert_eq!(spindle_launcher_status().2,"predecessor_cleanup_pending");
 info(1);assert_eq!(spindle_launcher_status(),(true,"REOPEN / RESERVED INSTANCE","boot_successor_ready"));
 info(2);assert_eq!(spindle_launcher_status().2,"original_incarnation_live");
 info(3);assert_eq!(spindle_launcher_status().2,"predecessor_cleanup_pending"); // third reserve remains, but is not ready
 for entry in &mut INFO{entry.as_mut().unwrap().state=3;}assert_eq!(spindle_launcher_status(),(false,"CLOSED / RESERVE EXHAUSTED","boot_reserve_exhausted"));
 }}
#[test]fn new_frame_input_requires_only_its_active_receipt(){let _l=LOCK.lock().unwrap();unsafe{
 for state in [0,1,3]{info(state);assert!(!spindle_has_focus());assert_eq!(spindle_route_slot(),14);}
 info(2);assert!(spindle_has_focus());assert_eq!(spindle_active_frame(),Some((17,306)));assert_eq!(spindle_route_slot(),52);
 FOCUSED_SURFACE_ID=301;assert!(!spindle_has_focus());
 }}
#[test]fn third_edge_waits_for_terminal_predecessor_and_cleanup_then_commits_only_132(){let _l=LOCK.lock().unwrap();unsafe{
 assert_eq!(app_catalog::COUNT,28);assert_eq!(app_catalog::SPINDLE_TARGETS,[12,17,24,25,26,27,28,29,40]);
 for state in [0,1,2]{info(state);INFO[1].as_mut().unwrap().state=1;
  assert_eq!(spindle_activation_info(false).unwrap().successor,17);assert_eq!(CALLS,[(49,false)]);
 }
 info(3);assert_eq!(spindle_launcher_status().2,"predecessor_cleanup_pending");
 assert_eq!(spindle_activation_info(true),Err(2)); // terminal17 alone does not establish cleanup/ready24
 assert_eq!(INFO[1].unwrap().state,0);assert!(!CALLS.contains(&(49,true)));
 INFO[1].as_mut().unwrap().state=1;CALLS.clear();
 assert_eq!(spindle_launcher_status().2,"boot_successor_ready");
 COMMIT[1]=Ok(a::ActivationInfo{successor:24,task:240,surface:313,state:2});CALLS.clear();
 assert_eq!(spindle_activation_info(true),COMMIT[1]);assert_eq!(CALLS,[(49,false),(132,false),(132,true)]);
 assert_eq!(spindle_active_frame(),Some((24,314)));FOCUSED_SURFACE_ID=314;
 assert!(spindle_has_focus());assert_eq!(spindle_route_slot(),133);
 for focus in [306,313,315,301]{FOCUSED_SURFACE_ID=focus;assert!(!spindle_has_focus());}
 INFO[1].as_mut().unwrap().state=3;assert!(spindle_active_frame().is_none());assert_eq!(spindle_route_slot(),14);
 assert_eq!(spindle_launcher_status().2,"predecessor_cleanup_pending");
 }}
#[test]fn third_edge_rejects_wrong_task_surface_lifecycle_and_missing_held_subject(){let _l=LOCK.lock().unwrap();unsafe{
 for i in [(24,24,313,2),(24,170,313,2),(17,240,313,2),(24,240,302,2),(24,240,314,2),(24,240,313,4)]{
  info(3);INFO[1]=Ok(a::ActivationInfo{successor:i.0,task:i.1,surface:i.2,state:i.3});
  assert_eq!(spindle_activation_info(true),Err(3));assert!(!CALLS.iter().any(|&(_,commit)|commit));
  FOCUSED_SURFACE_ID=314;assert!(!spindle_has_focus());assert_eq!(spindle_route_slot(),14);assert!(!spindle_launcher_status().0);
 }
 for stale in 0..4{info(3);INFO[1].as_mut().unwrap().state=2;
  match stale{0=>APP_CUSTODY[2]=None,1=>APP_CUSTODY[2].as_mut().unwrap().slot=50,
   2=>APP_CUSTODY[2].as_mut().unwrap().task=24,_=>APP_CUSTODY[2].as_mut().unwrap().surface=302}
  assert_eq!(spindle_activation_info(false),Err(3));assert!(spindle_active_frame().is_none());assert_eq!(spindle_route_slot(),14);
 }
 }}
#[test]fn unavailable_or_retry_authority_never_skips_an_edge_or_enables_execution(){let _l=LOCK.lock().unwrap();unsafe{
 for edge in 0..2{for code in [1,2,3,u64::MAX]{info(if edge==0{0}else{3});INFO[edge]=Err(code);FOCUSED_SURFACE_ID=if edge==0{306}else{314};
  assert_eq!(spindle_activation_info(true),Err(code));assert!(!spindle_launcher_status().0);assert!(!spindle_has_focus());assert_eq!(spindle_route_slot(),14);
  assert!(!CALLS.iter().any(|&(_,commit)|commit));if edge==0{assert!(CALLS.iter().all(|&(slot,_)|slot==49));}
 }}
 info(3);INFO[1].as_mut().unwrap().state=1;
 for code in [2,3,u64::MAX]{COMMIT[1]=Err(code);assert_eq!(spindle_activation_info(true),Err(code));assert_eq!(INFO[1].unwrap().state,1);assert_eq!(spindle_route_slot(),14);}
 }}
#[test]fn reserve_observations_cover_all_eight_grants_without_activating_or_accepting_early_third_state(){let _l=LOCK.lock().unwrap();unsafe{
 info(0);assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),Some(8));assert_eq!(CALLS,[(49,false),(132,false),(136,false),(140,false),(144,false),(148,false),(152,false),(196,false)]);
 info(2);assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),Some(7));
 INFO[1].as_mut().unwrap().state=1;assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),None);
 info(3);INFO[1].as_mut().unwrap().state=1;assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),Some(7));
 INFO[1].as_mut().unwrap().state=2;assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),Some(6));
 INFO[1]=Err(2);assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),None);
 assert!(!CALLS.iter().any(|&(_,commit)|commit));
 }}

#[test]fn full_nine_chain_advances_only_exact_ready_edges_and_exhausts_after_40(){let _l=LOCK.lock().unwrap();unsafe{
 info(0);
 let edges=[(49,17,170,302,306,52),(132,24,240,313,314,133),(136,25,250,318,319,137),(140,26,260,323,324,141),(144,27,270,328,329,145),(148,28,280,333,334,149),(152,29,290,338,339,153),(196,40,400,353,354,197)];
 for (index,&(slot,target,task,surface,frame,route)) in edges.iter().enumerate(){
  assert_eq!(spindle_launcher_status().2,"predecessor_cleanup_pending");
  assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),Some((8-index)as u8));
  INFO[index].as_mut().unwrap().state=1;
  assert_eq!(spindle_launcher_status().2,"boot_successor_ready");
  for code in [1,2,3,u64::MAX]{COMMIT[index]=Err(code);CALLS.clear();assert_eq!(spindle_activation_info(true),Err(code));assert_eq!(CALLS.last(),Some(&(slot,true)));assert_eq!(INFO[index].unwrap().state,1);assert_eq!(spindle_route_slot(),14);}
  COMMIT[index]=Ok(a::ActivationInfo{successor:target,task,surface,state:2});CALLS.clear();
  assert_eq!(spindle_activation_info(true),COMMIT[index]);
  let mut expected:Vec<_>=edges[..=index].iter().map(|e|(e.0,false)).collect();expected.push((slot,true));assert_eq!(CALLS,expected);
  FOCUSED_SURFACE_ID=frame;assert!(spindle_has_focus());assert_eq!(spindle_active_frame(),Some((target,frame)));assert_eq!(spindle_route_slot(),route);
  for wrong in [surface,frame|(1u64<<32),301]{FOCUSED_SURFACE_ID=wrong;assert!(!spindle_has_focus());}
  INFO[index].as_mut().unwrap().state=3;
  assert!(spindle_active_frame().is_none());assert_eq!(spindle_route_slot(),14);
  assert_eq!(spindle_launcher_status().2,if index==7{"boot_reserve_exhausted"}else{"predecessor_cleanup_pending"});
 }
 assert_eq!(app_reserve_remaining(app_catalog::App::Spindle),Some(0));
}}
#[test]fn eighth_edge_rejects_missing_stale_and_wrong_task_without_skipping_or_focus(){let _l=LOCK.lock().unwrap();unsafe{
 for bad in 0..8{
  info(3);for entry in &mut INFO[..6]{entry.as_mut().unwrap().state=3;}INFO[6].as_mut().unwrap().state=2;FOCUSED_SURFACE_ID=339;
  match bad{0=>INFO[6].as_mut().unwrap().task=29,1=>INFO[6].as_mut().unwrap().surface=333,
   2=>INFO[6].as_mut().unwrap().successor=28,3=>INFO[6].as_mut().unwrap().state=4,
   4=>APP_CUSTODY[7]=None,5=>APP_CUSTODY[7].as_mut().unwrap().slot=146,
   6=>APP_CUSTODY[7].as_mut().unwrap().task=280,_=>APP_CUSTODY[7].as_mut().unwrap().surface=333}
  assert_eq!(spindle_activation_info(true),Err(3));assert!(!spindle_has_focus());assert_eq!(spindle_route_slot(),14);assert!(!CALLS.iter().any(|c|c.1));
 }
 for edge in 0..8{for code in [1,2,3,u64::MAX]{
  info(0);for entry in &mut INFO[..edge]{entry.as_mut().unwrap().state=3;}INFO[edge]=Err(code);CALLS.clear();
  assert_eq!(spindle_activation_info(true),Err(code));assert_eq!(CALLS.len(),edge+1);assert!(!CALLS.iter().any(|c|c.1));
 }}
}}

#[test]fn ninth_edge_rejects_missing_stale_and_wrong_task_without_skipping_or_focus(){let _l=LOCK.lock().unwrap();unsafe{
 for bad in 0..8{
  info(3);for entry in &mut INFO[..7]{entry.as_mut().unwrap().state=3;}INFO[7].as_mut().unwrap().state=2;FOCUSED_SURFACE_ID=354;
  match bad{0=>INFO[7].as_mut().unwrap().task=40,1=>INFO[7].as_mut().unwrap().surface=338,
   2=>INFO[7].as_mut().unwrap().successor=29,3=>INFO[7].as_mut().unwrap().state=4,
   4=>APP_CUSTODY[8]=None,5=>APP_CUSTODY[8].as_mut().unwrap().slot=150,
   6=>APP_CUSTODY[8].as_mut().unwrap().task=290,_=>APP_CUSTODY[8].as_mut().unwrap().surface=338}
  assert_eq!(spindle_activation_info(true),Err(3));assert!(!spindle_has_focus());assert_eq!(spindle_route_slot(),14);assert!(!CALLS.iter().any(|c|c.1));
 }
 for edge in 0..8{for code in [1,2,3,u64::MAX]{
  info(0);for entry in &mut INFO[..edge]{entry.as_mut().unwrap().state=3;}INFO[edge]=Err(code);CALLS.clear();
  assert_eq!(spindle_activation_info(true),Err(code));assert_eq!(CALLS.len(),edge+1);assert!(!CALLS.iter().any(|c|c.1));
 }}
}}
