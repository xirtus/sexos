#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"] pub mod app_lifecycle;
// The runner supplies both verbatim production function bodies, namespaced.
include!(env!("APP_STARTUP_POLICY"));

use app_catalog::{App, Incarnation};
use app_lifecycle::CatalogInfo;
type Resolve = fn(CatalogInfo) -> Option<&'static Incarnation>;

fn catalog(row: &Incarnation) -> CatalogInfo {
    CatalogInfo { target_pd: row.target, task_id: row.target * 10,
        surface_id: row.surface, coordinator_pd: 3 }
}
fn accepts_family(resolve: Resolve, family: App) {
    let mut seen = 0;
    for row in &app_catalog::INCARNATIONS {
        if row.app != family { continue; }
        let supplied = catalog(row);
        assert_ne!(supplied.target_pd, supplied.task_id);
        let accepted = resolve(supplied).expect("valid independently numbered held task must be accepted");
        assert!(core::ptr::eq(accepted, row), "must return exact catalog row");
        seen += 1;
    }
    assert!(seen >= 2, "fixture must exercise original and successor");
}
fn rejects_other_families(resolve: Resolve, family: App) {
    let mut seen = 0;
    for row in &app_catalog::INCARNATIONS {
        if row.app == family { continue; }
        assert!(resolve(catalog(row)).is_none(), "another real app family is not this app");
        seen += 1;
    }
    assert!(seen >= 2);
}
fn rejects_invalid_identity(resolve: Resolve, family: App) {
    for row in &app_catalog::INCARNATIONS {
        if row.app != family { continue; }
        let valid = catalog(row);
        for invalid in [CatalogInfo { task_id: 0, ..valid },
            CatalogInfo { coordinator_pd: 0, ..valid }, CatalogInfo { coordinator_pd: 11, ..valid },
            CatalogInfo { target_pd: 0, ..valid }, CatalogInfo { target_pd: u32::MAX, ..valid },
            CatalogInfo { surface_id: 0, ..valid }, CatalogInfo { surface_id: u64::MAX, ..valid }] {
            assert!(resolve(invalid).is_none(), "invalid held metadata: {invalid:?}");
        }
        for other in &app_catalog::INCARNATIONS {
            if other.target == row.target { continue; }
            assert!(resolve(CatalogInfo { target_pd: other.target, ..valid }).is_none(), "valid target paired with another surface");
            assert!(resolve(CatalogInfo { surface_id: other.surface, ..valid }).is_none(), "valid surface paired with another target");
        }
    }
}
#[test] fn quil_accepts_independent_task_namespace() { accepts_family(quil::resolve, App::Quil); }
#[test] fn spindle_accepts_independent_task_namespace() { accepts_family(spindle::resolve, App::Spindle); }
#[test] fn quil_rejects_other_families() { rejects_other_families(quil::resolve, App::Quil); }
#[test] fn spindle_rejects_other_families() { rejects_other_families(spindle::resolve, App::Spindle); }
#[test] fn quil_rejects_invalid_identity() { rejects_invalid_identity(quil::resolve, App::Quil); }
#[test] fn spindle_rejects_invalid_identity() { rejects_invalid_identity(spindle::resolve, App::Spindle); }
