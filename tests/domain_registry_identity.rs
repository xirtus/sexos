//! Native tests of the actual kernel registry, without kernel/x86 dependencies.
use core::sync::atomic::Ordering;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard};

#[macro_export]
macro_rules! serial_println {
    ($($args:tt)*) => { let _ = format_args!($($args)*); };
}

mod capability {
    pub struct ProtectionDomain {
        pub id: u32,
    }
}

mod ipc {
    use core::sync::atomic::AtomicU8;
    pub const MAX_DOMAINS: usize = 1024;
    pub enum BootPhase {
        RegistryBuild = 1,
    }
    pub static BOOT_PHASE: AtomicU8 = AtomicU8::new(0);
}

#[path = "../kernel/src/ipc/domain_registry.rs"]
mod domain_registry;

use capability::ProtectionDomain;
use domain_registry::DomainRegistry;
use ipc::{BootPhase, BOOT_PHASE, MAX_DOMAINS};

// The real BOOT_PHASE is process-global. Serialize fixture phase changes while
// still exercising concurrent publication inside the dedicated race test.
static PHASE_FIXTURE: Mutex<()> = Mutex::new(());

fn fixture() -> MutexGuard<'static, ()> {
    let guard = PHASE_FIXTURE.lock().unwrap();
    ipc::BOOT_PHASE.store(ipc::BootPhase::RegistryBuild as u8, Ordering::SeqCst);
    guard
}

fn domain(id: u32) -> *mut ProtectionDomain {
    Box::into_raw(Box::new(ProtectionDomain { id }))
}

fn rejects(registry: &DomainRegistry, id: u32, pd: *mut ProtectionDomain) {
    assert!(catch_unwind(AssertUnwindSafe(|| registry.insert(id, pd))).is_err());
}

#[test]
fn exact_identity_including_reserved_kernel_zero_and_last_valid_slot() {
    let _phase = fixture();
    let registry = DomainRegistry::new();
    assert_eq!(registry.len(), 0);
    for id in [0, 1, 1023] {
        assert!(registry.get(id).is_none());
        let pd = domain(id);
        registry.insert(id, pd);
        assert!(core::ptr::eq(registry.get(id).unwrap(), pd));
        assert_eq!(registry.get(id).unwrap().id, id);
    }
    assert_eq!(registry.len(), 3);
}

#[test]
fn out_of_range_lookups_cannot_alias_an_existing_domain() {
    let _phase = fixture();
    let registry = DomainRegistry::new();
    for id in [0, 7, 1023] {
        registry.insert(id, domain(id));
    }
    for id in [1024, 7 + 1024, 2047, u32::MAX] {
        assert!(registry.get(id).is_none(), "aliased ID {id}");
    }
    assert_eq!(registry.len(), 3);
}

#[test]
fn out_of_range_insertions_cannot_overwrite_an_aliased_slot() {
    let _phase = fixture();
    let registry = DomainRegistry::new();
    let original = domain(7);
    registry.insert(7, original);
    for id in [1024, 7 + 1024, u32::MAX] {
        rejects(&registry, id, domain(id));
    }
    assert!(core::ptr::eq(registry.get(7).unwrap(), original));
    assert_eq!(registry.len(), 1);
}

#[test]
fn null_and_mismatched_registrations_publish_nothing() {
    let _phase = fixture();
    let registry = DomainRegistry::new();
    rejects(&registry, 7, core::ptr::null_mut());
    rejects(&registry, 7, domain(8));
    rejects(&registry, 0, domain(7));
    assert!(registry.get(7).is_none());
    assert!(registry.get(0).is_none());
    assert_eq!(registry.len(), 0);
}

#[test]
fn duplicate_registration_even_same_pointer_preserves_original() {
    let _phase = fixture();
    let registry = DomainRegistry::new();
    let original = domain(7);
    registry.insert(7, original);
    rejects(&registry, 7, domain(7));
    rejects(&registry, 7, original);
    assert!(core::ptr::eq(registry.get(7).unwrap(), original));
    assert_eq!(registry.len(), 1);
}

#[test]
fn only_registry_build_phase_accepts_publication() {
    let _phase = fixture();
    let registry = DomainRegistry::new();
    let original = domain(7);
    registry.insert(7, original);
    for phase in [0, 2, 3, 4, 5, u8::MAX] {
        ipc::BOOT_PHASE.store(phase, Ordering::SeqCst);
        rejects(&registry, 8, domain(8));
        assert!(registry.get(8).is_none());
        assert!(core::ptr::eq(registry.get(7).unwrap(), original));
        assert_eq!(registry.len(), 1);
    }
}

#[test]
fn lookup_checks_stored_identity_even_if_public_array_is_misused() {
    let _phase = fixture();
    let registry = DomainRegistry::new();
    registry.domains[7].store(domain(8), Ordering::Release);
    assert!(registry.get(7).is_none());
}

#[test]
fn concurrent_duplicate_publications_have_exactly_one_winner() {
    let _phase = fixture();
    let registry = Arc::new(DomainRegistry::new());
    let candidates = [domain(7) as usize, domain(7) as usize];
    let threads = candidates.map(|candidate| {
        let registry = registry.clone();
        std::thread::spawn(move || {
            catch_unwind(AssertUnwindSafe(|| {
                registry.insert(7, candidate as *mut ProtectionDomain)
            }))
            .is_ok()
        })
    });
    let wins = threads.map(|thread| thread.join().unwrap());
    assert_eq!(wins.into_iter().filter(|won| *won).count(), 1);
    let actual = registry.get(7).unwrap() as *const _ as usize;
    assert!(candidates.contains(&actual));
    assert_eq!(registry.len(), 1);
}
