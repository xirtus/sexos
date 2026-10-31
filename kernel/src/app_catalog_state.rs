// SPDX-License-Identifier: GPL-3.0-only
//! Read-only BOOT audit of catalog data against already-created kernel state.
//! Observations must come from APPS and the actual typed capability tables.
//! No lookup here creates authority, changes liveness, or validates Display's
//! separate frame/child policy. Run before seal and before task admission.
use crate::app_resource_state::{ActivationGrant, LifecycleGrant, ACK, MAX_APPS, QUERY};
use sex_pdx::app_catalog::{App, Incarnation, INCARNATIONS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observed {
    pub target: u32,
    pub task: u32,
    pub task_domain: u32,
    pub task_domain_pointer_matches: bool,
    pub content_surface: u64,
    pub logical_app: u32,
    pub incarnation: u32,
    pub predecessor: u32,
    pub coordinator: u32,
    pub dormant: bool,
    /// Actual terminal/retired/dead state, not the catalog's intended state.
    pub dead: bool,
    pub shell_lifecycle: LifecycleGrant,
    pub self_catalog: LifecycleGrant,
    pub close_target: u32,
    pub route_target: u32,
    pub activation: Option<ActivationGrant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    CatalogCapacity,
    CatalogLength,
    CatalogMissingApp,
    CatalogTarget,
    CatalogSurface,
    CatalogOrder,
    CatalogSlot,
    CatalogSlotAlias,
    CatalogActivationSlot,
    CatalogRow,
    Coordinator,
    Target,
    Task,
    TaskDomain,
    TaskDomainPointer,
    ContentSurface,
    LogicalApp,
    IncarnationNumber,
    Predecessor,
    Dormant,
    Dead,
    ShellLifecycleTarget,
    ShellLifecycleTask,
    ShellLifecycleHolder,
    ShellLifecycleRights,
    ShellLifecycleAck,
    SelfCatalogTarget,
    SelfCatalogTask,
    SelfCatalogHolder,
    SelfCatalogRights,
    SelfCatalogAck,
    CloseTarget,
    RouteTarget,
    MissingActivation,
    UnexpectedActivation,
    ActivationHolder,
    ActivationLogicalApp,
    ActivationIncarnation,
    ActivationPredecessor,
    ActivationPredecessorTask,
    ActivationSuccessor,
    ActivationSuccessorTask,
}

fn slots(row: &Incarnation) -> [u64; 4] {
    [row.lifecycle_slot as u64, row.close_slot, row.route_slot,
     row.activation_slot.map_or(0, |slot| slot as u64)]
}

/// Check only kernel identity/slot layout. Geometry associations are not
/// kernel child grants. Work is bounded by the actual private-app capacity.
pub fn validate_layout(rows: &[Incarnation]) -> Result<(), Error> {
    if rows.len() > MAX_APPS { return Err(Error::CatalogCapacity); }
    if rows.len() != INCARNATIONS.len() { return Err(Error::CatalogLength); }
    for app in [App::Quil, App::Spindle, App::Browser] {
        if !rows.iter().any(|row| row.app == app) { return Err(Error::CatalogMissingApp); }
    }
    for (index, row) in rows.iter().enumerate() {
        if row.target == 0 || row.target == 3
            || rows[..index].iter().any(|old| old.target == row.target) {
            return Err(Error::CatalogTarget);
        }
        if row.surface == 0 || rows[..index].iter().any(|old| old.surface == row.surface) {
            return Err(Error::CatalogSurface);
        }
        if row.ordinal as usize != rows[..index].iter().filter(|old| old.app == row.app).count() {
            return Err(Error::CatalogOrder);
        }
        if (row.ordinal == 0) != row.activation_slot.is_none() {
            return Err(Error::CatalogActivationSlot);
        }
        let current = slots(row);
        if current[..3].iter().any(|slot| *slot == 0 || *slot > u32::MAX as u64)
            || row.activation_slot.is_some() && (current[3] == 0 || current[3] > u32::MAX as u64) {
            return Err(Error::CatalogSlot);
        }
        // Close and view routes may intentionally share the same Domain
        // grant for one subject (Browser43). Lifecycle/activation never may.
        if current[0] == current[1] || current[0] == current[2]
            || current[3] != 0 && current[..3].contains(&current[3]) {
            return Err(Error::CatalogSlotAlias);
        }
        for old in &rows[..index] {
            if current.iter().any(|slot| *slot != 0 && slots(old).contains(slot)) {
                return Err(Error::CatalogSlotAlias);
            }
        }
    }
    Ok(())
}

/// Validate one immutable catalog row against observations of actual BOOT
/// objects. Exact rights are required, including absence of surplus rights.
/// This current catalog explicitly binds every main task ID to its PD ID;
/// generic lifecycle protocol identities remain separate namespaces.
pub fn validate(row: &Incarnation, actual: &Observed, coordinator: u32) -> Result<(), Error> {
    validate_layout(&INCARNATIONS)?;
    if !INCARNATIONS.iter().any(|canonical| canonical == row) { return Err(Error::CatalogRow); }
    if coordinator != 3 || actual.coordinator != coordinator { return Err(Error::Coordinator); }
    if actual.target != row.target { return Err(Error::Target); }
    if actual.task != row.target { return Err(Error::Task); }
    if actual.task_domain != actual.target { return Err(Error::TaskDomain); }
    if !actual.task_domain_pointer_matches { return Err(Error::TaskDomainPointer); }
    if actual.content_surface != row.surface { return Err(Error::ContentSurface); }
    let original = INCARNATIONS.iter().find(|first| first.app == row.app && first.ordinal == 0)
        .ok_or(Error::CatalogOrder)?;
    let predecessor = if row.ordinal == 0 { 0 } else {
        INCARNATIONS.iter().find(|previous| previous.app == row.app && previous.ordinal == row.ordinal - 1)
            .ok_or(Error::CatalogOrder)?.target
    };
    let number = row.ordinal as u32 + 1;
    if actual.logical_app != original.target { return Err(Error::LogicalApp); }
    if actual.incarnation != number { return Err(Error::IncarnationNumber); }
    if actual.predecessor != predecessor { return Err(Error::Predecessor); }
    if actual.dormant != (row.ordinal != 0) { return Err(Error::Dormant); }
    if actual.dead { return Err(Error::Dead); }
    let grant = actual.shell_lifecycle;
    if grant.target_pd != row.target { return Err(Error::ShellLifecycleTarget); }
    if grant.task_id != row.target { return Err(Error::ShellLifecycleTask); }
    if grant.holder_pd != coordinator { return Err(Error::ShellLifecycleHolder); }
    if grant.rights != (QUERY | ACK) { return Err(Error::ShellLifecycleRights); }
    if grant.ack_bit != 1 { return Err(Error::ShellLifecycleAck); }
    let own = actual.self_catalog;
    if own.target_pd != row.target { return Err(Error::SelfCatalogTarget); }
    if own.task_id != row.target { return Err(Error::SelfCatalogTask); }
    if own.holder_pd != row.target { return Err(Error::SelfCatalogHolder); }
    if own.rights != QUERY { return Err(Error::SelfCatalogRights); }
    if own.ack_bit != 0 { return Err(Error::SelfCatalogAck); }
    if actual.close_target != row.target { return Err(Error::CloseTarget); }
    if actual.route_target != row.target { return Err(Error::RouteTarget); }
    if row.ordinal == 0 {
        if actual.activation.is_some() { return Err(Error::UnexpectedActivation); }
    } else {
        let activation = actual.activation.ok_or(Error::MissingActivation)?;
        if activation.holder != coordinator { return Err(Error::ActivationHolder); }
        if activation.logical_app != original.target { return Err(Error::ActivationLogicalApp); }
        if activation.incarnation != number { return Err(Error::ActivationIncarnation); }
        if activation.predecessor != predecessor { return Err(Error::ActivationPredecessor); }
        if activation.predecessor_task != predecessor { return Err(Error::ActivationPredecessorTask); }
        if activation.successor != row.target { return Err(Error::ActivationSuccessor); }
        if activation.successor_task != row.target { return Err(Error::ActivationSuccessorTask); }
    }
    Ok(())
}
