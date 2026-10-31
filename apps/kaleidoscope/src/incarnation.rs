// SPDX-License-Identifier: GPL-3.0-only
//! Browser identity metadata selected after a successful held SELF_CATALOG
//! query. The descriptive catalog supplies associations, never authority;
//! this helper neither queries capabilities nor publishes mutable state.
use sex_pdx::app_catalog::{self, App};
use sex_pdx::app_lifecycle::CatalogInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    target: u32,
    task: u32,
    surface: u64,
    ordinal: u8,
}

impl Identity {
    pub const fn target(self) -> u32 { self.target }
    pub const fn task(self) -> u32 { self.task }
    pub const fn surface(self) -> u64 { self.surface }
    pub const fn ordinal(self) -> u8 { self.ordinal }
    pub const fn is_original(self) -> bool { self.ordinal == 0 }
}

/// The caller must pass the result of its successful held catalog query.
/// In particular, a task ID is retained exactly; it is not inferred from a PD.
pub fn select(c: CatalogInfo) -> Option<Identity> {
    let row = app_catalog::by_surface(c.target_pd, c.surface_id)?;
    if row.app != App::Browser || !c.is_subject(row.target, row.surface, 3) {
        return None;
    }
    Some(Identity {
        target: row.target,
        task: c.task_id,
        surface: row.surface,
        ordinal: row.ordinal,
    })
}
