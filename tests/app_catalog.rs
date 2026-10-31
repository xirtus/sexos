//! The BOOT app incarnation catalog must describe exactly what BOOT grants.
#[path = "../crates/sex-pdx/src/app_catalog.rs"]
mod app_catalog;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"]
#[allow(dead_code)]
mod app_lifecycle;
use app_catalog::{by_surface, by_target, is_document_app, App, INCARNATIONS};

#[test]
fn catalog_slots_equal_the_lifecycle_protocol_constants() {
    let lifecycle: Vec<usize> = INCARNATIONS.iter().map(|i| i.lifecycle_slot).collect();
    assert_eq!(lifecycle, app_lifecycle::SLOTS.to_vec());
    let closes: Vec<u64> = INCARNATIONS.iter().map(|i| i.close_slot).collect();
    assert_eq!(&closes[..3], &app_lifecycle::CLOSE_SLOTS.map(|s| s as u64));
    assert_eq!(closes[3..], [app_lifecycle::SUCCESSOR_CLOSE_SLOT, app_lifecycle::SPINDLE_CLOSE_SLOT, app_lifecycle::QUIL_THIRD_CLOSE_SLOT]);
    let activations: Vec<Option<usize>> = INCARNATIONS.iter().map(|i| i.activation_slot).collect();
    assert_eq!(activations, [None, None, None, Some(app_lifecycle::ACTIVATION_SLOT), Some(app_lifecycle::SPINDLE_ACTIVATION_SLOT), Some(app_lifecycle::QUIL_THIRD_ACTIVATION_SLOT)]);
    assert_eq!(app_lifecycle::QUIL_THIRD_LIFECYCLE_SLOT, INCARNATIONS[5].lifecycle_slot);
    assert_eq!(app_lifecycle::SPINDLE_LIFECYCLE_SLOT, INCARNATIONS[4].lifecycle_slot);
}

#[test]
fn kernel_boot_plan_is_built_from_the_catalog() {
    let init = include_str!("../kernel/src/init.rs");
    for needle in ["plan_boot_apps(&sex_pdx::app_catalog::TARGETS)", "&sex_pdx::app_catalog::CONTENT,",
                   "init_documents(sexfiles_id, sex_pdx::app_catalog::QUIL_TARGETS)",
                   "for id in sex_pdx::app_catalog::TARGETS{", "sex_pdx::app_catalog::predecessor(incarnation)",
                   "for slot in [incarnation.close_slot,incarnation.route_slot]{"] {
        assert!(init.contains(needle), "{needle}");
    }
    // No successor identity is hand-wired in boot any more.
    for stale in ["Some(16),16,9", "Some(17),17,12", "Some(18),18,9", "SUCCESSOR_CLOSE_SLOT", "QUIL_THIRD_ACTIVATION_SLOT"] {
        assert!(!init.contains(stale), "{stale}");
    }
    assert_eq!(app_catalog::TARGETS, [9, 12, 15, 16, 17, 18]);
    assert_eq!(app_catalog::CONTENT, [(9,156), (12,154), (15,300), (16,301), (17,302), (18,307)]);
    let chain: Vec<(u32, u32)> = INCARNATIONS.iter().filter(|i| i.ordinal != 0).map(|i| (app_catalog::predecessor(i).unwrap().target, i.target)).collect();
    assert_eq!(chain, [(9, 16), (12, 17), (16, 18)]);
}

#[test]
fn identities_are_unique_and_ordinals_follow_activation() {
    for (a, x) in INCARNATIONS.iter().enumerate() {
        for y in &INCARNATIONS[a + 1..] {
            assert!(x.target != y.target && x.surface != y.surface && x.lifecycle_slot != y.lifecycle_slot && x.close_slot != y.close_slot);
            assert!(x.shell_surface.is_none() || x.shell_surface != y.shell_surface);
        }
        assert_eq!(x.ordinal == 0, x.activation_slot.is_none());
    }
    for app in [App::Quil, App::Spindle, App::Browser] {
        let ordinals: Vec<u8> = INCARNATIONS.iter().filter(|i| i.app == app).map(|i| i.ordinal).collect();
        assert_eq!(ordinals, (0..ordinals.len() as u8).collect::<Vec<_>>());
    }
}

#[test]
fn lookups_require_the_exact_pair() {
    assert_eq!(by_surface(9, 156).map(|i| i.shell_surface), Some(Some(201)));
    assert!(by_surface(9, 301).is_none() && by_surface(16, 156).is_none());
    assert!(by_target(0).is_none() && by_target(3).is_none() && by_target(19).is_none());
    assert!([9, 16, 18].iter().all(|&t| is_document_app(t)) && ![12, 15, 17, 0].iter().any(|&t| is_document_app(t)));
}

#[test]
fn shell_view_tables_equal_the_previous_hand_wired_order() {
    // Shell's delivery arrays are positional; this pins the exact old order.
    let docs: Vec<(u32, u64, u64)> = app_catalog::document_views().map(|i| (i.target, i.frame(), i.route_slot)).collect();
    assert_eq!(docs, [(9, 201, 11), (16, 301, app_lifecycle::SUCCESSOR_ROUTE_SLOT), (18, 307, app_lifecycle::QUIL_THIRD_ROUTE_SLOT), (15, 300, app_lifecycle::CLOSE_SLOTS[2] as u64)]);
    let spindle: Vec<(u32, u64, u64)> = app_catalog::spindle_views().map(|i| (i.target, i.frame(), i.route_slot)).collect();
    assert_eq!(spindle, [(12, 153, 14), (17, 306, app_lifecycle::SPINDLE_ROUTE_SLOT)]);
    assert_eq!(app_catalog::document_view_frame(201).map(|(i, v)| (i, v.target)), Some((0, 9)));
    assert!(app_catalog::document_view_frame(156).is_none(), "content surface is not a frame");
    let pdx = include_str!("../crates/sex-pdx/src/lib.rs");
    assert!(pdx.contains("pub const SLOT_QUIL: u64 = 11;") && pdx.contains("pub const SLOT_SPINDLE: u64 = 14;"));
}

#[test]
fn display_tables_equal_the_previous_hand_wired_ones() {
    let old = |owner: u32, sid: u64| -> u64 { match (owner, sid) { (9,156) => 1, (12,154|160|161|162) => 2, (15,300) => 4, (16,301) => 8, (17,302|303|304|305) => 16, (18,307) => 32, _ => 0 } };
    for owner in 0..40u32 { for sid in 0..400u64 { assert_eq!(app_catalog::native_pixel_bit(owner, sid), old(owner, sid), "{owner}/{sid}"); } }
    const BIT: u64 = app_catalog::native_pixel_bit(17, 304);
    assert_eq!(BIT, 16, "usable in const context");
    for (target, surface, sid, want) in [(12,154,161,true),(12,154,303,false),(17,302,305,true),(9,156,201,false),(9,156,156,true)] {
        assert_eq!(app_catalog::is_content(target, surface, sid), want, "{target}/{surface}/{sid}");
    }
}

#[test]
fn sex_pdx_policy_helpers_equal_the_previous_hand_wired_sets() {
    for t in 0..40u32 {
        assert_eq!(app_catalog::is_quil(t), matches!(t, 9 | 16 | 18), "{t}");
        assert_eq!(app_catalog::is_spindle(t), matches!(t, 12 | 17), "{t}");
        assert_eq!(app_catalog::is_browser(t), t == 15, "{t}");
        // document_view: frameless document holders used a 36-cell top band.
        let frameless_doc = app_catalog::app_of(t).is_some() && !app_catalog::has_shell_frame(t) && !app_catalog::is_spindle(t);
        assert_eq!(frameless_doc, matches!(t, 15 | 16 | 18), "{t}");
        for s in 0..400u64 {
            // desktop_style::native_canvas
            let native = app_catalog::native_pixel_bit(t, s) != 0 && !app_catalog::is_browser(t);
            let old = matches!((t, s), (9,156)|(16,301)|(18,307)|(12,154|160|161|162)|(17,302|303|304|305));
            assert_eq!(native, old, "{t}/{s}");
            // clipboard::Participant routes
            let new = by_surface(t, s).filter(|i| i.app != App::Spindle).map(|i| (i.close_slot, i.app == App::Quil));
            let old = match (t, s) { (9,156)=>Some((41,true)), (15,300)=>Some((43,false)), (16,301)=>Some((46,true)), (18,307)=>Some((56,true)), _=>None };
            assert_eq!(new, old, "{t}/{s}");
        }
    }
}
