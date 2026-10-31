// SPDX-License-Identifier: GPL-3.0-only
//! Actual pure BOOT validator over explicit six-subject kernel observations.
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
        _ => panic!("not a current BOOT subject"),
    };
    Observed {
        target, task: target, task_domain: target, task_domain_pointer_matches: true,
        content_surface, logical_app, incarnation,
        predecessor, coordinator: 3, dormant, dead: false,
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
fn all_six_existing_boot_subjects_validate_without_changing_observations() {
    assert_eq!(INCARNATIONS.map(|row| row.target), [9, 12, 15, 16, 17, 18]);
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
        (|o| o.task = 12, Error::Task),
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
    for target in [16, 17, 18] {
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
    let mutations: [(fn(&mut [Incarnation; 6]), Error); 11] = [
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
    let mut rows = INCARNATIONS;rows[2].app = App::Quil;
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
    for target in [9, 16, 18] {
        let mut actual = observed(target);actual.task_domain = 12;
        assert_eq!(validate(row(target), &actual, 3), Err(Error::TaskDomain));
        let mut actual = observed(target);actual.task_domain_pointer_matches = false;
        assert_eq!(validate(row(target), &actual, 3), Err(Error::TaskDomainPointer));
    }
}
