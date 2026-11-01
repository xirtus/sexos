//! Actual SDK/grant decoders and immutable Net binding. Explicit future rows
//! below are host metadata only, not actual BOOT capabilities or runtime tasks.
#![allow(dead_code)]
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod original_catalog;
#[cfg(not(browser_source_catalog))]pub use original_catalog as app_catalog;
#[cfg(browser_source_catalog)]pub mod app_catalog{
 pub use crate::original_catalog::*;
 pub const COUNT:usize=19;
 pub const INCARNATIONS:[Incarnation;COUNT]={
  let mut rows=[crate::original_catalog::INCARNATIONS[0];COUNT];let mut i=0;
  while i<17{rows[i]=crate::original_catalog::INCARNATIONS[i];i+=1;}
  rows[17]=Incarnation{app:App::Browser,ordinal:1,target:30,surface:343,shell_surface:None,lifecycle_slot:154,close_slot:155,activation_slot:Some(156),route_slot:157,children:&[]};
  rows[18]=Incarnation{app:App::Browser,ordinal:2,target:31,surface:344,shell_surface:None,lifecycle_slot:158,close_slot:159,activation_slot:Some(160),route_slot:161,children:&[]};rows
 };
 pub const LIFECYCLE_SLOTS:[usize;COUNT]={let mut slots=[0;COUNT];let mut i=0;while i<COUNT{slots[i]=INCARNATIONS[i].lifecycle_slot;i+=1;}slots};
 pub const fn count(app:App)->usize{let mut n=0;let mut i=0;while i<COUNT{if INCARNATIONS[i].app as u8==app as u8{n+=1;}i+=1;}n}
 pub fn by_surface(pd:u32,sid:u64)->Option<&'static Incarnation>{INCARNATIONS.iter().find(|r|r.target==pd&&r.surface==sid)}
}
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../crates/sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../crates/sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../crates/sex-pdx/src/trust_source.rs"]pub mod trust_source;
#[path="../crates/sex-pdx/src/browser_sources.rs"]pub mod browser_sources;
#[path="../servers/sexnet/src/source_binding.rs"]mod source_binding;
use source_binding::{Binding,lookup};
use browser_sources::Plan;
#[derive(Clone,Copy)]struct Queries{
 catalog:(usize,Result<app_lifecycle::CatalogInfo,u64>),
 entropy:(u64,Result<entropy_source::Info,u64>),
 clock:(u64,Result<clock_source::Info,u64>),
 trust:(u64,Result<trust_source::Info,u64>),
}
impl Queries{
 fn bind(self,p:Plan)->Option<Binding>{Binding::from_held(p,self.catalog,self.entropy,self.clock,self.trust)}
}
fn raw_infos(pd:u32,task:u32,available:bool)->[[u64;5];3]{
 let e=entropy_source::Grant{holder:entropy_source::Identity{pd:13,task:130},consumer:entropy_source::Identity{pd,task},source:entropy_source::RDSEED};
 let c=clock_source::Grant{holder:clock_source::Identity{pd:13,task:130},consumer:clock_source::Identity{pd,task},source:clock_source::CMOS_UTC};
 let t=trust_source::Grant{holder:trust_source::Identity{pd:13,task:130},consumer:trust_source::Identity{pd,task},profile:trust_config::PROFILE};
 [entropy_source::invoke(Some(&e),e.holder,entropy_source::INFO,[0;3],||available,||panic!("INFO must not sample entropy")),
 clock_source::invoke(Some(&c),c.holder,clock_source::INFO,[0;3],||panic!("INFO must not read RTC")),
 trust_source::invoke(Some(&t),t.holder,trust_source::INFO,[0;3])]
}
fn queries(p:Plan,task:u32,available:bool)->Queries{
 let [e,c,t]=raw_infos(p.target(),task,available);
 Queries{catalog:(p.lifecycle_slot(),Ok(app_lifecycle::CatalogInfo{target_pd:p.target(),task_id:task,surface_id:p.surface(),coordinator_pd:3})),entropy:(p.entropy_slot(),entropy_source::Info::decode(e)),clock:(p.clock_slot(),clock_source::Info::decode(c)),trust:(p.trust_slot(),trust_source::Info::decode(t))}
}
#[test]fn binding_preserves_actual_held_unequal_task_and_each_typed_decoder_tuple(){
 let p=*browser_sources::by_target(15).unwrap();let q=queries(p,150,true);let b=q.bind(p).unwrap();
 assert_eq!((b.target(),b.task(),b.plan().surface()),(15,150,300));assert_eq!((b.entropy_slot(),b.clock_slot(),b.trust_slot()),(60,61,62));
 assert_eq!(b.entropy_info(),q.entropy.1.unwrap());assert_eq!(b.clock_info(),q.clock.1.unwrap());assert_eq!(b.trust_info(),q.trust.1.unwrap());
 assert!(b.matches(15,150));for(pd,task)in [(15,15),(15,0),(0,150),(30,150),(15,151)]{assert!(!b.matches(pd,task));}
 assert_eq!(lookup(&[Some(b)],15,150),Some(&b));assert!(lookup(&[Some(b)],15,15).is_none());
 // Each valid held task is preserved, never guessed from the static plan.
 for task in [1,151,u32::MAX]{assert_eq!(queries(p,task,true).bind(p).unwrap().task(),task);}
}
#[test]fn binding_unavailable_entropy_remains_held_metadata_without_fallback(){
 let p=browser_sources::PLANS[0];let b=queries(p,150,false).bind(p).unwrap();assert!(!b.entropy_info().available);assert_eq!(b.entropy_slot(),60);
}
#[test]fn binding_query_errors_absent_wrong_kind_and_slot_labels_fail_closed(){
 let p=browser_sources::PLANS[0];let good=queries(p,150,true);
 for error in [1,2,u64::MAX,u64::MAX-1,u64::MAX-2,u64::MAX-3]{for source in 0..4{let mut q=good;match source{0=>q.catalog.1=Err(error),1=>q.entropy.1=Err(error),2=>q.clock.1=Err(error),_=>q.trust.1=Err(error)}assert!(q.bind(p).is_none());}}
 for source in 0..4{for slot in [0,38,43,63,70,1025,u32::MAX as u64,1u64<<32,u64::MAX]{let mut q=good;match source{0=>q.catalog.0=slot as usize,1=>q.entropy.0=slot,2=>q.clock.0=slot,_=>q.trust.0=slot}assert!(q.bind(p).is_none());}}
 let mut q=good;std::mem::swap(&mut q.entropy.0,&mut q.clock.0);assert!(q.bind(p).is_none());
}
#[test]fn binding_catalog_family_coordinator_surface_and_consumer_mismatch_fail_closed(){
 let p=browser_sources::PLANS[0];let good=queries(p,150,true);
 for field in 0..5{let mut q=good;let c=q.catalog.1.as_mut().unwrap();match field{0=>c.target_pd=9,1=>c.surface_id=156,2=>c.task_id=0,3=>c.task_id=15,_=>c.coordinator_pd=4};assert!(q.bind(p).is_none());}
 for source in 0..3{for(pd,task)in [(0,150),(15,0),(15,15),(15,151),(30,300),(31,310)]{let mut q=good;match source{0=>q.entropy.1.as_mut().unwrap().consumer=entropy_source::Identity{pd,task},1=>q.clock.1.as_mut().unwrap().consumer=clock_source::Identity{pd,task},_=>q.trust.1.as_mut().unwrap().consumer=trust_source::Identity{pd,task}}assert!(q.bind(p).is_none());}}
 assert!(browser_sources::by_target(9).is_none());assert!(browser_sources::by_target(12).is_none());
}
#[test]fn binding_rechecks_retained_source_profile_root_and_version(){
 let p=browser_sources::PLANS[0];let good=queries(p,150,true);
 for source in [0,2,u32::MAX]{let mut q=good;q.clock.1.as_mut().unwrap().source=source;assert!(q.bind(p).is_none());}
 for field in 0..3{let mut q=good;let t=q.trust.1.as_mut().unwrap();match field{0=>t.profile+=1,1=>t.root_bytes-=1,_=>t.version+=1};assert!(q.bind(p).is_none());}
}
#[test]fn binding_actual_decoders_reject_lost_attempt_precision_profile_and_dirty_error_fields(){
 let p=browser_sources::PLANS[0];let good=queries(p,150,true);let raw=raw_infos(15,150,true);
 for(source,field,value)in [(0,1,2),(0,3,9),(0,4,2),(0,1,1u64<<32),(1,1,2),(1,3,0),(1,3,5),(1,4,0),(1,4,2),(2,1,2),(2,3,0),(2,4,0),(2,1,1u64<<32)]{
  let mut words=raw[source];words[field]=value;let mut q=good;match source{0=>q.entropy.1=entropy_source::Info::decode(words),1=>q.clock.1=clock_source::Info::decode(words),_=>q.trust.1=trust_source::Info::decode(words)}assert!(q.bind(p).is_none());
 }
 for source in 0..3{let words=[u64::MAX,1,0,0,0];let mut q=good;match source{0=>q.entropy.1=entropy_source::Info::decode(words),1=>q.clock.1=clock_source::Info::decode(words),_=>q.trust.1=trust_source::Info::decode(words)}assert!(q.bind(p).is_none());}
 // Consumer packing differs by typed protocol; exchanging formats cannot
 // invent a match when PD15 and task150 intentionally differ.
 assert_ne!(raw[0][2],raw[2][2]);let mut clock=raw[1];clock[2]=raw[2][2];let mut q=good;q.clock.1=clock_source::Info::decode(clock);assert!(q.bind(p).is_none());
}
#[test]fn binding_lookup_exact_owner_has_no_implicit_current_client_or_duplicate_preference(){
 let p=browser_sources::PLANS[0];let a=queries(p,150,true).bind(p).unwrap();let b=queries(p,151,false).bind(p).unwrap();
 let bindings=[Some(a),None,Some(b)];assert_eq!(lookup(&bindings,15,150),Some(&a));assert_eq!(lookup(&bindings,15,151),Some(&b));assert_eq!(lookup(&[Some(a),Some(a)],15,150),None);
 assert_eq!(a.task(),150);assert!(a.entropy_info().available);assert_eq!(a.entropy_slot(),60);
}
#[cfg(not(browser_source_catalog))]
#[test]fn binding_current_plan_has_no_successor_authority(){assert_eq!(browser_sources::BROWSER_COUNT,1);assert!(browser_sources::by_target(30).is_none());assert!(browser_sources::by_target(31).is_none());}
#[cfg(browser_source_catalog)]
#[test]fn binding_future_host_catalog_keeps_three_distinct_retained_owners_and_slots(){
 assert_eq!(browser_sources::BROWSER_COUNT,3);
 let expected=[(15,150,300,40,[60,61,62]),(30,300,343,154,[70,71,72]),(31,310,344,158,[73,74,75])];
 let mut bindings=[None;3];for(i,(pd,task,sid,life,slots))in expected.into_iter().enumerate(){let p=*browser_sources::by_target(pd).unwrap();let b=queries(p,task,true).bind(p).unwrap();assert_eq!((p.surface(),p.lifecycle_slot()),(sid,life));assert_eq!([b.entropy_slot(),b.clock_slot(),b.trust_slot()],slots);bindings[i]=Some(b);}
 for(pd,task,_,_,slots)in expected{let b=lookup(&bindings,pd,task).unwrap();assert_eq!(b.entropy_slot(),slots[0]);assert!(lookup(&bindings,pd,pd).is_none());}
 let old=lookup(&bindings,15,150).copied().unwrap();let selected=lookup(&bindings,31,310).unwrap();assert_eq!(selected.entropy_slot(),73);assert_eq!(old.entropy_slot(),60);
 let p30=*browser_sources::by_target(30).unwrap();let q15=queries(browser_sources::PLANS[0],150,true);assert!(q15.bind(p30).is_none());
}
#[test]fn binding_actual_absent_and_wrong_holder_source_queries_fail_before_providers(){
 let p=browser_sources::PLANS[0];let good=queries(p,150,true);
 let e=entropy_source::Grant{holder:entropy_source::Identity{pd:13,task:130},consumer:entropy_source::Identity{pd:15,task:150},source:1};
 let c=clock_source::Grant{holder:clock_source::Identity{pd:13,task:130},consumer:clock_source::Identity{pd:15,task:150},source:1};
 let t=trust_source::Grant{holder:trust_source::Identity{pd:13,task:130},consumer:trust_source::Identity{pd:15,task:150},profile:trust_config::PROFILE};
 for absent in [true,false]{
  let eraw=entropy_source::invoke(if absent{None}else{Some(&e)},entropy_source::Identity{pd:13,task:13},entropy_source::INFO,[0;3],||panic!("denied query reached feature provider"),||panic!("denied query sampled"));
  let craw=clock_source::invoke(if absent{None}else{Some(&c)},clock_source::Identity{pd:13,task:13},clock_source::INFO,[0;3],||panic!("denied query read RTC"));
  let traw=trust_source::invoke(if absent{None}else{Some(&t)},trust_source::Identity{pd:13,task:13},trust_source::INFO,[0;3]);
  assert_eq!(eraw,[entropy_source::DENIED,0,0,0,0]);assert_eq!(craw,[clock_source::DENIED,0,0,0,0]);assert_eq!(traw,[trust_source::DENIED,0,0,0,0]);
  for source in 0..3{let mut q=good;match source{0=>q.entropy.1=entropy_source::Info::decode(eraw),1=>q.clock.1=clock_source::Info::decode(craw),_=>q.trust.1=trust_source::Info::decode(traw)}assert!(q.bind(p).is_none());}
 }
}
