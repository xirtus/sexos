// SPDX-License-Identifier: GPL-3.0-only
//! Host metadata selection, not a runtime unequal-task allocator or BOOT grant.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[cfg(not(browser_incarnation_catalog))]
#[path="../crates/sex-pdx/src/app_catalog.rs"] pub mod app_catalog;
// A checker may supply a descriptive future catalog only in this host fixture.
// The included fixture is Rust module contents (without inner module docs).
#[cfg(browser_incarnation_catalog)]
pub mod app_catalog { include!(env!("BROWSER_INCARNATION_CATALOG")); }
#[path="../crates/sex-pdx/src/app_lifecycle.rs"] pub mod app_lifecycle;
#[path="../apps/kaleidoscope/src/incarnation.rs"] mod incarnation;
use app_catalog::{App, Incarnation};
use app_lifecycle::CatalogInfo;
use incarnation::select;

fn held(row:&Incarnation)->CatalogInfo {
    CatalogInfo { target_pd:row.target, task_id:row.target*10,
        surface_id:row.surface, coordinator_pd:3 }
}

#[test] fn original_browser_preserves_independent_task_namespace_and_exact_surface() {
    let identity=select(CatalogInfo {target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}).unwrap();
    assert_eq!((identity.target(),identity.task(),identity.surface(),identity.ordinal()),(15,150,300,0));
    assert!(identity.is_original());
    let copied=identity;assert_eq!(copied,identity);assert!(!format!("{identity:?}").is_empty());
}

#[test] fn all_described_browser_incarnations_preserve_held_task_and_ordinal() {
    let mut count=0;
    for row in app_catalog::INCARNATIONS.iter().filter(|row|row.app==App::Browser) {
        let identity=select(held(row)).unwrap();count+=1;
        assert_eq!((identity.target(),identity.task(),identity.surface(),identity.ordinal()),
                   (row.target,row.target*10,row.surface,row.ordinal));
        assert_eq!(identity.is_original(),row.ordinal==0);
    }
    assert!(count>0);
}

#[test] fn other_families_cannot_select_even_with_exact_nonzero_held_metadata() {
    for row in app_catalog::INCARNATIONS.iter().filter(|row|row.app!=App::Browser) {
        assert!(select(held(row)).is_none(),"other family accepted: {row:?}");
    }
}

#[test] fn zero_unknown_high_alias_and_mismatched_subject_fields_fail_closed() {
    for row in app_catalog::INCARNATIONS.iter().filter(|row|row.app==App::Browser) {
        let c=held(row);
        for bad in [CatalogInfo{target_pd:0,..c},CatalogInfo{task_id:0,..c},
                    CatalogInfo{surface_id:0,..c},CatalogInfo{coordinator_pd:0,..c},
                    CatalogInfo{coordinator_pd:7,..c},CatalogInfo{coordinator_pd:c.target_pd,..c},
                    CatalogInfo{target_pd:u32::MAX,..c},CatalogInfo{target_pd:c.target_pd|(1<<31),..c},
                    CatalogInfo{surface_id:u64::MAX,..c},CatalogInfo{surface_id:c.surface_id|(1<<63),..c},
                    CatalogInfo{coordinator_pd:3|(1<<31),..c}] {
            assert!(select(bad).is_none(),"invalid subject accepted: {bad:?}");
        }
        // A known but unrelated source row cannot be paired with this surface.
        for other in app_catalog::INCARNATIONS.iter().filter(|other|other.target!=row.target) {
            assert!(select(CatalogInfo{target_pd:other.target,..c}).is_none());
            assert!(select(CatalogInfo{surface_id:other.surface,..c}).is_none());
        }
    }
}

#[test] fn nonzero_task_bits_are_preserved_without_hidden_domain_aliasing() {
    // CatalogInfo already has checked u32 fields from the successful SDK query.
    // High bits in the task namespace are data to preserve, not PD bits to mask.
    for task in [15,150,0x8000_0096,u32::MAX] {
        let identity=select(CatalogInfo{target_pd:15,task_id:task,surface_id:300,coordinator_pd:3}).unwrap();
        assert_eq!(identity.task(),task);assert_eq!(identity.target(),15);
    }
}
