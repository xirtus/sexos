use x86_64::VirtAddr;
use crate::serial_println;
use crate::ipc::DOMAIN_REGISTRY;
use x86_64::structures::paging::PageTableFlags;

pub struct ElfLoader;

impl ElfLoader {
    fn pd_code_base(domain_id: u8) -> u64 {
        0x4000_0000 + ((domain_id as u64).saturating_sub(1) * 0x0100_0000)
    }

    fn pd_stack_top(domain_id: u8) -> u64 {
        0x7000_0000_0000 + ((domain_id as u64).saturating_sub(1) * 0x0100_0000) + 0x0010_0000
    }

    /// Loads a 64-bit ELF via PDX to sexfiles and maps it into the Global SAS.
    pub fn load_elf(path: &str, pd_id: u32, domain_id: u8, pku_key: u8) -> Result<(VirtAddr, u64), &'static str> {
        let start_ticks = crate::hal::get_monotonic_counter();
        let mut checkpoint = "loader.start";
        serial_println!("loader: Loading ELF {} (PKU Key {})...", path, pku_key);
        let code_base = Self::pd_code_base(domain_id);
        let stack_top = Self::pd_stack_top(domain_id);
        serial_println!(
            "sasos.layout pd_id={} name={} base={:#x} entry=pending stack={:#x}",
            pd_id,
            path,
            code_base,
            stack_top
        );

        let current_pd_ptr = crate::core_local::CoreLocal::get().current_pd_ptr.load(core::sync::atomic::Ordering::Acquire);

        // 1. Resolve path to sexfiles (Via capability slot 1 - granted at boot)
        // Bootstrap: If kernel (PD 0) or no PD, bypass VFS check
        if !current_pd_ptr.is_null() {
            let current_pd = unsafe { &*current_pd_ptr };
            if current_pd.id != 0 {
                let vfs_cap = unsafe { (&*current_pd.cap_table).find(1).ok_or("loader: VFS cap missing")? };
                let sexfiles_pd_id = match vfs_cap.data {
                    crate::capability::CapabilityData::IPC(data) => data.target_pd_id,
                    _ => return Err("loader: Invalid VFS cap"),
                };
                let _sexfiles_pd = DOMAIN_REGISTRY.get(sexfiles_pd_id).ok_or("loader: sexfiles not found")?;
            } else {
                serial_println!("loader: Bootstrap mode (PD 0). Using internal initrd.");
            }
        } else {
            serial_println!("loader: Bootstrap mode (no active PD). Using internal initrd.");
        }

        // 2. Resolve module bytes
        checkpoint = "loader.before_module_lookup";
        let module = crate::MODULE_REQUEST
            .response()
            .and_then(|mods| mods.modules().iter().find(|m| m.path() == path))
            .ok_or("loader: module not found in limine list")?;
        let elf_data = module.data();
        serial_println!(
            "loader: module resolved path={} bytes={}",
            path,
            elf_data.len()
        );

        // 3. Map PT_LOAD segments into GLOBAL_VAS with real flags
        checkpoint = "loader.before_elf_header";
        serial_println!("loader: checkpoint={}", checkpoint);
        let entry = {
            let mut vas_lock = crate::memory::manager::GLOBAL_VAS.lock();
            let vas = vas_lock.as_mut().ok_or("loader: VAS not initialized")?;
            let entry =
                crate::elf::load_elf_for_pd(elf_data, vas, pku_key, VirtAddr::new(code_base))?;
            checkpoint = "loader.after_memory_mapping";
            serial_println!("loader: checkpoint={}", checkpoint);
            entry
        };

        serial_println!("loader: ELF header parsed. Entry at {:#x}", entry.as_u64());
        checkpoint = "loader.after_elf_header";
        serial_println!("loader: checkpoint={}", checkpoint);

        // 4. Create Stack with Guard Page (mapped USER|RW|NX)
        {
            let mut vas_lock = crate::memory::manager::GLOBAL_VAS.lock();
            let vas = vas_lock.as_mut().ok_or("loader: VAS not initialized")?;
            vas.map_pku_range(
                VirtAddr::new(stack_top - (64 * 1024)),
                64 * 1024,
                PageTableFlags::PRESENT
                    | PageTableFlags::USER_ACCESSIBLE
                    | PageTableFlags::WRITABLE
                    | PageTableFlags::NO_EXECUTE,
                pku_key,
            )?;
        }
        serial_println!("loader: Guard-page stack initialized at {:#x}", stack_top);

        crate::app_resources::capture_boot_image(pd_id,pku_key,elf_data,code_base,stack_top)?;

        // 5. Verify entrypoint mapping contract before task creation
        Self::assert_user_rip_mapped_executable(0, entry)?;

        serial_println!(
            "loader: done path={} elapsed_ticks={} last_checkpoint={}",
            path,
            crate::hal::get_monotonic_counter().saturating_sub(start_ticks),
            checkpoint
        );

        Ok((entry, stack_top))
    }

    pub fn assert_user_rip_mapped_executable(pd_id: u32, entry: VirtAddr) -> Result<(), &'static str> {
        let pte = crate::memory::manager::read_pte_flags(entry)?;
        let present = (pte & 1) != 0;
        let writable = (pte & (1 << 1)) != 0;
        let user = (pte & (1 << 2)) != 0;
        let nx = (pte & (1u64 << 63)) != 0;
        serial_println!(
            "loader: entry.assert rip={:#x} pte={:#x} present={} user={} writable={} nx={}",
            entry.as_u64(),
            pte,
            present,
            user,
            writable,
            nx
        );
        serial_println!("loader: entry.assert.pd_id={}", pd_id);
        if !present || !user || nx {
            return Err("loader: entrypoint mapping not user-executable");
        }
        Ok(())
    }
}
