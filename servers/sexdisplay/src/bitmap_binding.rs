//! Immutable Display-local bitmap metadata, not a capability or grant.
//!
//! Display startup must obtain `(slot, catalog)` through its own held lifecycle
//! query before constructing this value. Catalog rows alone confer no authority.
//! No packet may construct, replace, or rebind an installed bitmap store.
use sex_pdx::{app_catalog::{self, App}, app_lifecycle::CatalogInfo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    slot: usize,
    target: u32,
    task: u32,
    surface: u64,
    coordinator: u32,
}

impl Binding {
    /// Validate the already-held query's complete tuple against descriptive
    /// BOOT catalog metadata. There is deliberately no default identity.
    pub fn from_catalog(slot: usize, catalog: CatalogInfo) -> Option<Self> {
        let row = app_catalog::by_surface(catalog.target_pd, catalog.surface_id)?;
        if row.app != App::Browser || row.lifecycle_slot != slot
            || !catalog.is_subject(row.target, row.surface, 3)
            || catalog.surface_id == 0 || catalog.surface_id > u16::MAX as u64
        {
            return None;
        }
        Some(Self {
            slot,
            target: catalog.target_pd,
            task: catalog.task_id,
            surface: catalog.surface_id,
            coordinator: catalog.coordinator_pd,
        })
    }

    pub const fn slot(self) -> usize { self.slot }
    pub const fn target(self) -> u32 { self.target }
    pub const fn task(self) -> u32 { self.task }
    pub const fn surface(self) -> u64 { self.surface }
    pub const fn coordinator(self) -> u32 { self.coordinator }

    /// Only an actual kernel-stamped sender tuple can select this store.
    pub const fn admits(self, caller_pd: u32, caller_task: u32, sid: u64) -> bool {
        caller_pd == self.target && caller_task == self.task && sid == self.surface
    }

    /// The caller separately obtains exact terminal lifecycle proof. This check
    /// does not turn a notification or an app-supplied tuple into death proof.
    pub const fn matches_retirement(self, target: u32, task: u32) -> bool {
        target == self.target && task == self.task
    }
}
