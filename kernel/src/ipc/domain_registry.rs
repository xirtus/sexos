//! Boot-built protection-domain registry. Identities are exact array indices,
//! never modulo aliases. Entries are published once and remain stable.
use crate::capability::ProtectionDomain;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use super::{BootPhase, BOOT_PHASE, MAX_DOMAINS};

pub struct DomainRegistry {
    pub domains: [AtomicPtr<ProtectionDomain>; MAX_DOMAINS],
}

impl DomainRegistry {
    pub const fn new() -> Self {
        const INIT: AtomicPtr<ProtectionDomain> = AtomicPtr::new(ptr::null_mut());
        Self {
            domains: [INIT; MAX_DOMAINS],
        }
    }

    pub fn get(&self, id: u32) -> Option<&'static ProtectionDomain> {
        let slot = self.domains.get(id as usize)?;
        let ptr = slot.load(Ordering::Acquire);
        // Registry insertions accept only kernel-owned, permanently live PDs.
        let pd = unsafe { ptr.as_ref() }?;
        if pd.id != id {
            return None;
        }
        Some(pd)
    }

    /// Boot misuse is fatal, as with the existing phase invariant. Invalid or
    /// duplicate registration never removes or replaces an existing domain.
    /// `pd` must point to a fully initialized, permanently live kernel PD.
    pub fn insert(&self, id: u32, pd: *mut ProtectionDomain) {
        let phase = BOOT_PHASE.load(Ordering::SeqCst);
        if phase != BootPhase::RegistryBuild as u8 {
            crate::serial_println!(
                "registry.mutation.denied registry=DOMAIN_REGISTRY op=insert domain_id={} phase={}",
                id,
                phase
            );
        }
        assert!(
            phase == BootPhase::RegistryBuild as u8,
            "REGISTRY_MUTATION_VIOLATION"
        );
        assert!((id as usize) < MAX_DOMAINS, "REGISTRY_ID_OUT_OF_RANGE");
        assert!(!pd.is_null(), "REGISTRY_NULL_DOMAIN");
        assert!(unsafe { (*pd).id } == id, "REGISTRY_DOMAIN_ID_MISMATCH");
        assert!(
            self.domains[id as usize]
                .compare_exchange(ptr::null_mut(), pd, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "REGISTRY_DUPLICATE_DOMAIN"
        );
    }

    pub fn len(&self) -> usize {
        self.domains
            .iter()
            .filter(|p| !p.load(Ordering::Acquire).is_null())
            .count()
    }
}
