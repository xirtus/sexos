#![allow(dead_code)]
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod lifecycle_protocol;
#[path="../servers/linen/src/reply_delivery.rs"]mod reply_delivery;
use std::sync::Mutex;
static LOCK:Mutex<()>=Mutex::new(());
static mut CATALOG:Option<sex_pdx::app_lifecycle::CatalogInfo>=None;
static mut STATUS:u64=5;
static mut CATALOG_ERROR:u64=1;
mod sex_pdx {pub use crate::app_catalog;
 #[macro_export]macro_rules! rec{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
 pub use crate::rec as serial_record;
 pub mod app_lifecycle{
  pub use crate::lifecycle_protocol::CatalogInfo;
  pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{assert!(crate::app_catalog::spindle_views().any(|row|row.lifecycle_slot==slot));unsafe{super::super::CATALOG.ok_or(super::super::CATALOG_ERROR)}}
  pub fn info(_:usize)->Result<(),u64>{unsafe{if super::super::STATUS==0 {Ok(())}else{Err(super::super::STATUS)}}}
 }
}
include!(env!("LINEN_OBSERVER_PRODUCTION"));
unsafe fn setup(){CATALOG=Some(sex_pdx::app_lifecycle::CatalogInfo{target_pd:17,task_id:17,surface_id:302,coordinator_pd:3});STATUS=5;CATALOG_ERROR=1;}
#[test]fn real_observer_admits_exact_fresh_subject_only(){let _l=LOCK.lock().unwrap();unsafe{setup();assert!(linen_request_allowed(17,0x42));assert!(!linen_request_allowed(12,0x42));assert!(!linen_request_allowed(16,0x42));}}
#[test]fn wrong_catalog_target_zero_task_surface_or_coordinator_cannot_authorize_successor(){let _l=LOCK.lock().unwrap();unsafe{setup();for c in [(12,12,154,3),(17,0,302,3),(17,17,154,3),(17,17,302,7)]{CATALOG=Some(sex_pdx::app_lifecycle::CatalogInfo{target_pd:c.0,task_id:c.1,surface_id:c.2,coordinator_pd:c.3});assert!(!linen_request_allowed(17,0x46));}}}
#[test]fn dead_retry_missing_and_unknown_operation_never_reach_handler(){let _l=LOCK.lock().unwrap();unsafe{setup();for status in [0,1,2,3]{STATUS=status;assert!(!linen_request_allowed(17,0x47));}setup();assert!(!linen_request_allowed(17,0x41));CATALOG=None;assert!(!linen_request_allowed(17,0x42));}}

#[test]fn held_nonzero_task_namespace_is_independent_for_both_spindle_subjects(){
 let _l=LOCK.lock().unwrap();unsafe{
 for(target,task,surface)in[(12,120,154),(17,170,302)]{
  setup();CATALOG=Some(sex_pdx::app_lifecycle::CatalogInfo{target_pd:target,task_id:task,surface_id:surface,coordinator_pd:3});
  for opcode in [0x42,0x46,0x47]{assert!(linen_request_allowed(target,opcode));}
  assert!(!linen_request_allowed(target,0x41));
  // The only task association is the successful held catalog; no request
  // task word exists in this legacy (caller PD, opcode) admission helper.
  for changed in [sex_pdx::app_lifecycle::CatalogInfo{target_pd:target+1,task_id:task,surface_id:surface,coordinator_pd:3},
   sex_pdx::app_lifecycle::CatalogInfo{target_pd:target,task_id:0,surface_id:surface,coordinator_pd:3},
   sex_pdx::app_lifecycle::CatalogInfo{target_pd:target,task_id:task,surface_id:surface+1,coordinator_pd:3},
   sex_pdx::app_lifecycle::CatalogInfo{target_pd:target,task_id:task,surface_id:surface,coordinator_pd:7}]{
   CATALOG=Some(changed);assert!(!linen_request_allowed(target,0x42));
  }
 }
}}
#[test]fn missing_or_revoked_held_observer_cannot_be_replaced_by_descriptive_catalog(){
 let _l=LOCK.lock().unwrap();unsafe{
 for target in [12,17]{for error in [1,3]{setup();CATALOG=None;CATALOG_ERROR=error;for opcode in [0x42,0x46,0x47]{assert!(!linen_request_allowed(target,opcode));}}}
 setup();for target in [0,9,15,16,18,99,u32::MAX]{assert!(!linen_request_allowed(target,0x42));}
}}
