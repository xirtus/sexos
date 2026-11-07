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
    assert_eq!(lifecycle, [38,39,40,45,50,54,110,114,118,122,126,130,134,138,142,146,150,154,158,162,166,170,174,178,182,186,190,194,240,244,248,252,420,424,428,432,436]);
    let closes: Vec<u64> = INCARNATIONS.iter().map(|i| i.close_slot).collect();
    assert_eq!(&closes[..3], &app_lifecycle::CLOSE_SLOTS.map(|s| s as u64));
    assert_eq!(closes[3..6], [app_lifecycle::SUCCESSOR_CLOSE_SLOT, app_lifecycle::SPINDLE_CLOSE_SLOT, app_lifecycle::QUIL_THIRD_CLOSE_SLOT]);
    assert_eq!(closes[6..], [111, 115, 119, 123, 127, 131, 135, 139, 143, 147, 151,155,159,163,167,171,175,179,183,187,191,195,241,245,249,253,421,425,429,433,437]);
    let activations: Vec<Option<usize>> = INCARNATIONS.iter().map(|i| i.activation_slot).collect();
    assert_eq!(activations, [None, None, None, Some(app_lifecycle::ACTIVATION_SLOT), Some(app_lifecycle::SPINDLE_ACTIVATION_SLOT), Some(app_lifecycle::QUIL_THIRD_ACTIVATION_SLOT), Some(112), Some(116), Some(120), Some(124), Some(128), Some(132), Some(136), Some(140), Some(144), Some(148), Some(152),Some(156),Some(160),Some(164),Some(168),Some(172),Some(176),Some(180),Some(184),Some(188),Some(192),Some(196), None, Some(246), Some(250), Some(254), Some(422), Some(426), Some(430), Some(434), Some(438)]);
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
    assert_eq!(app_catalog::TARGETS, [9, 12, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49]);
    assert_eq!(app_catalog::CONTENT, [(9,156), (12,154), (15,300), (16,301), (17,302), (18,307), (19,308), (20,309), (21,310), (22,311), (23,312), (24,313), (25,318), (26,323), (27,328), (28,333), (29,338),(30,343),(31,344),(32,345),(33,346),(34,347),(35,348),(36,349),(37,350),(38,351),(39,352),(40,353),(41,358),(42,359),(43,360),(44,361),(45,362),(46,363),(47,364),(48,365),(49,366)]);
    let chain: Vec<(u32, u32)> = INCARNATIONS.iter().filter(|i| i.ordinal != 0).map(|i| (app_catalog::predecessor(i).unwrap().target, i.target)).collect();
    assert_eq!(chain, [(9, 16), (12, 17), (16, 18), (18, 19), (19, 20), (20, 21), (21, 22), (22, 23), (17, 24), (24,25), (25,26), (26,27), (27,28), (28,29),(15,30),(30,31),(23,32),(32,33),(33,34),(34,35),(35,36),(36,37),(37,38),(38,39),(29,40),(41,42),(42,43),(43,44),(44,45),(45,46),(46,47),(47,48),(48,49)]);
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
    assert!(by_target(0).is_none() && by_target(3).is_none() && by_target(50).is_none() && by_target(u32::MAX).is_none());
    assert!([9, 16, 18, 19, 20, 21, 22, 23,32,33,34,35,36,37,38,39].iter().all(|&t| is_document_app(t)) && ![12, 15, 17, 24, 25, 26, 27, 28, 29,40,30,31, 0].iter().any(|&t| is_document_app(t)));
}

#[test]
fn shell_view_tables_preserve_old_rows_and_append_reserved_quils() {
    // Shell delivery is positional: all sixteen Quils, then three Browsers.
    let docs: Vec<(u32, u64, u64)> = app_catalog::document_views().map(|i| (i.target, i.frame(), i.route_slot)).collect();
    assert_eq!(docs, [(9, 201, 11), (16, 301, app_lifecycle::SUCCESSOR_ROUTE_SLOT), (18, 307, app_lifecycle::QUIL_THIRD_ROUTE_SLOT), (19,308,113), (20,309,117), (21,310,121), (22,311,125), (23,312,129),(32,345,165),(33,346,169),(34,347,173),(35,348,177),(36,349,181),(37,350,185),(38,351,189),(39,352,193), (15, 300, app_lifecycle::CLOSE_SLOTS[2] as u64),(30,343,157),(31,344,161)]);
    let spindle: Vec<(u32, u64, u64)> = app_catalog::spindle_views().map(|i| (i.target, i.frame(), i.route_slot)).collect();
    assert_eq!(spindle, [(12, 153, 14), (17, 306, app_lifecycle::SPINDLE_ROUTE_SLOT), (24,314,133), (25,319,137), (26,324,141), (27,329,145), (28,334,149), (29,339,153),(40,354,197)]);
    assert_eq!(app_catalog::document_view_frame(201).map(|(i, v)| (i, v.target)), Some((0, 9)));
    assert!(app_catalog::document_view_frame(156).is_none(), "content surface is not a frame");
    let pdx = include_str!("../crates/sex-pdx/src/lib.rs");
    assert!(pdx.contains("pub const SLOT_QUIL: u64 = 11;") && pdx.contains("pub const SLOT_SPINDLE: u64 = 14;"));
}

#[test]
fn display_tables_preserve_old_bits_and_assign_exact_new_bits() {
    let old = |owner: u32, sid: u64| -> u64 { match (owner, sid) { (41,358) => 1<<28, (42,359) => 1<<29, (43,360) => 1<<30, (44,361) => 1<<31, (45,362) => 1<<32, (46,363) => 1<<33, (47,364) => 1<<34, (48,365) => 1<<35, (49,366) => 1<<36, (9,156) => 1, (12,154|160|161|162) => 2, (15,300) => 4, (16,301) => 8, (17,302|303|304|305) => 16, (18,307) => 32, (19,308) => 64, (20,309) => 128, (21,310) => 256, (22,311) => 512, (23,312) => 1024, (24,313|315|316|317) => 2048, (25,318|320|321|322) => 4096, (26,323|325|326|327) => 8192, (27,328|330|331|332) => 16384, (28,333|335|336|337) => 32768, (29,338|340|341|342) => 65536, (30,343) => 131072, (31,344) => 262144, (32,345) => 524288, (33,346) => 1048576, (34,347) => 2097152, (35,348) => 4194304, (36,349) => 8388608, (37,350) => 16777216, (38,351) => 33554432, (39,352) => 67108864, (40,353|355|356|357) => 134217728, _ => 0 } };
    for owner in 0..48u32 { for sid in 0..400u64 { assert_eq!(app_catalog::native_pixel_bit(owner, sid), old(owner, sid), "{owner}/{sid}"); } }
    const BIT: u64 = app_catalog::native_pixel_bit(17, 304);
    assert_eq!(BIT, 16, "usable in const context");
    for (target, surface, sid, want) in [(12,154,161,true),(12,154,303,false),(17,302,305,true),(9,156,201,false),(9,156,156,true)] {
        assert_eq!(app_catalog::is_content(target, surface, sid), want, "{target}/{surface}/{sid}");
    }
}

#[test]
fn sex_pdx_policy_helpers_match_explicit_twenty_eight_subject_sets() {
    for t in 0..48u32 {
        assert_eq!(app_catalog::is_quil(t), matches!(t, 9 | 16 | 18 | 19 | 20 | 21 | 22 | 23 | 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39), "{t}");
        assert_eq!(app_catalog::is_spindle(t), matches!(t, 12 | 17 | 24 | 25 | 26 | 27 | 28 | 29 |40), "{t}");
        assert_eq!(app_catalog::is_browser(t), matches!(t,15|30|31), "{t}");
        // document_view: frameless document holders used a 36-cell top band.
        let frameless_doc = app_catalog::app_of(t).is_some() && !app_catalog::has_shell_frame(t) && !app_catalog::is_spindle(t);
        assert_eq!(frameless_doc, matches!(t, 15 | 16 | 18 | 19 | 20 | 21 | 22 | 23 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 41 | 42 | 43 | 44 | 45 | 46 | 47 | 48 | 49), "{t}");
        for s in 0..400u64 {
            // desktop_style::native_canvas
            let native = app_catalog::native_pixel_bit(t, s) != 0 && !app_catalog::is_browser(t);
            let old = matches!((t, s), (41,358)|(42,359)|(43,360)|(44,361)|(45,362)|(46,363)|(47,364)|(48,365)|(49,366)|(9,156)|(16,301)|(18,307)|(19,308)|(20,309)|(21,310)|(22,311)|(23,312)|(12,154|160|161|162)|(17,302|303|304|305)|(24,313|315|316|317)|(25,318|320|321|322)|(26,323|325|326|327)|(27,328|330|331|332)|(28,333|335|336|337)|(29,338|340|341|342)|(32,345)|(33,346)|(34,347)|(35,348)|(36,349)|(37,350)|(38,351)|(39,352)|(40,353|355|356|357));
            assert_eq!(native, old, "{t}/{s}");
            // clipboard::Participant routes
            let new = by_surface(t, s).filter(|i| i.app != App::Spindle && i.app != App::Program).map(|i| (i.close_slot, i.app == App::Quil));
            let old = match (t, s) { (9,156)=>Some((41,true)), (15,300)=>Some((43,false)), (16,301)=>Some((46,true)), (18,307)=>Some((56,true)), (19,308)=>Some((111,true)), (20,309)=>Some((115,true)), (21,310)=>Some((119,true)), (22,311)=>Some((123,true)), (23,312)=>Some((127,true)), (30,343)=>Some((155,false)), (31,344)=>Some((159,false)), (32,345)=>Some((163,true)), (33,346)=>Some((167,true)), (34,347)=>Some((171,true)), (35,348)=>Some((175,true)), (36,349)=>Some((179,true)), (37,350)=>Some((183,true)), (38,351)=>Some((187,true)), (39,352)=>Some((191,true)), _=>None };
            assert_eq!(new, old, "{t}/{s}");
        }
    }
}

#[test]
fn exact_new_reserve_rows_and_last_identity_have_no_aliases() {
    // Independent goldens: metadata is not permission to add more subjects.
    let expected = [
        (3,19,308,110,111,112,113), (4,20,309,114,115,116,117),
        (5,21,310,118,119,120,121), (6,22,311,122,123,124,125),
        (7,23,312,126,127,128,129),
    ];
    for (ordinal,target,surface,lifecycle,close,activation,route) in expected {
        let row = by_target(target).unwrap();
        assert_eq!((row.app,row.ordinal,row.surface,row.shell_surface,row.lifecycle_slot,row.close_slot,row.activation_slot,row.route_slot),
                   (App::Quil,ordinal,surface,None,lifecycle,close,Some(activation),route));
        assert!(row.children.is_empty());
    }
    assert_eq!(app_catalog::QUIL_TARGETS, [9,16,18,19,20,21,22,23,32,33,34,35,36,37,38,39]);
    assert_eq!(app_catalog::document_view_frame(312).map(|(index,row)|(index,row.target)), Some((7,23)));
    assert_eq!(app_catalog::native_pixel_bit(23,312), 1024);
    for (target,surface) in [(23,311),(22,312),(24,312),(23,312+(1u64<<32)),(23+(1u32<<31),312)] {
        assert!(by_surface(target,surface).is_none());
        assert_eq!(app_catalog::native_pixel_bit(target,surface),0);
        assert!(!app_catalog::is_content(target,surface,surface));
    }
    // Exactly sixteen BOOT-reserved Quil instances; no implicit next reserve.
    assert_eq!(app_catalog::count(App::Quil),16);
    assert!(by_target(50).is_none());
}

#[test]fn app_surface_inventory_covers_all_frames_content_and_panes_once(){
 assert_eq!(app_catalog::APP_SURFACE_COUNT,74);
 assert_eq!(1+app_catalog::APP_SURFACE_COUNT,75);
 assert_eq!(app_catalog::APP_SURFACES,[201,153,300,301,306,307,308,309,310,311,312,314,319,324,329,334,339,343,344,345,346,347,348,349,350,351,352,354,358,359,360,361,362,363,364,365,366,156,154,160,161,162,302,303,304,305,313,315,316,317,318,320,321,322,323,325,326,327,328,330,331,332,333,335,336,337,338,340,341,342,353,355,356,357]);
 for(i,sid)in app_catalog::APP_SURFACES.iter().enumerate(){assert!(!app_catalog::APP_SURFACES[..i].contains(sid));}
}


#[test]fn third_spindle_has_exact_frame_grid_routes_lineage_and_new_pixel_bit(){
 let row=by_target(24).unwrap();
 assert_eq!((row.app,row.ordinal,row.target,row.surface,row.shell_surface,row.lifecycle_slot,row.close_slot,row.activation_slot,row.route_slot),
  (App::Spindle,2,24,313,Some(314),130,131,Some(132),133));
 assert_eq!(row.children,[315,316,317]);assert_eq!(row.frame(),314);
 assert_eq!(app_catalog::predecessor(row).map(|row|row.target),Some(17));
 assert_eq!(by_surface(24,313).map(|row|row.frame()),Some(314));
 assert_eq!(app_catalog::spindle_grid(24),Some([313,315,316,317]));
 assert_eq!(app_catalog::SPINDLE_COUNT,9);assert_eq!(app_catalog::SPINDLE_TARGETS,[12,17,24,25,26,27,28,29,40]);
 assert_eq!(app_catalog::QUIL_COUNT,16);assert_eq!(app_catalog::DOCUMENT_COUNT,19);
 assert_eq!(app_catalog::DOCUMENT_TARGETS,[9,16,18,19,20,21,22,23,32,33,34,35,36,37,38,39,15,30,31]);
 assert_eq!(app_catalog::count(App::Browser),3);
 const THIRD_BIT:u64=app_catalog::native_pixel_bit(24,317);assert_eq!(THIRD_BIT,2048);
 for sid in [313,315,316,317]{assert_eq!(app_catalog::native_pixel_bit(24,sid),2048);assert!(app_catalog::is_content(24,313,sid));}
 assert_eq!(app_catalog::native_pixel_bit(24,314),0);assert!(!app_catalog::is_content(24,313,314));
 for(target,surface,sid)in[(24,313,305),(17,302,315),(24,302,313),(25,313,313),
  (24|(1u32<<31),313,313),(24,313|(1u64<<32),313),(24,313,315|(1u64<<32))]{
  assert!(!app_catalog::is_content(target,surface,sid));
 }
 // Pixel metadata accepts an exact owner/content pair, not a parent-surface argument.
 for(owner,sid)in[(24,305),(17,315),(25,313),(24|(1u32<<31),313),
  (24,313|(1u64<<32)),(24,315|(1u64<<32))]{assert_eq!(app_catalog::native_pixel_bit(owner,sid),0);}
 assert!(app_catalog::document_view_index(24).is_none());assert!(!is_document_app(24));
 assert!(by_target(50).is_none());
}

#[test]
fn eight_spindle_chain_has_exact_independent_groups_and_last_bit() {
    let goldens = [
        (3,25,318,319,[320,321,322],134,135,136,137,24,4096),
        (4,26,323,324,[325,326,327],138,139,140,141,25,8192),
        (5,27,328,329,[330,331,332],142,143,144,145,26,16384),
        (6,28,333,334,[335,336,337],146,147,148,149,27,32768),
        (7,29,338,339,[340,341,342],150,151,152,153,28,65536),
    ];
    for (ordinal,target,content,frame,children,life,close,activation,route,parent,bit) in goldens {
        let r=by_target(target).unwrap();
        assert_eq!((r.app,r.ordinal,r.surface,r.frame(),r.lifecycle_slot,r.close_slot,r.activation_slot,r.route_slot),
            (App::Spindle,ordinal,content,frame,life,close,Some(activation),route));
        assert_eq!(r.children,children);
        assert_eq!(app_catalog::predecessor(r).map(|p|p.target),Some(parent));
        assert_eq!(app_catalog::spindle_grid(target),Some([content,children[0],children[1],children[2]]));
        for sid in [content,children[0],children[1],children[2]] {
            assert_eq!(app_catalog::native_pixel_bit(target,sid),bit);
            assert!(app_catalog::is_content(target,content,sid));
            assert!(!app_catalog::is_content(target,frame,sid));
            assert!(!app_catalog::is_content(parent,content,sid));
            assert_eq!(app_catalog::native_pixel_bit(parent,sid),0);
            assert_eq!(app_catalog::native_pixel_bit(target,sid|(1u64<<32)),0);
            assert_eq!(app_catalog::native_pixel_bit(target|(1u32<<31),sid),0);
        }
        assert_eq!(app_catalog::native_pixel_bit(target,frame),0);
        assert!(!app_catalog::is_content(target,content,frame));
        assert!(!is_document_app(target));
        assert!(app_catalog::document_view_index(target).is_none());
    }
    const LAST:u64=app_catalog::native_pixel_bit(29,342);
    assert_eq!(LAST,1<<16);
    assert_eq!(INCARNATIONS.len(),37);
    assert!(by_target(50).is_none());
}

#[test]
fn sixteen_quil_nine_spindle_append_exact_rows_without_reusing_old_slots_or_bits() {
    // These are independently spelled BOOT metadata expectations, not grants.
    let new_quils=[
        (8,32,345,162,163,164,165,23,524288),
        (9,33,346,166,167,168,169,32,1048576),
        (10,34,347,170,171,172,173,33,2097152),
        (11,35,348,174,175,176,177,34,4194304),
        (12,36,349,178,179,180,181,35,8388608),
        (13,37,350,182,183,184,185,36,16777216),
        (14,38,351,186,187,188,189,37,33554432),
        (15,39,352,190,191,192,193,38,67108864),
    ];
    for (ordinal,target,surface,life,close,activation,route,parent,bit) in new_quils {
        let r=by_target(target).unwrap();
        assert_eq!((r.app,r.ordinal,r.surface,r.shell_surface,r.lifecycle_slot,r.close_slot,r.activation_slot,r.route_slot),
                   (App::Quil,ordinal,surface,None,life,close,Some(activation),route));
        assert!(r.children.is_empty());
        assert_eq!(app_catalog::predecessor(r).map(|p|p.target),Some(parent));
        assert_eq!(app_catalog::document_view_frame(surface).map(|(i,r)|(i,r.target)),Some((ordinal as usize,target)));
        assert_eq!(app_catalog::native_pixel_bit(target,surface),bit);
        for (wrong_target,wrong_surface) in [(parent,surface),(target,surface+1),(target|(1<<31),surface),(target,surface|(1u64<<32))] {
            assert!(by_surface(wrong_target,wrong_surface).is_none());
            assert_eq!(app_catalog::native_pixel_bit(wrong_target,wrong_surface),0);
        }
    }
    let last=by_target(40).unwrap();
    assert_eq!((last.app,last.ordinal,last.surface,last.shell_surface,last.lifecycle_slot,last.close_slot,last.activation_slot,last.route_slot),
               (App::Spindle,8,353,Some(354),194,195,Some(196),197));
    assert_eq!(last.children,[355,356,357]);
    assert_eq!(app_catalog::predecessor(last).map(|p|p.target),Some(29));
    assert_eq!(app_catalog::spindle_grid(40),Some([353,355,356,357]));
    for sid in [353,355,356,357] {
        assert_eq!(app_catalog::native_pixel_bit(40,sid),134217728);
        assert!(app_catalog::is_content(40,353,sid));
        assert!(!app_catalog::is_content(40,354,sid));
        assert_eq!(app_catalog::native_pixel_bit(29,sid),0);
        assert_eq!(app_catalog::native_pixel_bit(40,sid|(1u64<<32)),0);
    }
    assert_eq!(app_catalog::native_pixel_bit(40,354),0);
    assert!(by_surface(40,354).is_none());
    assert_eq!(app_catalog::native_pixel_bit(40|(1u32<<31),353),0);
    assert!(by_target(50).is_none());
    // The 17 original Shell metadata entries include original Quil/Spindle.
    // Every later reserve and original Browser retains one additional entry.
    let retained:Vec<_>=INCARNATIONS.iter().filter(|r|r.ordinal>0||matches!(r.app,App::Browser|App::Program)).map(|r|r.frame()).collect();
    assert_eq!(retained,[300,301,306,307,308,309,310,311,312,314,319,324,329,334,339,343,344,345,346,347,348,349,350,351,352,354,358,359,360,361,362,363,364,365,366]);
    assert_eq!(17+retained.len(),52);
    let mut slots=std::collections::HashMap::new();
    for r in INCARNATIONS {
        for slot in [Some(r.lifecycle_slot as u64),Some(r.close_slot),r.activation_slot.map(|s|s as u64),Some(r.route_slot)].into_iter().flatten() {
            assert!((1..=1024).contains(&slot));
            if let Some(prior)=slots.insert(slot,r.target) {assert_eq!((prior,r.target,slot),(15,15,43));}
        }
    }
}
