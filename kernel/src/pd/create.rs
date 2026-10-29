use crate::capability::ProtectionDomain;
use crate::ipc::DOMAIN_REGISTRY;
use crate::loader::elf::ElfLoader;
use crate::serial_println;
use crate::capabilities::engine::CapEngine;

use core::sync::atomic::{AtomicU64, Ordering};
static NEXT_PD_ID: AtomicU64 = AtomicU64::new(1);
const PD_CREATE_WATCHDOG_TICKS: u64 = 5_000_000; // temporary debug watchdog

#[inline(always)]
fn pd_watchdog(start: u64, last_checkpoint: &str) {
    let now = crate::hal::get_monotonic_counter();
    let dt = now.saturating_sub(start);
    if dt > PD_CREATE_WATCHDOG_TICKS {
        serial_println!(
            "pd: WATCHDOG timeout ticks={} last_checkpoint={}",
            dt,
            last_checkpoint
        );
    }
}

/// create_protection_domain: Ruthless Phase 6/8/10 implementation.
pub fn create_protection_domain(elf_path: &str, requested_id: Option<u32>, domain_id: u8) -> Result<u32, &'static str> {
    create_boot_domain(elf_path,requested_id,domain_id,domain_id)
}

/// BOOT layout identity and finite hardware key are separate inputs.
pub fn create_boot_domain(elf_path:&str,requested_id:Option<u32>,layout:u8,domain_id:u8)->Result<u32,&'static str>{
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),crate::ipc::BootPhase::RegistryBuild as u8,"PD_GENESIS_AFTER_BOOT");
    assert!(domain_id>0 && domain_id<16 && layout>0,"PD_BOOT_LAYOUT_OR_KEY");
    let start_ticks = crate::hal::get_monotonic_counter();
    let mut checkpoint = "create.start";
    serial_println!("pd: Creating domain for {} (Domain ID {})...", elf_path, domain_id);
    serial_println!("pd: checkpoint=post.create.log.1");
    serial_println!("pd: checkpoint=post.create.log.2");

    // 1. Allocate a unique PD ID
    checkpoint = "pd_id.allocate";
    serial_println!("pd: checkpoint={} before rdseed/id", checkpoint);
    let pd_id = if let Some(id) = requested_id {
        id
    } else {
        NEXT_PD_ID.fetch_add(1, core::sync::atomic::Ordering::SeqCst) as u32
    };
    serial_println!("pd: checkpoint=pd_id.allocate.exit");
    serial_println!("pd: checkpoint={} after rdseed/id pd_id={}", checkpoint, pd_id);
    
    // 2. Precompute PKU domain mask (Hardware isolation)
    checkpoint = "pkru.precompute";
    serial_println!("pd: checkpoint=pkru.precompute.enter");
    let pkru_mask = crate::capability::PkruValue::for_domain(domain_id);
    serial_println!("pd: checkpoint=pkru.precompute.exit");

    // Raw pointer for ProtectionDomain
    checkpoint = "pd.alloc_struct";
    serial_println!("pd: checkpoint=pd.alloc_struct.enter");
    let new_pd = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(ProtectionDomain::new(pd_id, domain_id)));
    serial_println!("pd: checkpoint=pd.alloc_struct.post_alloc");
    unsafe { (*new_pd).current_pkru_mask.store(pkru_mask, core::sync::atomic::Ordering::Release); }
    serial_println!("pd: checkpoint=pd.alloc_struct.post_store");

    // 3. Load ELF via PDX to sexfiles
    checkpoint = "elf.before_load";
    serial_println!("pd: checkpoint={} path={}", checkpoint, elf_path);
    let (entry, stack_top) = ElfLoader::load_elf(elf_path, pd_id, layout, domain_id)?;
    checkpoint = "elf.after_load";
    serial_println!("pd: checkpoint={} entry={:#x}", checkpoint, entry.as_u64());
    ElfLoader::assert_user_rip_mapped_executable(pd_id, entry)?;
    pd_watchdog(start_ticks, checkpoint);

    // 4. Register with Registry (Lock-free insertion) - Metadata ONLY
    checkpoint = "registry.insert";
    DOMAIN_REGISTRY.insert(pd_id, new_pd);

    let pd_ref = unsafe { &*new_pd };

    // 5. Mint initial root capabilities
    checkpoint = "caps.grant_initial";
    CapEngine::grant_initial_rights(pd_ref);
    pd_watchdog(start_ticks, checkpoint);

    // 6. Create main execution Task (Raw Pointer)
    checkpoint = "entrypoint.before_set";
    serial_println!("pd: checkpoint={} pd_id={} entry={:#x}", checkpoint, pd_id, entry.as_u64());
    let mut task = crate::scheduler::Task::new(
        pd_id, entry.as_u64(), stack_top, pd_ref, true
    );
    // Explicitly set precomputed PKRU mask in context
    task.context.pkru = pkru_mask as u64;

    let task_ptr = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(task));
    
    // Phase 13.2.1: Store main task for interrupt unparking
    checkpoint = "scheduler.before_enqueue";
    serial_println!("pd: checkpoint={} pd_id={}", checkpoint, pd_id);
    unsafe { (*new_pd).main_task.store(task_ptr, core::sync::atomic::Ordering::Release); }
    serial_println!("pd: main_task registered pd_id={} (not enqueued yet)", pd_id);
    pd_watchdog(start_ticks, checkpoint);

    // 7. Create Dedicated Signal Trampoline Task
    checkpoint = "trampoline.create";
    let trampoline_stack_top = 0x_7000_1000_0000u64;
 
    let mut trampoline_task = crate::scheduler::Task::new(
        pd_id | 0x8000_0000, 0, trampoline_stack_top, pd_ref, true
    );
    trampoline_task.context.pkru = pkru_mask as u64;

    let trampoline_task_ptr = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(trampoline_task));
    unsafe { (*new_pd).trampoline_task.store(trampoline_task_ptr, core::sync::atomic::Ordering::Release); }

    // Bootstrap execution identity without placing into runqueue

    checkpoint = "create.done";
    serial_println!(
        "pd: PD {} Spawned (Bootstrap CoreLocal identity initialized). elapsed_ticks={}",
        pd_id,
        crate::hal::get_monotonic_counter().saturating_sub(start_ticks)
    );
    Ok(pd_id)
}

/// spawn_from_elf: Ruthless Phase 24 implementation.
pub fn spawn_from_elf(_elf_addr: u64, _elf_len: u64) -> Result<u32, &'static str> {
    serial_println!("pd: spawn_from_elf called (Stubbed for Phase 24)");
    // For now, redirect to silk-shell to maintain boot chain (Domain 4)
    create_protection_domain("/servers/silk-shell", None, 4)
}
