#![allow(dead_code)]
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../servers/linen/src/reply_delivery.rs"]mod reply_delivery;
use std::sync::Mutex;
static LOCK:Mutex<()>=Mutex::new(());
static mut CATALOG:Option<sex_pdx::app_lifecycle::CatalogInfo>=None;
static mut STATUS:u64=5;
mod sex_pdx {pub use crate::app_catalog;
 #[macro_export]macro_rules! rec{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
 pub use crate::rec as serial_record;
 pub mod app_lifecycle{
  #[derive(Clone,Copy)]pub struct CatalogInfo{pub target_pd:u32,pub task_id:u32,pub surface_id:u64,pub coordinator_pd:u32}
  pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{assert!(matches!(slot,39|50));unsafe{super::super::CATALOG.ok_or(1)}}
  pub fn info(_:usize)->Result<(),u64>{unsafe{if super::super::STATUS==0 {Ok(())}else{Err(super::super::STATUS)}}}
 }
}
include!(env!("LINEN_OBSERVER_PRODUCTION"));
unsafe fn setup(){CATALOG=Some(sex_pdx::app_lifecycle::CatalogInfo{target_pd:17,task_id:17,surface_id:302,coordinator_pd:3});STATUS=5;}
#[test]fn real_observer_admits_exact_fresh_subject_only(){let _l=LOCK.lock().unwrap();unsafe{setup();assert!(linen_request_allowed(17,0x42));assert!(!linen_request_allowed(12,0x42));assert!(!linen_request_allowed(16,0x42));}}
#[test]fn predecessor_catalog_or_task_cannot_authorize_successor(){let _l=LOCK.lock().unwrap();unsafe{setup();for c in [(12,12,154,3),(17,12,302,3),(17,17,154,3),(17,17,302,7)]{CATALOG=Some(sex_pdx::app_lifecycle::CatalogInfo{target_pd:c.0,task_id:c.1,surface_id:c.2,coordinator_pd:c.3});assert!(!linen_request_allowed(17,0x46));}}}
#[test]fn dead_retry_missing_and_unknown_operation_never_reach_handler(){let _l=LOCK.lock().unwrap();unsafe{setup();for status in [0,1,2,3]{STATUS=status;assert!(!linen_request_allowed(17,0x47));}setup();assert!(!linen_request_allowed(17,0x41));CATALOG=None;assert!(!linen_request_allowed(17,0x42));}}
