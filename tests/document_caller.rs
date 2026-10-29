//! Actual admission helper, actual catalog validator; only syscalls are doubles.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod protocol;
mod app_lifecycle {
 pub use crate::protocol::{CatalogInfo,LIVE};
 thread_local!{pub static BAD:std::cell::Cell<u8>=const{std::cell::Cell::new(0)};pub static CALLS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
 pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{
  CALLS.set(CALLS.get()+1);let(pd,surface)=match slot{38=>(9,156),45=>(16,301),54=>(18,307),_=>panic!()};let bad=BAD.get();
  if bad==1{return Err(2)}
  Ok(CatalogInfo{target_pd:if bad==2{99}else{pd},task_id:if bad==3{0}else{pd*10},surface_id:if bad==4{999}else{surface},coordinator_pd:if bad==5{99}else{3}})
 }
 pub fn info(slot:usize)->Result<(),u64>{assert!(matches!(slot,38|45|54));CALLS.set(CALLS.get()+1);match BAD.get(){6=>Ok(()),7=>Err(2),8=>Err(3),_=>Err(LIVE)}}
}
include!(env!("DOCUMENT_CALLER_SOURCE"));
#[test]fn all_three_domains_use_their_exact_catalog_task_namespace(){for pd in [9,16,18]{app_lifecycle::BAD.set(0);assert!(document_caller_live(pd));}}
#[test]fn wrong_catalog_dead_retry_or_denied_never_admit(){for pd in [9,16,18]{for bad in 1..=8{app_lifecycle::BAD.set(bad);assert!(!document_caller_live(pd),"pd={pd} bad={bad}");}}}
#[test]fn unknown_or_wide_lookalike_subject_cannot_query_authority(){for pd in [0,3,12,17,19,u32::MAX]{app_lifecycle::CALLS.set(0);assert!(!document_caller_live(pd));assert_eq!(app_lifecycle::CALLS.get(),0);}}
