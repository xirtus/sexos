// SPDX-License-Identifier: GPL-3.0-only
//! Actual descriptive plan and typed mechanism; future catalog rows are host
//! metadata only, not BOOT grants, task allocation, native admission or sampling.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[cfg(not(browser_source_catalog))]
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[cfg(browser_source_catalog)]
pub mod app_catalog{include!(env!("BROWSER_SOURCE_CATALOG"));}
#[path="../crates/sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../crates/sex-pdx/src/browser_sources.rs"]mod browser_sources;
use app_catalog::App;
use browser_sources::{by_index,by_target,Plan,PLANS,BROWSER_COUNT};
fn triple(plan:Plan)->[u64;3]{[plan.entropy_slot(),plan.clock_slot(),plan.trust_slot()]}

#[test]fn original_plan_preserves_existing_browser_association_and_source_slots(){
 let plan=*by_target(15).unwrap();assert_eq!(plan.target(),15);assert_eq!(plan.surface(),300);
 assert_eq!(plan.lifecycle_slot(),40);assert_eq!(triple(plan),[60,61,62]);
 assert_eq!(plan.entropy_slot(),entropy_source::SLOT);assert_eq!(by_index(0),Some(&plan));
 assert_eq!(plan,plan.clone());assert!(!format!("{plan:?}").is_empty());
}
#[test]fn actual_catalog_only_controls_number_and_order_not_candidate_source_slot_count(){
 let rows:Vec<_>=app_catalog::INCARNATIONS.iter().filter(|row|row.app==App::Browser).collect();
 assert_eq!(BROWSER_COUNT,rows.len());assert_eq!(PLANS.len(),rows.len());
 #[cfg(not(browser_source_catalog))]{assert_eq!(BROWSER_COUNT,3);assert!(by_target(30).is_some());assert!(by_target(31).is_some());}
 for(index,row)in rows.iter().enumerate(){let plan=*by_index(index).unwrap();
  assert_eq!(plan.target(),row.target);assert_eq!(plan.surface(),row.surface);assert_eq!(plan.lifecycle_slot(),row.lifecycle_slot);
  assert_eq!(by_target(row.target),Some(&plan));assert_eq!(row.ordinal as usize,index);
  assert_eq!(triple(plan),match row.ordinal{0=>[60,61,62],1=>[70,71,72],2=>[73,74,75],_=>panic!("unsupported test catalog")});
 }
 assert!(by_index(BROWSER_COUNT).is_none());assert!(by_index(usize::MAX).is_none());
}
#[test]fn every_provider_slot_is_full_width_bounded_distinct_and_noncolliding(){
 let mut seen=Vec::new();for plan in PLANS{for slot in triple(plan){
  assert!((1..=1024).contains(&slot));assert!(!seen.contains(&slot));seen.push(slot);
  assert!(![1,2,3,19,32].contains(&slot));
  for row in app_catalog::INCARNATIONS{assert_ne!(slot,row.lifecycle_slot as u64);}
 }}assert_eq!(seen.len(),BROWSER_COUNT*3);
}
#[test]fn non_browser_unknown_and_high_alias_keys_never_select_a_plan(){
 for row in app_catalog::INCARNATIONS{if row.app!=App::Browser{assert!(by_target(row.target).is_none());}}
 for target in[0,13,u32::MAX,15|(1<<16),15|(1<<31)]{assert!(by_target(target).is_none());}
 for row in app_catalog::INCARNATIONS.iter().filter(|row|row.app==App::Browser){
  for other in app_catalog::INCARNATIONS.iter().filter(|other|other.target!=row.target){assert_ne!(by_target(row.target).unwrap().target(),other.target);}
 }
}
#[test]fn future_host_rows_receive_separate_triples_without_inferring_task_identity(){
 // Literal host expectations remain independent of the production slot map.
 for(target,surface,life,slots,task)in[(30,343,154,[70,71,72],300),(31,344,158,[73,74,75],310)]{
  if let Some(plan)=by_target(target){
   assert_eq!((plan.target(),plan.surface(),plan.lifecycle_slot()),(target,surface,life));assert_eq!(triple(*plan),slots);
   assert_ne!(task,plan.target());
  }else{assert!(!app_catalog::INCARNATIONS.iter().any(|row|row.target==target));}
 }
}
#[test]fn plan_metadata_never_supplies_source_authority_or_holder_task(){
 use entropy_source::{self as e,Identity,Grant,Info};
 let holder=Identity{pd:13,task:130};
 for plan in PLANS{
  assert_eq!(e::invoke(None,holder,e::INFO,[0;3],||panic!("unheld feature lookup"),||panic!("unheld sample")),[e::DENIED,0,0,0,0]);
  // This explicit host grant exercises the existing pure mechanism, not BOOT.
  // Its independently supplied task is preserved; no Plan task getter exists.
  let task=match plan.target(){15=>150,30=>300,31=>310,_=>panic!("unknown fixture task")};
  let consumer=Identity{pd:plan.target(),task};let grant=Grant{holder,consumer,source:e::RDSEED};
  assert_eq!(e::invoke(Some(&grant),Identity{pd:13,task:13},e::INFO,[0;3],||panic!("wrong holder"),||panic!("wrong sample")),[e::DENIED,0,0,0,0]);
  let raw=e::invoke(Some(&grant),holder,e::INFO,[0;3],||false,||panic!("Info must not sample"));
  assert_eq!(Info::decode(raw),Ok(Info{consumer,available:false}));assert_ne!(consumer.pd,consumer.task);
 }
}
