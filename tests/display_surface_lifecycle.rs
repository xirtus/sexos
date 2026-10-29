// SPDX-License-Identifier: GPL-3.0-only
//! Production display registry and cleanup state; framebuffer commit is explicit,
//! so these tests do not claim guest pixel writes or kernel quiescence proof.
#![allow(dead_code)]
#[path = "../servers/sexdisplay/src/surface_lifecycle.rs"] mod production;
use production::*;
fn catalog(slot: usize, target: u32, sid: u64) -> Catalog {
    Catalog { slot, target_pd: target, task_id: 0x8000 + target, surface_id: sid, coordinator_pd: 3 }
}
fn state() -> Lifecycle { let mut l=Lifecycle::new(); assert!(l.register(catalog(38,15,300))); l }
fn create(l:&Lifecycle,s:&mut[Surface],caller:u32,sid:u64)->bool {
    upsert(s,l.admit(s,caller,sid),sid,10,60,100,80,0x123456)
}
#[test] fn foreign_existing_sid_does_not_allocate_duplicate() {
    let l=state(); let mut s=[SURFACE_EMPTY;4]; assert!(create(&l,&mut s,9,777));
    assert_eq!(l.admit(&s,12,777),Admission::Denied); assert!(!create(&l,&mut s,12,777));
    assert_eq!(s.iter().filter(|s|s.active).count(),1); assert_eq!(s[0].owner_pd,9);
}
#[test] fn coordinator_first_and_app_first_keep_catalog_owner() {
    for first in [3,15] { let l=state();let mut s=[SURFACE_EMPTY;4];assert!(create(&l,&mut s,first,300));
        assert_eq!(s[0].owner_pd,15); assert_eq!(l.admit(&s,3,300),Admission::Existing(0));
        assert_eq!(l.admit(&s,15,300),Admission::Existing(0));assert!(!create(&l,&mut s,9,300));
        assert_eq!(s.iter().filter(|s|s.active).count(),1);
    }
}
#[test] fn prepare_erases_all_owned_storage_and_scene_references() {
    let mut l=state();let mut s=[SURFACE_EMPTY;4];create(&l,&mut s,15,300);create(&l,&mut s,15,701);create(&l,&mut s,9,702);
    s[0].text_buf[0]=b'X';s[0].text_len=1;s[0].fill_count=3;
    // Legacy anonymous allocations remain attributable and collectible.
    s[3]=Surface{active:true,owner_pd:15,..SURFACE_EMPTY};
    let mut r=SceneRefs{focused_sid:701,active_app_sid:300,token_owner:15,token_pending:true,cursor_under_valid:true};
    assert_eq!(l.prepare(0,15,0x800f,&mut s,&mut r),Some(3));
    assert_eq!(r,SceneRefs::default());assert_eq!(s[0].text_buf,[0;TEXT_BUF_CAP]);assert_eq!(s[0].fill_count,0);
    assert!(s[2].active);assert_eq!(s[2].owner_pd,9);assert_eq!(l.phase(0),Some(Phase::Prepared));
}
#[test] fn no_ack_before_real_scene_commit_and_retry_is_idempotent() {
    let mut l=state();let mut s=[SURFACE_EMPTY;2];let mut r=SceneRefs::default();create(&l,&mut s,15,300);
    assert!(!l.acknowledge(0,15,0x800f)); l.prepare(0,15,0x800f,&mut s,&mut r).unwrap();
    for _ in 0..8 {assert!(!l.acknowledge(0,15,0x800f));assert_eq!(l.prepare(0,15,0x800f,&mut s,&mut r),None);}
    assert!(l.scene_committed(0));assert!(!l.acknowledge(0,15,15));assert!(!l.acknowledge(0,12,0x800f));
    // Failed syscall does not invoke acknowledge: committed resources remain fenced.
    assert_eq!(l.phase(0),Some(Phase::Committed));assert!(l.acknowledge(0,15,0x800f));
    assert!(!l.scene_committed(0));assert!(!l.acknowledge(0,15,0x800f));
}
#[test] fn stale_dead_calls_cannot_touch_reused_unrelated_sid() {
    let mut l=state();let mut s=[SURFACE_EMPTY;3];let mut r=SceneRefs::default();create(&l,&mut s,15,900);
    l.prepare(0,15,0x800f,&mut s,&mut r).unwrap();assert!(create(&l,&mut s,12,900));
    assert!(!l.caller_live(15));assert!(!create(&l,&mut s,15,900));assert!(!create(&l,&mut s,15,901));
    assert!(l.caller_live(12));assert_eq!(s[0].owner_pd,12);
    assert_eq!(l.prepare(0,15,0x800f,&mut s,&mut r),None);assert!(s[0].active);
}
#[test] fn delayed_shell_proxy_never_resurrects_catalog_surface() {
    let mut l=state();let mut s=[SURFACE_EMPTY;2];let mut r=SceneRefs::default();l.prepare(0,15,0x800f,&mut s,&mut r).unwrap();
    for caller in [0,3,9,15] {assert!(!create(&l,&mut s,caller,300));}
    assert!(!l.sid_live(300));assert!(create(&l,&mut s,3,205)); // independent browser placeholder
}
#[test] fn foreign_or_stale_attestation_has_no_effect() {
    let mut l=state();let mut s=[SURFACE_EMPTY;2];let mut r=SceneRefs{focused_sid:300,..SceneRefs::default()};create(&l,&mut s,15,300);
    for (target,task) in [(9,0x800f),(15,15),(15,0),(0,0x800f)] {assert_eq!(l.prepare(0,target,task,&mut s,&mut r),None);}
    assert!(s[0].active);assert_eq!(r.focused_sid,300);assert_eq!(l.phase(0),Some(Phase::Live));
}
#[test] fn catalog_uniqueness_and_coordinator_are_static() {
    let mut l=state();assert!(l.register(catalog(38,15,300)));assert!(!l.register(catalog(39,9,300)));
    assert!(!l.register(catalog(38,9,156)));assert!(!l.register(catalog(39,15,156)));
    let mut bad=catalog(39,9,156);bad.coordinator_pd=7;assert!(!l.register(bad));
    assert!(l.register(catalog(39,9,156)));assert!(l.register(catalog(40,12,157)));
    assert!(l.is_coordinator(3));assert!(!l.is_coordinator(9));assert!(!l.is_coordinator(0));
}
#[test] fn unrelated_target_remains_live_during_cleanup_and_ack() {
    let mut l=state();l.register(catalog(39,9,156));let mut s=[SURFACE_EMPTY;4];let mut r=SceneRefs{focused_sid:156,active_app_sid:156,..SceneRefs::default()};
    create(&l,&mut s,9,156);create(&l,&mut s,15,300);l.prepare(0,15,0x800f,&mut s,&mut r).unwrap();
    assert_eq!(r.focused_sid,156);assert_eq!(r.active_app_sid,156);assert!(l.sid_live(156));assert!(l.caller_live(9));
    assert_eq!(l.admit(&s,3,156),Admission::Existing(0));
}
#[test] fn full_registry_and_noop_geometry_preserve_state() {
    let l=state();let mut s=[SURFACE_EMPTY;1];assert!(create(&l,&mut s,9,700));
    assert!(!create(&l,&mut s,9,700));assert_eq!(l.admit(&s,12,701),Admission::Full);
    assert_eq!(l.admit(&s,12,700),Admission::Denied);assert_eq!(s[0].surface_id,700);
}
#[test] fn minimized_catalog_reference_is_cleared_without_active_surface() {
    let mut l=state();let mut s=[SURFACE_EMPTY;1];let mut r=SceneRefs{focused_sid:300,active_app_sid:300,token_owner:3,token_pending:true,cursor_under_valid:true};
    assert_eq!(l.prepare(0,15,0x800f,&mut s,&mut r),Some(0));assert_eq!(r.focused_sid,0);assert_eq!(r.active_app_sid,0);
    assert!(!r.cursor_under_valid);assert!(r.token_pending);assert_eq!(r.token_owner,3);
}
#[test] fn native_frame_aliases_require_exact_boot_subject_and_coordinator() {
    let mut l=Lifecycle::new();assert!(l.register(catalog(38,9,156)));assert!(l.register(catalog(39,12,154)));
    let mut s=[SURFACE_EMPTY;6];
    for sid in [201,153] {assert!(!create(&l,&mut s,15,sid));assert!(!create(&l,&mut s,9,sid));assert!(create(&l,&mut s,3,sid));}
    assert!(create(&l,&mut s,9,156));assert!(create(&l,&mut s,12,154));
    assert!(l.coordinator_controls(3,201));assert!(l.coordinator_controls(3,156));
    assert!(!l.coordinator_controls(15,201));assert!(!l.coordinator_controls(3,205));
    let mut r=SceneRefs{focused_sid:201,active_app_sid:201,..SceneRefs::default()};
    assert_eq!(l.prepare(0,9,0x8009,&mut s,&mut r),Some(2));
    assert_eq!(r.focused_sid,0);assert_eq!(r.active_app_sid,0);
    assert!(!l.sid_live(201));assert!(!l.sid_live(156));assert!(!l.coordinator_controls(3,201));
    assert!(l.sid_live(153));assert!(l.coordinator_controls(3,153));
    assert!(!create(&l,&mut s,3,201));assert!(!create(&l,&mut s,9,156));
    assert_eq!(s.iter().filter(|s|s.active).count(),2);
}
#[test] fn ordinary_coordinator_hide_permission_never_grants_foreign_destroy() {
    let l=state();assert!(l.coordinator_controls(3,300));assert!(!l.coordinator_controls(12,300));
    assert!(!l.coordinator_controls(3,777));assert!(!l.coordinator_controls(0,300));
}
#[test] fn inactive_owned_records_are_scrubbed_defensively() {
    let mut l=state();let mut s=[SURFACE_EMPTY;2];
    s[0].owner_pd=15;s[0].surface_id=700;s[0].text_buf[0]=b'S';s[0].text_len=1;s[0].fill_count=2;
    let mut refs=SceneRefs::default();assert_eq!(l.prepare(0,15,0x800f,&mut s,&mut refs),Some(1));
    assert_eq!(s[0].owner_pd,0);assert_eq!(s[0].surface_id,0);assert_eq!(s[0].text_len,0);
    assert_eq!(s[0].text_buf,[0;TEXT_BUF_CAP]);assert_eq!(s[0].fill_count,0);
}
#[test] fn presentation_requires_exact_owner_valid_id_and_completed_visible_pixels() {
    let mut l=state();
    assert!(!l.request_present(3,300,1,0));assert!(!l.request_present(12,300,1,0));
    assert!(!l.request_present(15,205,1,0));assert!(!l.request_present(15,300,0,0));
    assert!(!l.request_present(15,300,(u64::MAX>>8)+1,0));assert!(!l.request_present(15,300,1,1));
    assert!(l.request_present(15,300,1,0));assert!(l.needs_presentation());assert_eq!(l.presentation_reply(0),None);
    l.pixels_presented(0);l.pixels_presented(1|2);assert!(l.needs_presentation());assert_eq!(l.presentation_reply(0),None);
    l.pixels_presented(4);assert!(!l.needs_presentation());assert_eq!(l.presentation_reply(0),Some((15,256)));
}
#[test] fn full_reply_mailbox_retains_exact_request_and_prevents_replacement() {
    let mut l=state();assert!(l.request_present(15,300,7,0));assert!(l.request_present(15,300,7,0));
    assert!(!l.request_present(15,300,8,0));l.pixels_presented(4);
    for _ in 0..16 {assert_eq!(l.presentation_reply(0),Some((15,7<<8)));assert!(!l.request_present(15,300,8,0));}
    assert!(!l.presentation_reply_sent(0,8<<8));assert!(l.presentation_reply_sent(0,7<<8));assert_eq!(l.presentation_reply(0),None);
    assert!(l.request_present(15,300,8,0));assert!(l.needs_presentation());assert_eq!(l.presentation_reply(0),None);
}
#[test] fn duplicate_presentation_id_replays_but_new_id_needs_new_frame() {
    let mut l=state();l.request_present(15,300,4,0);l.pixels_presented(4);l.presentation_reply_sent(0,4<<8);
    assert!(l.request_present(15,300,4,0));assert_eq!(l.presentation_reply(0),Some((15,4<<8)));
    l.presentation_reply_sent(0,4<<8);assert!(!l.request_present(15,300,3,0));
    assert!(l.request_present(15,300,5,0));assert_eq!(l.presentation_reply(0),None);
}
#[test] fn death_cancels_waiting_or_unsent_presentation_before_service_ack() {
    for rendered in [false,true] {let mut l=state();l.request_present(15,300,1,0);if rendered{l.pixels_presented(4);}
        let mut s=[SURFACE_EMPTY;1];let mut refs=SceneRefs::default();l.prepare(0,15,0x800f,&mut s,&mut refs).unwrap();
        assert!(!l.needs_presentation());assert_eq!(l.presentation_reply(0),None);assert!(!l.request_present(15,300,1,0));
        l.pixels_presented(4);assert_eq!(l.presentation_reply(0),None);assert!(!l.presentation_reply_sent(0,256));
    }
}
#[test] fn native_pixel_bits_do_not_confuse_foreign_owners_or_shell_frame_aliases() {
    assert_eq!(native_pixel_bit(9,156),1);assert_eq!(native_pixel_bit(12,154),2);assert_eq!(native_pixel_bit(15,300),4);
    for (pd,sid) in [(3,300),(15,156),(9,201),(12,153),(17,160),(15,205)] {assert_eq!(native_pixel_bit(pd,sid),0);}
}
#[test] fn simultaneous_native_requests_complete_only_the_visible_subject() {
    let mut l=state();l.register(catalog(39,9,156));l.register(catalog(40,12,154));
    for (pd,sid,id) in [(15,300,1),(9,156,2),(12,154,3)] {assert!(l.request_present(pd,sid,id,0));}
    l.pixels_presented(1);assert_eq!(l.presentation_reply(0),None);assert_eq!(l.presentation_reply(1),Some((9,512)));assert_eq!(l.presentation_reply(2),None);
    l.pixels_presented(4|2);assert_eq!(l.presentation_reply(0),Some((15,256)));assert_eq!(l.presentation_reply(2),Some((12,768)));
}

#[test]fn spindle_successor_reserves_all_children_and_frame_without_reviving_predecessor(){
 let mut l=Lifecycle::new();assert!(l.register(catalog(39,12,154)));assert!(l.register(catalog(50,17,302)));
 let mut surfaces=[SURFACE_EMPTY;7];for sid in [302,303,304,305]{assert!(create(&l,&mut surfaces,17,sid));assert!(!create(&l,&mut surfaces,12,sid));}
 assert!(create(&l,&mut surfaces,3,306));assert!(!create(&l,&mut surfaces,17,306));
 let mut refs=SceneRefs::default();assert!(l.prepare(0,12,0x800c,&mut surfaces,&mut refs).is_some());
 for sid in [302,303,304,305]{assert!(l.sid_live(sid));}assert_eq!(surfaces.iter().filter(|s|s.active).count(),5);
 assert_eq!(l.prepare(1,17,0x8011,&mut surfaces,&mut refs),Some(5));
 for sid in [302,303,304,305,306]{assert!(!create(&l,&mut surfaces,3,sid));assert!(!create(&l,&mut surfaces,17,sid));}
}

#[test]fn collapsed_spindle_primary_can_present_only_via_its_exact_owned_group(){for sid in[154,160,161,162]{assert_eq!(native_pixel_bit(12,sid),2);assert_eq!(native_pixel_bit(17,sid),0);}for sid in[302,303,304,305]{assert_eq!(native_pixel_bit(17,sid),16);assert_eq!(native_pixel_bit(12,sid),0);}let mut l=Lifecycle::new();assert!(l.register(catalog(39,12,154)));assert!(l.request_present(12,154,7,0));l.pixels_presented(native_pixel_bit(12,161));assert_eq!(l.presentation_reply(0),Some((12,7<<8)));}

#[test]fn all_six_boot_catalogs_fit_and_third_quil_cannot_impersonate_old_surface(){
 let mut l=Lifecycle::new();
 for (slot,pd,sid) in [(38,9,156),(39,12,154),(40,15,300),(45,16,301),(50,17,302),(54,18,307)] {assert!(l.register(catalog(slot,pd,sid)));}
 assert!(!l.register(catalog(58,19,308)));
 let mut surfaces=[SURFACE_EMPTY;16];assert!(create(&l,&mut surfaces,18,307));
 assert!(!create(&l,&mut surfaces,16,307));assert!(!create(&l,&mut surfaces,18,301));
 assert_eq!(native_pixel_bit(18,307),32);assert_eq!(native_pixel_bit(16,307),0);
 let mut refs=SceneRefs::default();assert_eq!(l.prepare(5,18,0x8012,&mut surfaces,&mut refs),Some(1));
 assert!(!create(&l,&mut surfaces,18,307));assert!(!create(&l,&mut surfaces,3,307));
}
