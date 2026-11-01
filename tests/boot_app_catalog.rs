// SPDX-License-Identifier: GPL-3.0-only
//! Actual pure BOOT validator over explicit twenty-eight-subject kernel observations.
//! These are host observations, not native grant extraction or task admission.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/app_catalog.rs"]
pub mod app_catalog;
#[path = "../kernel/src/app_resource_state.rs"]
pub mod app_resource_state;
#[path = "../kernel/src/app_catalog_state.rs"]
mod production;
use app_catalog::{App, Incarnation, INCARNATIONS};
use app_resource_state::{ActivationGrant, LifecycleGrant, ACK, QUERY, RETIRE};
use production::{validate, validate_layout, Error, Observed};

fn observed(target: u32) -> Observed {
    // Explicit existing BOOT identities; not derived through the validator.
    let (content_surface, logical_app, incarnation, predecessor, dormant) = match target {
        9 => (156, 9, 1, 0, false),
        12 => (154, 12, 1, 0, false),
        15 => (300, 15, 1, 0, false),
        16 => (301, 9, 2, 9, true),
        17 => (302, 12, 2, 12, true),
        18 => (307, 9, 3, 16, true),
        19 => (308, 9, 4, 18, true),
        20 => (309, 9, 5, 19, true),
        21 => (310, 9, 6, 20, true),
        22 => (311, 9, 7, 21, true),
        23 => (312, 9, 8, 22, true),
        24 => (313, 12, 3, 17, true),
        25 => (318, 12, 4, 24, true),
        26 => (323, 12, 5, 25, true),
        27 => (328, 12, 6, 26, true),
        28 => (333, 12, 7, 27, true),
        29 => (338, 12, 8, 28, true),
        30 => (343, 15, 2, 15, true),
        31 => (344, 15, 3, 30, true),
        32 => (345, 9, 9, 23, true),
        33 => (346, 9, 10, 32, true),
        34 => (347, 9, 11, 33, true),
        35 => (348, 9, 12, 34, true),
        36 => (349, 9, 13, 35, true),
        37 => (350, 9, 14, 36, true),
        38 => (351, 9, 15, 37, true),
        39 => (352, 9, 16, 38, true),
        40 => (353, 12, 9, 29, true),
        _ => panic!("not a current BOOT subject"),
    };
    Observed {
        target, task: target, task_domain: target, task_domain_pointer_matches: true,
        content_surface, logical_app, incarnation,
        predecessor, predecessor_registered_task: predecessor, coordinator: 3, dormant, dead: false,
        shell_lifecycle: LifecycleGrant {
            target_pd: target, task_id: target, holder_pd: 3, rights: QUERY | ACK, ack_bit: 1,
        },
        self_catalog: LifecycleGrant {
            target_pd: target, task_id: target, holder_pd: target, rights: QUERY, ack_bit: 0,
        },
        close_target: target, route_target: target,
        activation: (predecessor != 0).then_some(ActivationGrant {
            holder: 3, logical_app, incarnation, predecessor, predecessor_task: predecessor,
            successor: target, successor_task: target,
        }),
    }
}

fn row(target: u32) -> &'static Incarnation {
    INCARNATIONS.iter().find(|row| row.target == target).unwrap()
}

#[test]
fn all_twenty_eight_current_boot_subjects_validate_without_changing_observations() {
    assert_eq!(INCARNATIONS.map(|row| row.target), [9, 12, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40]);
    assert_eq!(validate_layout(&INCARNATIONS), Ok(()));
    for row in &INCARNATIONS {
        let actual = observed(row.target);
        let retained = actual;
        assert_eq!(validate(row, &actual, 3), Ok(()), "{}", row.target);
        assert_eq!(actual, retained);
    }
}

#[test]
fn actual_identity_surface_and_lineage_mismatches_are_distinct() {
    let changes: [(fn(&mut Observed), Error); 7] = [
        (|o| o.target = 12, Error::Target),
        (|o| o.task = 12, Error::ShellLifecycleTask),
        (|o| o.task = 0, Error::Task),
        (|o| o.content_surface = 156, Error::ContentSurface),
        (|o| o.logical_app = 16, Error::LogicalApp),
        (|o| o.incarnation = 1, Error::IncarnationNumber),
        (|o| o.predecessor = 12, Error::Predecessor),
    ];
    for (change, expected) in changes {
        let mut actual = observed(16);change(&mut actual);
        assert_eq!(validate(row(16), &actual, 3), Err(expected));
    }
    let mut actual = observed(18);actual.predecessor = 9;
    assert_eq!(validate(row(18), &actual, 3), Err(Error::Predecessor), "third Quil succeeds16, not logical9");
}

#[test]
fn shell_identity_cannot_be_changed_by_matching_a_forged_coordinator_argument() {
    let mut actual = observed(9);actual.coordinator = 4;
    assert_eq!(validate(row(9), &actual, 3), Err(Error::Coordinator));
    assert_eq!(validate(row(9), &actual, 4), Err(Error::Coordinator));
    assert_eq!(validate(row(9), &observed(9), 0), Err(Error::Coordinator));
}

#[test]
fn dormant_original_live_successor_and_every_dead_subject_reject() {
    for row in &INCARNATIONS {
        let mut actual = observed(row.target);actual.dormant = !actual.dormant;
        assert_eq!(validate(row, &actual, 3), Err(Error::Dormant));
        let mut actual = observed(row.target);actual.dead = true;
        assert_eq!(validate(row, &actual, 3), Err(Error::Dead));
    }
}

#[test]
fn shell_lifecycle_grant_requires_each_exact_field() {
    let changes: [(fn(&mut LifecycleGrant), Error); 7] = [
        (|g| g.target_pd = 18, Error::ShellLifecycleTarget),
        (|g| g.task_id = 18, Error::ShellLifecycleTask),
        (|g| g.holder_pd = 9, Error::ShellLifecycleHolder),
        (|g| g.rights = QUERY, Error::ShellLifecycleRights),
        (|g| g.rights = QUERY | ACK | RETIRE, Error::ShellLifecycleRights),
        (|g| g.ack_bit = 0, Error::ShellLifecycleAck),
        (|g| g.ack_bit = 2, Error::ShellLifecycleAck),
    ];
    for (change, expected) in changes {
        let mut actual = observed(9);change(&mut actual.shell_lifecycle);
        assert_eq!(validate(row(9), &actual, 3), Err(expected));
    }
}

#[test]
fn a_more_powerful_grant_does_not_pass_as_the_exact_boot_grant() {
    let mut actual = observed(9);actual.shell_lifecycle.rights |= RETIRE;
    assert_eq!(actual.shell_lifecycle.authorize(3, 9, 9, QUERY), Ok(()));
    assert_eq!(validate(row(9), &actual, 3), Err(Error::ShellLifecycleRights));
    actual = observed(9);actual.self_catalog.rights |= ACK;
    assert_eq!(actual.self_catalog.authorize(9, 9, 9, QUERY), Ok(()));
    assert_eq!(validate(row(9), &actual, 3), Err(Error::SelfCatalogRights));
}

#[test]
fn self_catalog_grant_requires_each_exact_field_and_no_ack_authority() {
    let changes: [(fn(&mut LifecycleGrant), Error); 6] = [
        (|g| g.target_pd = 9, Error::SelfCatalogTarget),
        (|g| g.task_id = 9, Error::SelfCatalogTask),
        (|g| g.holder_pd = 3, Error::SelfCatalogHolder),
        (|g| g.rights = QUERY | ACK, Error::SelfCatalogRights),
        (|g| g.rights = 0, Error::SelfCatalogRights),
        (|g| g.ack_bit = 1, Error::SelfCatalogAck),
    ];
    for (change, expected) in changes {
        let mut actual = observed(16);change(&mut actual.self_catalog);
        assert_eq!(validate(row(16), &actual, 3), Err(expected));
    }
}

#[test]
fn domain_routes_cannot_target_another_incarnation() {
    let mut actual = observed(16);actual.close_target = 9;
    assert_eq!(validate(row(16), &actual, 3), Err(Error::CloseTarget));
    let mut actual = observed(18);actual.route_target = 16;
    assert_eq!(validate(row(18), &actual, 3), Err(Error::RouteTarget));
}

#[test]
fn activation_presence_is_exact_for_originals_and_successors() {
    for target in [16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40] {
        let mut actual = observed(target);actual.activation = None;
        assert_eq!(validate(row(target), &actual, 3), Err(Error::MissingActivation));
    }
    for target in [9, 12, 15] {
        let mut actual = observed(target);actual.activation = observed(16).activation;
        assert_eq!(validate(row(target), &actual, 3), Err(Error::UnexpectedActivation));
    }
}

#[test]
fn every_activation_binding_is_compared_without_logical_or_task_aliases() {
    let changes: [(fn(&mut ActivationGrant), Error); 7] = [
        (|g| g.holder = 9, Error::ActivationHolder),
        (|g| g.logical_app = 16, Error::ActivationLogicalApp),
        (|g| g.incarnation = 2, Error::ActivationIncarnation),
        (|g| g.predecessor = 9, Error::ActivationPredecessor),
        (|g| g.predecessor_task = 9, Error::ActivationPredecessorTask),
        (|g| g.successor = 16, Error::ActivationSuccessor),
        (|g| g.successor_task = 16, Error::ActivationSuccessorTask),
    ];
    for (change, expected) in changes {
        let mut actual = observed(18);change(actual.activation.as_mut().unwrap());
        assert_eq!(validate(row(18), &actual, 3), Err(expected));
    }
}

#[test]
fn fabricated_catalog_rows_are_not_new_authority() {
    let mut fabricated = *row(9);fabricated.target = 99;
    assert_eq!(validate(&fabricated, &observed(9), 3), Err(Error::CatalogRow));
    fabricated = *row(9);fabricated.close_slot = 46;
    assert_eq!(validate(&fabricated, &observed(9), 3), Err(Error::CatalogRow));
}

#[test]
fn malformed_catalog_identity_order_and_slot_layouts_reject() {
    let mutations: [(fn(&mut [Incarnation; 28]), Error); 11] = [
        (|r| r[1].target = 9, Error::CatalogTarget),
        (|r| r[0].target = 0, Error::CatalogTarget),
        (|r| r[0].target = 3, Error::CatalogTarget),
        (|r| r[1].surface = 156, Error::CatalogSurface),
        (|r| r[0].surface = 0, Error::CatalogSurface),
        (|r| r[3].ordinal = 2, Error::CatalogOrder),
        (|r| r[3].activation_slot = None, Error::CatalogActivationSlot),
        (|r| r[0].activation_slot = Some(60), Error::CatalogActivationSlot),
        (|r| r[0].route_slot = 0, Error::CatalogSlot),
        (|r| r[0].close_slot = 38, Error::CatalogSlotAlias),
        (|r| r[3].activation_slot = Some(39), Error::CatalogSlotAlias),
    ];
    for (change, expected) in mutations {
        let mut rows = INCARNATIONS;change(&mut rows);
        assert_eq!(validate_layout(&rows), Err(expected));
    }
    assert_eq!(validate_layout(&INCARNATIONS[..5]), Err(Error::CatalogLength));
    let mut rows = INCARNATIONS;for row in &mut rows{if row.app==App::Browser{row.app=App::Quil;}}
    assert_eq!(validate_layout(&rows), Err(Error::CatalogMissingApp));
    let mut rows = INCARNATIONS;rows[0].route_slot = u32::MAX as u64 + 1;
    assert_eq!(validate_layout(&rows), Err(Error::CatalogSlot));
}

#[test]
fn same_subject_domain_route_alias_is_allowed_but_cross_subject_slot_alias_is_not() {
    let browser = row(15);
    assert_eq!((browser.close_slot, browser.route_slot), (43, 43));
    assert_eq!(validate(browser, &observed(15), 3), Ok(()));
    let mut rows = INCARNATIONS;rows[3].route_slot = 43;
    assert_eq!(validate_layout(&rows), Err(Error::CatalogSlotAlias));
}

#[test]
fn matching_task_number_does_not_authorize_a_different_execution_domain() {
    for target in [9, 16, 18, 19, 20, 21, 22, 23] {
        let mut actual = observed(target);actual.task_domain = 12;
        assert_eq!(validate(row(target), &actual, 3), Err(Error::TaskDomain));
        let mut actual = observed(target);actual.task_domain_pointer_matches = false;
        assert_eq!(validate(row(target), &actual, 3), Err(Error::TaskDomainPointer));
    }
}

#[test]
fn final_reserve_requires_exact_eighth_lineage_task_and_routes() {
    assert_eq!(validate(row(23), &observed(23), 3), Ok(()));
    let changes: [(fn(&mut Observed), Error); 9] = [
        (|o| o.target = 23 | (1 << 31), Error::Target),
        (|o| o.task = 23 | (1 << 31), Error::ShellLifecycleTask),
        (|o| o.content_surface = 312 | (1u64 << 32), Error::ContentSurface),
        (|o| o.incarnation = 7, Error::IncarnationNumber),
        (|o| o.predecessor = 18, Error::Predecessor),
        (|o| o.close_target = 22, Error::CloseTarget),
        (|o| o.route_target = 22, Error::RouteTarget),
        (|o| o.activation.as_mut().unwrap().predecessor_task = 18, Error::ActivationPredecessorTask),
        (|o| o.activation.as_mut().unwrap().successor_task = 23 | (1 << 31), Error::ActivationSuccessorTask),
    ];
    for (change,error) in changes {
        let mut actual=observed(23);change(&mut actual);
        assert_eq!(validate(row(23),&actual,3),Err(error));
    }
    let mut rows=INCARNATIONS;rows[10].route_slot=129 | (1u64 << 32);
    assert_eq!(validate_layout(&rows),Err(Error::CatalogSlot));
    let mut rows=INCARNATIONS;rows[10].activation_slot=Some(112);
    assert_eq!(validate_layout(&rows),Err(Error::CatalogSlotAlias));
    let mut rows=INCARNATIONS;rows[10].ordinal=8;
    assert_eq!(validate_layout(&rows),Err(Error::CatalogOrder));
}


#[test]
fn third_spindle_actual_grants_require_exact_lineage_task_routes_and_no_high_bit_aliases(){
 let actual=observed(24);
 assert_eq!((actual.target,actual.task,actual.content_surface,actual.logical_app,actual.incarnation,actual.predecessor,actual.dormant),(24,24,313,12,3,17,true));
 assert_eq!(actual.shell_lifecycle,LifecycleGrant{target_pd:24,task_id:24,holder_pd:3,rights:QUERY|ACK,ack_bit:1});
 assert_eq!(actual.self_catalog,LifecycleGrant{target_pd:24,task_id:24,holder_pd:24,rights:QUERY,ack_bit:0});
 assert_eq!((actual.close_target,actual.route_target),(24,24));
 assert_eq!(actual.activation,Some(ActivationGrant{holder:3,logical_app:12,incarnation:3,predecessor:17,predecessor_task:17,successor:24,successor_task:24}));
 assert_eq!(validate(row(24),&actual,3),Ok(()));
 let changes:[(fn(&mut Observed),Error);20]=[
  (|o|o.target=24|(1u32<<31),Error::Target),
  (|o|o.task=24|(1u32<<31),Error::ShellLifecycleTask),
  (|o|o.task_domain=17,Error::TaskDomain),
  (|o|o.task_domain_pointer_matches=false,Error::TaskDomainPointer),
  (|o|o.content_surface=313|(1u64<<32),Error::ContentSurface),
  (|o|o.logical_app=17,Error::LogicalApp),
  (|o|o.incarnation=2,Error::IncarnationNumber),
  (|o|o.predecessor=12,Error::Predecessor),
  (|o|o.close_target=17,Error::CloseTarget),
  (|o|o.route_target=24|(1u32<<31),Error::RouteTarget),
  (|o|o.shell_lifecycle.target_pd=17,Error::ShellLifecycleTarget),
  (|o|o.shell_lifecycle.task_id=24|(1u32<<31),Error::ShellLifecycleTask),
  (|o|o.self_catalog.task_id=17,Error::SelfCatalogTask),
  (|o|o.self_catalog.holder_pd=17,Error::SelfCatalogHolder),
  (|o|o.activation.as_mut().unwrap().holder=17,Error::ActivationHolder),
  (|o|o.activation.as_mut().unwrap().logical_app=17,Error::ActivationLogicalApp),
  (|o|o.activation.as_mut().unwrap().incarnation=2,Error::ActivationIncarnation),
  (|o|o.activation.as_mut().unwrap().predecessor=12,Error::ActivationPredecessor),
  (|o|o.activation.as_mut().unwrap().predecessor_task=17|(1u32<<31),Error::ActivationPredecessorTask),
  (|o|o.activation.as_mut().unwrap().successor_task=24|(1u32<<31),Error::ActivationSuccessorTask),
 ];
 for(change,error)in changes{let mut mutated=actual;change(&mut mutated);assert_eq!(validate(row(24),&mutated,3),Err(error));}
 let slots:[fn(&mut Incarnation);4]=[
  |row|row.lifecycle_slot=130|(1usize<<32),|row|row.close_slot=131|(1u64<<32),
  |row|row.activation_slot=Some(132|(1usize<<32)),|row|row.route_slot=133|(1u64<<32),
 ];
 for change in slots{let mut rows=INCARNATIONS;change(&mut rows[11]);assert_eq!(validate_layout(&rows),Err(Error::CatalogSlot));}
 let mut rows=INCARNATIONS;rows[11].activation_slot=Some(128);assert_eq!(validate_layout(&rows),Err(Error::CatalogSlotAlias));
 let mut rows=INCARNATIONS;rows[11].ordinal=1;assert_eq!(validate_layout(&rows),Err(Error::CatalogOrder));
}

#[test]
fn all_added_spindle_boot_observations_and_held_tuples_are_exact() {
    for (target,content,incarnation,parent) in [(25,318,4,24),(26,323,5,25),(27,328,6,26),(28,333,7,27),(29,338,8,28)] {
        let actual=observed(target);
        assert_eq!((actual.target,actual.task,actual.content_surface,actual.logical_app,actual.incarnation,actual.predecessor,actual.dormant),
            (target,target,content,12,incarnation,parent,true));
        assert_eq!(actual.shell_lifecycle,LifecycleGrant{target_pd:target,task_id:target,holder_pd:3,rights:QUERY|ACK,ack_bit:1});
        assert_eq!(actual.self_catalog,LifecycleGrant{target_pd:target,task_id:target,holder_pd:target,rights:QUERY,ack_bit:0});
        assert_eq!(actual.activation,Some(ActivationGrant{holder:3,logical_app:12,incarnation,predecessor:parent,predecessor_task:parent,successor:target,successor_task:target}));
        assert_eq!(validate(row(target),&actual,3),Ok(()));
        let changes:[(fn(&mut Observed),Error);12]=[
            (|o|o.target|=1<<31,Error::Target),
            (|o|o.task|=1<<31,Error::ShellLifecycleTask),
            (|o|o.content_surface|=1u64<<32,Error::ContentSurface),
            (|o|o.task_domain=12,Error::TaskDomain),
            (|o|o.task_domain_pointer_matches=false,Error::TaskDomainPointer),
            (|o|o.predecessor=12,Error::Predecessor),
            (|o|o.close_target=12,Error::CloseTarget),
            (|o|o.route_target=12,Error::RouteTarget),
            (|o|o.self_catalog.holder_pd=12,Error::SelfCatalogHolder),
            (|o|o.activation.as_mut().unwrap().holder=12,Error::ActivationHolder),
            (|o|o.activation.as_mut().unwrap().predecessor_task=12,Error::ActivationPredecessorTask),
            (|o|o.activation.as_mut().unwrap().successor_task|=1<<31,Error::ActivationSuccessorTask),
        ];
        for (change,error) in changes {
            let mut bad=actual;change(&mut bad);
            assert_eq!(validate(row(target),&bad,3),Err(error),"{target}");
        }
    }
    let mut rows=INCARNATIONS;rows[16].target=28;
    assert_eq!(validate_layout(&rows),Err(Error::CatalogTarget));
    let mut rows=INCARNATIONS;rows[16].surface=333;
    assert_eq!(validate_layout(&rows),Err(Error::CatalogSurface));
    let mut rows=INCARNATIONS;rows[16].activation_slot=Some(148);
    assert_eq!(validate_layout(&rows),Err(Error::CatalogSlotAlias));
    let mut rows=INCARNATIONS;rows[16].ordinal=6;
    assert_eq!(validate_layout(&rows),Err(Error::CatalogOrder));
    let mut rows=INCARNATIONS;rows[16].route_slot=153|(1u64<<32);
    assert_eq!(validate_layout(&rows),Err(Error::CatalogSlot));
}


// Explicit host observations of independently numbered registered task objects.
// The production BOOT capturer must establish these IDs and pointer facts;
// neither this pure validator nor descriptive catalog data queries a registry.
fn unequal_browser(target:u32)->Observed{
    let mut actual=observed(target);
    let(task,predecessor_task)=match target{15=>(150,0),30=>(300,150),31=>(310,300),_=>panic!("explicit Browser observation")};
    actual.task=task;
    actual.predecessor_registered_task=predecessor_task;
    actual.shell_lifecycle.task_id=task;
    actual.self_catalog.task_id=task;
    if let Some(activation)=actual.activation.as_mut(){activation.successor_task=task;activation.predecessor_task=predecessor_task;}
    actual
}
#[test]
fn unequal_registered_browser_tasks_validate_original_and_both_successors(){
    for(pd,task,predecessor,pred_task)in [(15,150,0,0),(30,300,15,150),(31,310,30,300)]{
        let actual=unequal_browser(pd);let retained=actual;
        assert_eq!((actual.target,actual.task,actual.predecessor,actual.predecessor_registered_task),(pd,task,predecessor,pred_task));
        assert_eq!(actual.task_domain,pd);assert!(actual.task_domain_pointer_matches);
        assert_eq!(validate(row(pd),&actual,3),Ok(()));assert_eq!(actual,retained);
    }
}
#[test]
fn unequal_registered_task_still_requires_exact_pointer_domain_and_grant_tasks(){
    for pd in [15,30,31]{let original=unequal_browser(pd);
        let mut bad=original;bad.task=0;assert_eq!(validate(row(pd),&bad,3),Err(Error::Task));
        let mut bad=original;bad.task_domain=original.task;assert_eq!(validate(row(pd),&bad,3),Err(Error::TaskDomain));
        let mut bad=original;bad.task_domain_pointer_matches=false;assert_eq!(validate(row(pd),&bad,3),Err(Error::TaskDomainPointer));
        for wrong in [0,pd,original.task+1,u32::MAX]{
            let mut bad=original;bad.shell_lifecycle.task_id=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::ShellLifecycleTask));
            let mut bad=original;bad.self_catalog.task_id=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::SelfCatalogTask));
            if let Some(activation)=bad.activation.as_mut(){
                bad.self_catalog=original.self_catalog;activation.successor_task=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::ActivationSuccessorTask));
            }
        }
    }
}
#[test]
fn predecessor_grant_task_matches_separate_registered_task_observation(){
    for pd in [30,31]{let original=unequal_browser(pd);
        for wrong in [0,original.predecessor,original.predecessor_registered_task+1,u32::MAX]{
            let mut bad=original;bad.activation.as_mut().unwrap().predecessor_task=wrong;
            assert_eq!(validate(row(pd),&bad,3),Err(Error::ActivationPredecessorTask));
        }
        let mut bad=original;bad.predecessor_registered_task=0;assert_eq!(validate(row(pd),&bad,3),Err(Error::PredecessorRegisteredTask));
        // Keeping the grant unchanged exposes a stale/replaced registry task;
        // copying a grant task into this observation would defeat that join.
        for wrong in [original.predecessor,original.predecessor_registered_task+1]{
            let mut bad=original;bad.predecessor_registered_task=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::ActivationPredecessorTask));
        }
    }
    for wrong in [15,150,u32::MAX]{let mut bad=unequal_browser(15);bad.predecessor_registered_task=wrong;assert_eq!(validate(row(15),&bad,3),Err(Error::PredecessorRegisteredTask));}
}
#[test]
fn last_browser_held_tasks_do_not_relax_canonical_row_rights_routes_or_terminal_state(){
    let original=unequal_browser(31);
    let controls:[(fn(&mut Observed),Error);8]=[
        (|v|v.dead=true,Error::Dead),(|v|v.dormant=false,Error::Dormant),
        (|v|v.shell_lifecycle.rights|=RETIRE,Error::ShellLifecycleRights),
        (|v|v.self_catalog.holder_pd=310,Error::SelfCatalogHolder),
        (|v|v.close_target=310,Error::CloseTarget),(|v|v.route_target=30,Error::RouteTarget),
        (|v|v.predecessor=15,Error::Predecessor),
        (|v|v.activation.as_mut().unwrap().predecessor=15,Error::ActivationPredecessor),
    ];
    for(change,error)in controls{let mut bad=original;change(&mut bad);assert_eq!(validate(row(31),&bad,3),Err(error));}
    let mut fake=*row(31);fake.route_slot=157;assert_eq!(validate(&fake,&original,3),Err(Error::CatalogRow));
}

#[test]
fn appended_final_quil_and_spindle_require_independent_registered_task_and_context_facts(){
    // Deliberately separate task namespace. These are supplied observations;
    // descriptive source rows do not establish runtime custody or grant them.
    for (pd,task,surface,logical,number,parent,pred_task) in [
        (32,320,345,9,9,23,230),(39,390,352,9,16,38,380),(40,400,353,12,9,29,290),
    ] {
        let mut actual=observed(pd);
        assert_eq!((actual.content_surface,actual.logical_app,actual.incarnation,actual.predecessor),(surface,logical,number,parent));
        actual.task=task;actual.predecessor_registered_task=pred_task;
        actual.shell_lifecycle.task_id=task;actual.self_catalog.task_id=task;
        actual.activation.as_mut().unwrap().successor_task=task;
        actual.activation.as_mut().unwrap().predecessor_task=pred_task;
        assert_eq!(validate(row(pd),&actual,3),Ok(()));
        let retained=actual;
        for wrong in [0,pd,task+1,u32::MAX] {
            let mut bad=actual;bad.shell_lifecycle.task_id=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::ShellLifecycleTask));
            let mut bad=actual;bad.self_catalog.task_id=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::SelfCatalogTask));
            let mut bad=actual;bad.activation.as_mut().unwrap().successor_task=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::ActivationSuccessorTask));
        }
        for wrong in [parent,pred_task+1] {
            let mut bad=actual;bad.predecessor_registered_task=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::ActivationPredecessorTask));
            let mut bad=actual;bad.activation.as_mut().unwrap().predecessor_task=wrong;assert_eq!(validate(row(pd),&bad,3),Err(Error::ActivationPredecessorTask));
        }
        let mut bad=actual;bad.task_domain=task;assert_eq!(validate(row(pd),&bad,3),Err(Error::TaskDomain));
        let mut bad=actual;bad.task_domain_pointer_matches=false;assert_eq!(validate(row(pd),&bad,3),Err(Error::TaskDomainPointer));
        let mut bad=actual;bad.close_target=parent;assert_eq!(validate(row(pd),&bad,3),Err(Error::CloseTarget));
        let mut bad=actual;bad.route_target=parent;assert_eq!(validate(row(pd),&bad,3),Err(Error::RouteTarget));
        let mut bad=actual;bad.dead=true;assert_eq!(validate(row(pd),&bad,3),Err(Error::Dead));
        let mut bad=actual;bad.dormant=false;assert_eq!(validate(row(pd),&bad,3),Err(Error::Dormant));
        let mut bad=actual;bad.shell_lifecycle.rights|=RETIRE;assert_eq!(validate(row(pd),&bad,3),Err(Error::ShellLifecycleRights));
        assert_eq!(actual,retained);
    }
    for index in [26,27] {
        let mut rows=INCARNATIONS;rows[index].route_slot|=1u64<<32;assert_eq!(validate_layout(&rows),Err(Error::CatalogSlot));
        let mut rows=INCARNATIONS;rows[index].activation_slot=Some(164);assert_eq!(validate_layout(&rows),Err(Error::CatalogSlotAlias));
        let mut rows=INCARNATIONS;rows[index].ordinal-=1;assert_eq!(validate_layout(&rows),Err(Error::CatalogOrder));
    }
}
