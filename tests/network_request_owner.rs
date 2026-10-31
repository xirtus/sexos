#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=2;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod lifecycle_wire;
mod jobs {
    #[derive(Clone,Copy,Debug,PartialEq,Eq)]
    pub struct Identity {pub pd:u32,pub task:u32}
}
use std::cell::RefCell;
#[derive(Clone,Copy)]
struct Held {slot:usize,catalog:Result<lifecycle_wire::CatalogInfo,u64>,state:Result<lifecycle_wire::LifecycleInfo,u64>}
thread_local! {
    static HELD:RefCell<Vec<Held>>=const{RefCell::new(Vec::new())};
    static QUERIES:RefCell<usize>=const{RefCell::new(0)};
}
pub mod app_lifecycle {
    pub use crate::lifecycle_wire::{CatalogInfo,LifecycleInfo,LIVE,SLOTS};
    pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{
        crate::QUERIES.with(|n|*n.borrow_mut()+=1);
        crate::HELD.with(|h|h.borrow().iter().find(|g|g.slot==slot).map_or(Err(1),|g|g.catalog))
    }
    pub fn info(slot:usize)->Result<LifecycleInfo,u64>{
        crate::QUERIES.with(|n|*n.borrow_mut()+=1);
        crate::HELD.with(|h|h.borrow().iter().find(|g|g.slot==slot).map_or(Err(1),|g|g.state))
    }
}
// The gate extracts these two exact production functions, not a rewritten
// policy. Kernel query transport is doubled; no host syscall is executed.
include!(env!("NETWORK_REQUEST_OWNER_POLICY"));

fn held(target:u32,task:u32)->Held {
    let row=app_catalog::by_target(target).unwrap();
    Held{slot:row.lifecycle_slot,catalog:Ok(lifecycle_wire::CatalogInfo{
        target_pd:target,task_id:task,surface_id:row.surface,coordinator_pd:3}),state:Err(app_lifecycle::LIVE)}
}
fn install(entries:Vec<Held>){HELD.with(|h|*h.borrow_mut()=entries);QUERIES.with(|n|*n.borrow_mut()=0);}
fn admitted(pd:u32,task:u32)->Option<jobs::Identity>{network_request_owner(pd,task)}
#[test]
fn original_browser_and_all_eight_spindles_require_exact_unequal_main_tasks(){
    let targets=[15,12,17,24,25,26,27,28,29];
    install(targets.map(|pd|held(pd,pd*10)).to_vec());
    for pd in targets {
        assert_eq!(admitted(pd,pd*10),Some(jobs::Identity{pd,task:pd*10}));
        assert_eq!(network_owner(pd),Some(jobs::Identity{pd,task:pd*10}));
        for wrong in [0,pd,pd*10+1,u32::MAX] {assert_eq!(admitted(pd,wrong),None);}
    }
}
#[test]
fn caller_zero_unknown_or_foreign_task_cannot_borrow_another_held_subject(){
    install(vec![held(15,150),held(17,170)]);
    for (pd,task)in[(0,150),(30,150),(31,150),(u32::MAX,150),(15,170),(17,150)] {
        assert_eq!(admitted(pd,task),None);
    }
}
#[test]
fn zero_incoming_task_does_not_even_query_metadata(){
    install(vec![held(15,150)]);assert_eq!(admitted(15,0),None);
    QUERIES.with(|n|assert_eq!(*n.borrow(),0));
}
#[test]
fn missing_dead_retry_or_denied_held_state_never_admits(){
    install(vec![]);assert_eq!(admitted(15,150),None);
    let base=held(15,150);
    for error in [1,2,3,4,6,u64::MAX] {
        install(vec![Held{state:Err(error),..base}]);assert_eq!(admitted(15,150),None);
        install(vec![Held{catalog:Err(error),..base}]);assert_eq!(admitted(15,150),None);
    }
    install(vec![Held{state:Ok(lifecycle_wire::LifecycleInfo{target_pd:15,task_id:150,required_mask:31,acked_mask:31}),..base}]);
    assert_eq!(admitted(15,150),None);assert_eq!(network_owner(15),None);
}
#[test]
fn held_target_and_task_components_cannot_be_invented_by_the_request(){
    let base=held(15,150);let good=base.catalog.unwrap();
    for catalog in [lifecycle_wire::CatalogInfo{target_pd:17,..good},
                    lifecycle_wire::CatalogInfo{target_pd:0,..good},
                    lifecycle_wire::CatalogInfo{task_id:0,..good},
                    lifecycle_wire::CatalogInfo{task_id:151,..good}] {
        install(vec![Held{catalog:Ok(catalog),..base}]);assert_eq!(admitted(15,150),None);
    }
    // Metadata outside the immutable held-slot inventory confers nothing.
    install(vec![Held{slot:999,..base}]);assert_eq!(admitted(15,150),None);
}
#[test]
fn retry_revalidates_current_liveness_instead_of_caching_admission(){
    let base=held(15,150);install(vec![base]);assert!(admitted(15,150).is_some());
    install(vec![Held{state:Err(2),..base}]);assert_eq!(admitted(15,150),None);
    install(vec![base]);assert!(admitted(15,150).is_some());
    install(vec![]);assert_eq!(admitted(15,150),None);
}
#[test]
fn old_task_cannot_rebind_to_a_different_held_task(){
    install(vec![held(15,150)]);let old=admitted(15,150).unwrap();
    // Adversarial metadata change, not a claim that BOOT identities respawn.
    install(vec![held(15,151)]);
    assert_eq!(admitted(15,150),None);
    assert_ne!(network_owner(15),Some(old));
    assert_eq!(admitted(15,151),Some(jobs::Identity{pd:15,task:151}));
}
