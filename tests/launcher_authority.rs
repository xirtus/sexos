#![allow(dead_code,static_mut_refs)]
#[path="../servers/silk-shell/src/launcher.rs"] mod launcher;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;
#[path="../crates/sex-pdx/src/app_catalog.rs"] mod app_catalog;
use launcher::{Evidence,Identity};
static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static mut APP_CUSTODY:[Option<app_custody::Entry>;3]=[None;3];
static mut CATALOG:Option<Identity>=None;
static mut EVIDENCE:Evidence=Evidence::Invalid;
mod sex_pdx {pub mod app_lifecycle {
 pub const LIVE:u64=5;pub const RETRY:u64=2;
 pub struct Catalog {pub target_pd:u32,pub task_id:u32,pub surface_id:u64,pub coordinator_pd:u32}
 pub struct Info {pub target_pd:u32,pub task_id:u32,pub required_mask:u64,pub acked_mask:u64}
 pub fn catalog(slot:usize)->Result<Catalog,u64>{assert_eq!(slot,38);unsafe{let c=crate::CATALOG.ok_or(1u64)?;Ok(Catalog{target_pd:c.domain,task_id:c.task,surface_id:c.surface,coordinator_pd:c.coordinator})}}
 pub fn info(slot:usize)->Result<Info,u64>{assert_eq!(slot,38);unsafe{match crate::EVIDENCE {
  crate::Evidence::Live=>Err(LIVE),crate::Evidence::Retry=>Err(RETRY),crate::Evidence::Invalid=>Err(1),
  crate::Evidence::Dead{domain,task,required,acked}=>Ok(Info{target_pd:domain,task_id:task,required_mask:required,acked_mask:acked})}}}
}}
include!(env!("LAUNCHER_PRODUCTION"));
unsafe fn reset(){APP_CUSTODY=[Some(app_custody::Entry::new(38,9,91,156)),None,None];CATALOG=Some(Identity{domain:9,task:91,surface:156,coordinator:3});EVIDENCE=Evidence::Live;}
#[test]fn actual_queries_accept_exact_live_not_missing_targets(){let _l=LOCK.lock().unwrap();unsafe{reset();}assert!(native_launcher_status(9).0);assert!(!native_launcher_status(12).0);}
#[test]fn actual_queries_deny_stale_task_and_missing_capability(){let _l=LOCK.lock().unwrap();unsafe{reset();CATALOG.as_mut().unwrap().task=9;}assert!(!native_launcher_status(9).0);unsafe{CATALOG=None;}assert!(!native_launcher_status(9).0);}
#[test]fn actual_queries_never_revive_cached_terminal_subject(){let _l=LOCK.lock().unwrap();unsafe{reset();APP_CUSTODY[0].as_mut().unwrap().dead=true;}assert!(!native_launcher_status(9).0);}
#[test]fn actual_dead_status_explains_absent_successor_and_rejects_bad_masks(){let _l=LOCK.lock().unwrap();unsafe{reset();EVIDENCE=Evidence::Dead{domain:9,task:91,required:31,acked:31};}assert_eq!(native_launcher_status(9).2,"no_boot_successor");unsafe{EVIDENCE=Evidence::Dead{domain:9,task:91,required:1,acked:1};}assert_eq!(native_launcher_status(9).2,"incarnation_unverified");}
#[test]fn actual_closing_and_retry_are_disabled(){let _l=LOCK.lock().unwrap();unsafe{reset();APP_CUSTODY[0].as_mut().unwrap().close_pending=true;}assert_eq!(native_launcher_status(9).2,"incarnation_closing");unsafe{reset();EVIDENCE=Evidence::Retry;}assert!(!native_launcher_status(9).0);}
