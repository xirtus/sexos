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
fn catalog_targets_and_surfaces_equal_the_kernel_boot_plan() {
    let init = include_str!("../kernel/src/init.rs");
    let targets: Vec<String> = INCARNATIONS.iter().map(|i| i.target.to_string()).collect();
    assert!(init.contains(&format!("plan_boot_apps(&[{}])", targets.join(", "))));
    let names = ["quil_id", "spindle_id", "kaleido_id", "successor", "spindle_successor", "quil_third"];
    let pairs: Vec<String> = INCARNATIONS.iter().zip(names).map(|(i, n)| format!("({},{})", n, i.surface)).collect();
    assert!(init.contains(&format!("&[{}]", pairs.join(", "))), "kernel app_resources::init list differs");
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
    let old = |owner: u32, sid: u64| -> u8 { match (owner, sid) { (9,156) => 1, (12,154|160|161|162) => 2, (15,300) => 4, (16,301) => 8, (17,302|303|304|305) => 16, (18,307) => 32, _ => 0 } };
    for owner in 0..40u32 { for sid in 0..400u64 { assert_eq!(app_catalog::native_pixel_bit(owner, sid), old(owner, sid), "{owner}/{sid}"); } }
    const BIT: u8 = app_catalog::native_pixel_bit(17, 304);
    assert_eq!(BIT, 16, "usable in const context");
    for (target, surface, sid, want) in [(12,154,161,true),(12,154,303,false),(17,302,305,true),(9,156,201,false),(9,156,156,true)] {
        assert_eq!(app_catalog::is_content(target, surface, sid), want, "{target}/{surface}/{sid}");
    }
}
