#![allow(dead_code)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY:u64=2;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]pub mod app_lifecycle;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"]mod bitmap_binding;
use app_lifecycle::CatalogInfo;
use bitmap_binding::Binding;

fn browser()->CatalogInfo {
    CatalogInfo{target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}
}
#[test]
fn actual_browser_catalog_and_unequal_task_bind_exactly() {
    let row=app_catalog::by_surface(15,300).unwrap();
    assert_eq!((row.lifecycle_slot,row.app),(40,app_catalog::App::Browser));
    let b=Binding::from_catalog(40,browser()).unwrap();
    assert_eq!((b.slot(),b.target(),b.task(),b.surface(),b.coordinator()),(40,15,150,300,3));
    assert!(b.admits(15,150,300));assert!(b.matches_retirement(15,150));
}
#[test]
fn exact_sender_tuple_and_retirement_require_actual_task_not_domain() {
    let b=Binding::from_catalog(40,browser()).unwrap();
    for (pd,task,sid) in [(15,15,300),(15,0,300),(15,151,300),(0,150,300),
                           (30,150,300),(15,150,0),(15,150,343),(u32::MAX,150,300),
                           (15,u32::MAX,300),(15,150,u64::MAX)] {
        assert!(!b.admits(pd,task,sid));
    }
    for (pd,task) in [(15,15),(15,0),(15,151),(0,150),(30,150)] {
        assert!(!b.matches_retirement(pd,task));
    }
    assert_eq!(b,Binding::from_catalog(40,browser()).unwrap());
}
#[test]
fn missing_wrong_slot_or_tuple_cannot_construct_binding() {
    for slot in [0,39,41,154,usize::MAX] {assert!(Binding::from_catalog(slot,browser()).is_none());}
    for c in [CatalogInfo{target_pd:0,..browser()},CatalogInfo{target_pd:30,..browser()},
              CatalogInfo{target_pd:31,..browser()},CatalogInfo{task_id:0,..browser()},
              CatalogInfo{surface_id:0,..browser()},CatalogInfo{surface_id:343,..browser()},
              CatalogInfo{surface_id:344,..browser()},CatalogInfo{surface_id:65536,..browser()},
              CatalogInfo{surface_id:u64::MAX,..browser()},CatalogInfo{coordinator_pd:0,..browser()},
              CatalogInfo{coordinator_pd:1,..browser()},CatalogInfo{coordinator_pd:15,..browser()}] {
        assert!(Binding::from_catalog(40,c).is_none(),"accepted {c:?}");
    }
}
#[test]
fn actual_non_browser_families_rejected_even_with_their_exact_held_tuple() {
    let mut count=0;
    for row in app_catalog::INCARNATIONS.iter().filter(|r|r.app!=app_catalog::App::Browser) {
        let c=CatalogInfo{target_pd:row.target,task_id:row.target*10,surface_id:row.surface,coordinator_pd:3};
        assert!(c.is_subject(row.target,row.surface,3));
        assert!(Binding::from_catalog(row.lifecycle_slot,c).is_none());count+=1;
    }
    assert_eq!(count,29); // Independent current sixteen-Quil/nine-Spindle/four-Program roster.
}
#[test]
fn constructor_preserves_held_task_without_assuming_or_selecting_one() {
    for task in [1,15,150,151,u32::MAX] {
        let b=Binding::from_catalog(40,CatalogInfo{task_id:task,..browser()}).unwrap();
        assert_eq!(b.task(),task);assert!(b.admits(15,task,300));
        if task!=150 {assert!(!b.admits(15,150,300));}
    }
    // This demonstrates pure tuple checking, not runtime metadata mutation:
    // production is responsible for the one-time held-query installation.
}
