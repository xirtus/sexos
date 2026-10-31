// SPDX-License-Identifier: GPL-3.0-only
//! BOOT_DAG bootstrap reservation and immutable region genesis.
//! Runtime resolution never calls these allocator/mapping mechanisms. Sext's
//! bounded paging pool and page-table infrastructure are constructed here too.

use crate::capability::{CapabilityData, ProtectionDomain};
use crate::ipc::{BootPhase, DOMAIN_REGISTRY};
use sex_pdx::memory_region::{RegionGrant, QUERY_VIEW, READ, WRITE, DMA_ADDRESS};
use x86_64::{VirtAddr, structures::paging::PageTableFlags};

fn require_boot() {
    assert_eq!(crate::ipc::BOOT_PHASE.load(core::sync::atomic::Ordering::Acquire),
        BootPhase::RegistryBuild as u8, "BOOT_MEMORY_AFTER_GENESIS");
}

pub(crate) fn backing(length: u64) -> u64 {
    require_boot();
    assert!(length > 0 && length % 4096 == 0);
    let pages = (length / 4096).next_power_of_two();
    let physical = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(pages.trailing_zeros() as usize)
        .expect("BOOT_REGION_NO_FRAMES");
    crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("BOOT_REGION_NO_VAS")
        .frame_allocator.reserve_boot_region(physical, pages * 4096)
        .expect("BOOT_REGION_FRAME_OWNERSHIP_COLLISION");
    let hhdm = crate::HHDM_REQUEST.response().expect("BOOT_REGION_NO_HHDM").offset;
    // Zero the entire reservation, including buddy rounding, before publication.
    unsafe { core::ptr::write_bytes((physical + hhdm) as *mut u8, 0, (pages * 4096) as usize); }
    physical
}

fn alias(pd: &ProtectionDomain, physical: u64, length: u64, writable: bool, mmio: bool) -> u64 {
    require_boot();
    let va = crate::memory::va_allocator::allocate_va(length as usize).expect("BOOT_REGION_NO_VA");
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE;
    if writable { flags |= PageTableFlags::WRITABLE; }
    if mmio { flags |= PageTableFlags::NO_CACHE | PageTableFlags::WRITE_THROUGH; }
    crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("BOOT_REGION_NO_VAS")
        .map_physical_range(VirtAddr::new(va), physical, length, flags, pd.pku_key)
        .expect("BOOT_REGION_MAP_FAILED");
    va
}

fn grant(pd: &ProtectionDomain, slot: u64, region: RegionGrant) {
    assert!(pd.find_capability(slot as u32).is_none(), "BOOT_REGION_SLOT_OCCUPIED");
    sex_pdx::memory_region::resolve_info(Some(&region)).expect("BOOT_REGION_INVALID_GRANT");
    pd.grant_capability(slot, CapabilityData::Region(region));
    crate::serial_println!("[boot.memory.region] pd={} slot={} region={} bytes={} rights={} peer={}",
        pd.id, slot, region.region_id, region.length, region.rights, region.peer_pd.unwrap_or(0));
}

fn dma(pd_id: u32, region_id: u32, length: u64) {
    if pd_id == 0 { return; }
    let pd = DOMAIN_REGISTRY.get(pd_id).expect("BOOT_DMA_PD_MISSING");
    let physical = backing(length);
    let virtual_base = alias(pd, physical, length, true, false);
    grant(pd, sex_pdx::SLOT_DMA_REGION, RegionGrant { region_id, virtual_base, length,
        rights: QUERY_VIEW | READ | WRITE | DMA_ADDRESS, dma_base: Some(physical), peer_pd: None });
}

fn paired(first_id: u32, second_id: u32, slot: u64, region_id: u32, length: u64, second_writable: bool) {
    paired_at(first_id, second_id, slot, slot, region_id, length, second_writable);
}

fn paired_at(first_id: u32, second_id: u32, first_slot: u64, second_slot: u64,
    region_id: u32, length: u64, second_writable: bool)
{
    require_boot();
    if first_id == 0 || second_id == 0 { return; }
    let first = DOMAIN_REGISTRY.get(first_id).expect("BOOT_SHARED_FIRST_MISSING");
    let second = DOMAIN_REGISTRY.get(second_id).expect("BOOT_SHARED_SECOND_MISSING");
    let physical = backing(length);
    let first_va = alias(first, physical, length, true, false);
    let second_va = alias(second, physical, length, second_writable, false);
    grant(first, first_slot, RegionGrant { region_id, virtual_base: first_va, length,
        rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: Some(second.id) });
    grant(second, second_slot, RegionGrant { region_id, virtual_base: second_va, length,
        rights: QUERY_VIEW | READ | if second_writable { WRITE } else { 0 },
        dma_base: None, peer_pd: Some(first.id) });
}

/// The first UiIR stream: the producer may write its ring and only read the
/// consumer's cursor; the consumer may only read the ring and write its
/// cursor. Both are zeroed, exact-peer BOOT regions; nothing is minted later.
pub fn init_uiir(producer: u32, consumer: u32) {
    use sex_pdx::uiir::{RING_SLOT, CURSOR_SLOT, RING_REGION_ID, CURSOR_REGION_ID, RING_BYTES, CURSOR_BYTES};
    require_boot();
    if producer == 0 || consumer == 0 { return; }
    paired_at(producer, consumer, RING_SLOT, RING_SLOT, RING_REGION_ID, RING_BYTES as u64, false);
    paired_at(consumer, producer, CURSOR_SLOT, CURSOR_SLOT, CURSOR_REGION_ID, CURSOR_BYTES as u64, false);
    crate::serial_println!("[boot.uiir.stream] producer={} consumer={} ring_region={} cursor_region={} runtime_mint=0", producer, consumer, RING_REGION_ID, CURSOR_REGION_ID);
}

/// Permanent BOOT budget, separate from reclaimable app-private pages. Each
/// successor gets different backing and an immutable, exact provider peer.
/// Runtime Load still uses scalar transport until snapshot publication exists.
pub fn init_documents(files: u32, clients: [u32; 3]) {
    use sex_pdx::document_buffer::{BYTES, CLIENT_SLOT, FIRST_REGION_ID, PROVIDER_SLOTS};
    require_boot();
    if files == 0 { return; }
    // Validate all subjects/slots before the first reservation or grant.
    let provider = DOMAIN_REGISTRY.get(files).expect("BOOT_DOCUMENT_PROVIDER_MISSING");
    for (index, client_id) in clients.into_iter().enumerate() {
        if client_id == 0 { continue; }
        assert!(client_id != files && !clients[..index].contains(&client_id), "BOOT_DOCUMENT_PEER_ALIAS");
        let client = DOMAIN_REGISTRY.get(client_id).expect("BOOT_DOCUMENT_CLIENT_MISSING");
        assert!(provider.find_capability(PROVIDER_SLOTS[index] as u32).is_none()
            && client.find_capability(CLIENT_SLOT as u32).is_none(), "BOOT_DOCUMENT_SLOT_OCCUPIED");
    }
    for (index, client_id) in clients.into_iter().enumerate() {
        paired_at(files, client_id, PROVIDER_SLOTS[index], CLIENT_SLOT,
            FIRST_REGION_ID + index as u32, BYTES, false);
    }
    document_selftest(files, clients);
}

fn document_selftest(files: u32, clients: [u32; 3]) {
    use sex_pdx::document_buffer::{BYTES, CLIENT_SLOT, FIRST_REGION_ID, PROVIDER_SLOTS};
    let provider = DOMAIN_REGISTRY.get(files).expect("DOCUMENT_TEST_PROVIDER_MISSING");
    let fail = [u64::MAX, 0, 0, 0, 0];
    let mut views = [(0u64, 0u64); 3];
    let mut pairs = 0;
    for (index, client_id) in clients.into_iter().enumerate() {
        if client_id == 0 { continue; }
        let client = DOMAIN_REGISTRY.get(client_id).expect("DOCUMENT_TEST_CLIENT_MISSING");
        let slot = PROVIDER_SLOTS[index];
        let id = (FIRST_REGION_ID + index as u32) as u64;
        assert_eq!(crate::syscalls::region_query(provider, 55, slot, 0, 0, 0),
            [0, BYTES, (QUERY_VIEW | READ | WRITE) as u64, client_id as u64, id]);
        assert_eq!(crate::syscalls::region_query(client, 55, CLIENT_SLOT, 0, 0, 0),
            [0, BYTES, (QUERY_VIEW | READ) as u64, files as u64, id]);
        let writer = crate::syscalls::region_query(provider, 54, slot, 0, BYTES, (QUERY_VIEW | READ | WRITE) as u64);
        let reader = crate::syscalls::region_query(client, 54, CLIENT_SLOT, 0, BYTES, (QUERY_VIEW | READ) as u64);
        assert_eq!([writer[0], writer[2], writer[3], writer[4]], [0, BYTES, 0, id]);
        assert_eq!([reader[0], reader[2], reader[3], reader[4]], [0, BYTES, 0, id]);
        assert_ne!(writer[1], reader[1], "DOCUMENT_TEST_DOMAIN_ALIAS");
        for rights in [QUERY_VIEW | WRITE, QUERY_VIEW | DMA_ADDRESS] {
            assert_eq!(crate::syscalls::region_query(client, 54, CLIENT_SLOT, 0, BYTES, rights as u64), fail);
        }
        assert_eq!(crate::syscalls::region_query(provider, 54, slot, 0, BYTES, (QUERY_VIEW | DMA_ADDRESS) as u64), fail);
        for wrong_slot in [65, 66] {
            assert_eq!(crate::syscalls::region_query(client, 55, wrong_slot, 0, 0, 0), fail);
        }
        assert_eq!(crate::syscalls::region_query(client, 54, CLIENT_SLOT, BYTES - 1, 2, (QUERY_VIEW | READ) as u64), fail);
        // Check the actual GLOBAL_VAS leaf permissions as well as authority
        // replies. No second address space or client-side writable alias exists.
        for page in (0..BYTES).step_by(4096) {
            let leaf = existing_leaf_address(reader[1] + page).expect("DOCUMENT_TEST_LEAF_MISSING");
            let entry = unsafe { core::ptr::read_volatile(leaf as *const u64) };
            assert_eq!(entry & 7, 5, "DOCUMENT_TEST_CLIENT_LEAF_WRITABLE");
            assert_ne!(entry & (1u64 << 63), 0, "DOCUMENT_TEST_CLIENT_LEAF_EXECUTABLE");
            assert_eq!((entry >> 59) & 15, client.pku_key as u64, "DOCUMENT_TEST_CLIENT_LEAF_KEY");
        }
        views[index] = (writer[1], reader[1]);
        // Different full-buffer patterns survive subsequent peer publication.
        unsafe { core::ptr::write_bytes(writer[1] as *mut u8, index as u8 + 1, BYTES as usize); }
        pairs += 1;
    }
    for (index, &(writer, reader)) in views.iter().enumerate() {
        if writer == 0 { continue; }
        for offset in 0..BYTES {
            assert_eq!(unsafe { core::ptr::read_volatile((reader + offset) as *const u8) }, index as u8 + 1,
                "DOCUMENT_TEST_CROSS_INCARNATION_ALIAS");
        }
        unsafe { core::ptr::write_bytes(writer as *mut u8, 0, BYTES as usize); }
    }
    for &(writer, reader) in &views {
        if writer == 0 { continue; }
        for offset in 0..BYTES {
            assert_eq!(unsafe { core::ptr::read_volatile((reader + offset) as *const u8) }, 0, "DOCUMENT_TEST_SCRUB");
        }
    }
    crate::serial_println!("[boot.document.views] pairs={} bytes_each={} total_bytes={} readonly=1 distinct=1 scrubbed=1 runtime_mint=0", pairs, BYTES, pairs * BYTES);
}

pub fn init(drive: u32, display: u32, usb: u32, net: u32, files: u32) {
    require_boot();
    // Fixed BOOT_DAG budget, not runtime allocation policy. Drivers may reserve
    // byte subregions within these grants; they cannot create frame authority.
    dma(usb, 1, 256 * 1024);
    dma(net, sex_pdx::pci_command::DMA_REGION_ID, sex_pdx::pci_command::DMA_BYTES);
    dma(drive, 3, 512 * 1024);
    paired(files, drive, sex_pdx::SLOT_BLOCK_BUFFER, 4, 4096, true);
    paired(drive, display, sex_pdx::SLOT_FRAME_REGION, 5, 3 * 1024 * 1024, false);
    if drive != 0 && files != 0 {
        negative_selftest(DOMAIN_REGISTRY.get(drive).unwrap(), DOMAIN_REGISTRY.get(files).unwrap());
    }
    if net != 0 {
        let pd=DOMAIN_REGISTRY.get(net).expect("BOOT_NIC_PD_MISSING");
        let fail=[sex_pdx::pci_command::DENIED,0,0,0,0];let mut denied=0;
        for (slot,set,clear,a,b) in [(0,0,0,0,0),((1u64<<32)|sex_pdx::SLOT_NIC,0,0,0,0),
            (sex_pdx::SLOT_NIC,1,0,0,0),(sex_pdx::SLOT_NIC,1u64<<32,0,0,0),
            (sex_pdx::SLOT_NIC,2,2,0,0),(sex_pdx::SLOT_NIC,0,0,1,0),(sex_pdx::SLOT_NIC,0,0,0,1)] {
            assert_eq!(crate::pci_command::invoke(pd,slot,set,clear,a,b),fail);denied+=1;
        }
        if files != 0 {
            let other=DOMAIN_REGISTRY.get(files).unwrap();
            for slot in [sex_pdx::SLOT_NIC,sex_pdx::SLOT_BLOCK] {
                assert_eq!(crate::pci_command::invoke(other,slot,0,0,0,0),fail);denied+=1;
            }
        }
        crate::serial_println!("[pci.command.negative.selftest] denied={} output_zero=1 runtime_mint=0 ok=1",denied);
    }
}

fn negative_selftest(pd: &ProtectionDomain, route_holder: &ProtectionDomain) {
    let slot = sex_pdx::SLOT_DMA_REGION;
    let before = crate::syscalls::region_query(pd, 55, slot, 0, 0, 0);
    assert_eq!(before[0], 0, "REGION_TEST_MISSING_DMA");
    let fail = [u64::MAX, 0, 0, 0, 0];
    let mut checked = 0;
    for number in [54, 55] {
        for bad_slot in [0, 1025, (1u64 << 32) | slot, u64::MAX] {
            assert_eq!(crate::syscalls::region_query(pd, number, bad_slot, 0, 4096, QUERY_VIEW as u64), fail);
            checked += 1;
        }
    }
    // Mandatory wrong-kind checks use sexfiles' real block route. Drive has no
    // shell route, and its PCI caps are valid views, so neither tests wrong kind.
    assert!(matches!(route_holder.find_capability(sex_pdx::SLOT_BLOCK as u32).map(|cap| cap.data),
        Some(CapabilityData::Domain(target)) if target == pd.id), "REGION_TEST_WRONG_KIND_ROUTE_MISSING");
    for number in [54, 55] {
        assert_eq!(crate::syscalls::region_query(route_holder, number, sex_pdx::SLOT_BLOCK, 0, 4096, QUERY_VIEW as u64), fail);
        checked += 1;
    }
    for (offset, length, rights) in [
        (0, 0, QUERY_VIEW as u64), (before[1], 1, QUERY_VIEW as u64),
        (u64::MAX, 2, QUERY_VIEW as u64), (0, 4096, 0x81),
        (0, 4096, (1u64 << 32) | QUERY_VIEW as u64),
    ] {
        assert_eq!(crate::syscalls::region_query(pd, 54, slot, offset, length, rights), fail);
        checked += 1;
    }
    assert_eq!(crate::syscalls::region_query(pd, 55, slot, 0, 0, 0), before);
    let first = crate::syscalls::region_query(pd, 54, slot, 0, 4096, QUERY_VIEW as u64);
    assert_eq!(first[0], 0);
    assert_eq!(first[3], 0); // Ordinary view discloses no physical address.
    for _ in 0..1024 { assert_eq!(crate::syscalls::region_query(pd, 54, slot, 0, 4096, QUERY_VIEW as u64), first); }
    crate::serial_println!("[memory.authority.negative.selftest] denied={} idempotent=1024 output_zero=1 ok=1", checked);
}

pub fn pci_bar0(pd: &ProtectionDomain, dev: &crate::hal::pci::PciDevice, slot: u64) -> RegionGrant {
    require_boot();
    let (physical, actual_length) = dev.probe_boot_bar0().expect("BOOT_MMIO_INVALID_BAR");
    // The active drivers need at most 64 KiB; a larger BAR grants only this prefix.
    let length = actual_length.min(64 * 1024);
    pci_view(pd,slot,physical,actual_length,length)
}

/// Only a supported BOOT-selected controller with a complete register prefix.
/// Reject before aliasing; probing itself remains guarded by BOOT RegistryBuild.
pub fn nic_bar0(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64)->Option<RegionGrant>{
    require_boot();
    if !crate::nic_boot::supported(dev.vendor_id,dev.device_id){return None}
    let(physical,actual_length)=dev.probe_boot_bar0()?;
    let length=crate::nic_boot::view_length(actual_length)?;
    Some(pci_view(pd,slot,physical,actual_length,length))
}

pub fn nic_command(pd:&ProtectionDomain,dev:&crate::hal::pci::PciDevice,slot:u64,view:RegionGrant)
    -> sex_pdx::pci_command::Grant
{
    require_boot();
    assert!(slot == sex_pdx::SLOT_NIC && crate::nic_boot::supported(dev.vendor_id,dev.device_id), "BOOT_NIC_COMMAND_SUBJECT");
    assert_eq!(view.region_id,0x10000 + pd.id * 1024 + slot as u32,"BOOT_NIC_COMMAND_VIEW_ID");
    assert!(sex_pdx::memory_region::resolve_info(Some(&view)).is_ok()
        && view.length >= crate::nic_boot::REGISTERS_END && view.length <= 65536
        && view.rights == QUERY_VIEW|READ|WRITE && view.dma_base.is_none() && view.peer_pd.is_none(), "BOOT_NIC_COMMAND_VIEW");
    let g=sex_pdx::pci_command::Grant {subject:pd.id,bus:dev.bus,device:dev.dev,function:dev.func,
        vendor_id:dev.vendor_id,device_id:dev.device_id,allowed:sex_pdx::pci_command::NIC_BITS};
    assert!(sex_pdx::pci_command::authorize(Some(&g),pd.id,0,0).is_ok(),"BOOT_NIC_COMMAND_GRANT");
    crate::serial_println!("[boot.nic.command] pd={} slot={} bus={} device={} function={} vendor={} product={} allowed={} dma_region={} runtime_mint=0",
        pd.id,slot,g.bus,g.device,g.function,g.vendor_id,g.device_id,g.allowed,sex_pdx::pci_command::DMA_REGION_ID);
    g
}

fn pci_view(pd:&ProtectionDomain,slot:u64,physical:u64,actual_length:u64,length:u64)->RegionGrant{
    require_boot();
    let virtual_base = alias(pd, physical, length, true, true);
    let region = RegionGrant { region_id: 0x10000 + pd.id * 1024 + slot as u32,
        virtual_base, length, rights: QUERY_VIEW | READ | WRITE, dma_base: None, peer_pd: None };
    sex_pdx::memory_region::resolve_info(Some(&region)).expect("BOOT_MMIO_INVALID_VIEW");
    crate::serial_println!("[boot.memory.mmio] pd={} slot={} bytes={} actual_bytes={}", pd.id, slot, length, actual_length);
    region
}

pub fn seal_all() {
    require_boot();
    for id in 1..crate::ipc::MAX_DOMAINS as u32 {
        if let Some(pd) = DOMAIN_REGISTRY.get(id) {
            unsafe { (*pd.cap_table).seal(); }
        }
    }
    crate::serial_println!("[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0");
}

/// Prepare one bounded paging arena. The runtime commit mechanism receives
/// exact existing leaf addresses and never invokes a frame/VA/table allocator.
pub fn init_paging(pager_pd: u32, client_pds: [u32; 2]) {
    use crate::paging_state::{FramePoolGrant,PagerMappingGrant,DemandRegionGrant,PAGE,PAGES,CLIENTS};
    use x86_64::structures::paging::{Mapper,Page,Size4KiB};
    require_boot();
    assert!(pager_pd != 0 && client_pds[0] != client_pds[1],"PAGING_BOOT_DOMAINS");
    let pager = DOMAIN_REGISTRY.get(pager_pd).expect("PAGING_BOOT_PAGER_MISSING");
    let length = PAGE * PAGES as u64;
    let physical = backing(length);
    let pool_va = alias(pager,physical,length,true,false);
    let pool = FramePoolGrant {pool_id:1,base_va:pool_va,length};
    let mut mappings = [PagerMappingGrant {range_id:0,pool_id:1,target_pd:0,base_va:0,length,rights:READ|WRITE,quota:4};CLIENTS];
    let mut pkeys = [0u8;CLIENTS];
    let mut leaves = [[0u64;PAGES];CLIENTS];
    for (index,client_pd) in client_pds.into_iter().enumerate() {
        assert!(client_pd != 0 && client_pd != pager_pd,"PAGING_BOOT_DOMAINS");
        let client = DOMAIN_REGISTRY.get(client_pd).expect("PAGING_BOOT_CLIENT_MISSING");
        assert!(client.pku_key != pager.pku_key && !pkeys[..index].contains(&client.pku_key),"PAGING_BOOT_PKEY_ALIAS");
        pkeys[index] = client.pku_key;
        // These temporary aliases precreate intermediate infrastructure only.
        // Every target leaf is absent before any domain begins execution.
        let target_va = alias(client,physical,length,true,false);
        {
            let mut vas = crate::memory::manager::GLOBAL_VAS.lock();
            let vas = vas.as_mut().expect("PAGING_BOOT_VAS_MISSING");
            for (page_index,leaf) in leaves[index].iter_mut().enumerate() {
                let address = target_va + page_index as u64 * PAGE;
                vas.mapper.unmap(Page::<Size4KiB>::containing_address(VirtAddr::new(address)))
                    .expect("PAGING_BOOT_UNMAP_FAILED").1.flush();
                *leaf = existing_leaf_address(address).expect("PAGING_BOOT_LEAF_MISSING");
                assert_eq!(unsafe { core::ptr::read_volatile(*leaf as *const u64) },0,"PAGING_BOOT_LEAF_PRESENT");
            }
        }
        let mapping = PagerMappingGrant {range_id:index as u32+1,pool_id:1,target_pd:client_pd,
            base_va:target_va,length,rights:READ|WRITE,quota:4};
        mappings[index] = mapping;
        crate::scheduler::register_paging_exit_watch(client_pd,pager_pd);
        let region = DemandRegionGrant {range_id:mapping.range_id,base_va:target_va,length,rights:READ|WRITE};
        let map_slot = if index == 0 { sex_pdx::paging::SLOT_PAGER_MAPPING } else { sex_pdx::paging::SLOT_PAGER_MAPPING_SECOND };
        for (pd,slot,data) in [
            (pager,map_slot,CapabilityData::PagerMapping(mapping)),
            (client,sex_pdx::paging::SLOT_DEMAND_REGION,CapabilityData::DemandRegion(region)),
        ] {
            assert!(pd.find_capability(slot as u32).is_none(),"PAGING_BOOT_SLOT_OCCUPIED");
            pd.grant_capability(slot,data);
        }
        crate::serial_println!("[boot.paging.genesis] pager={} target={} pool_pages={} arena_pages={} quota={} target_pkey={} leaves_absent=8 tables_preallocated=1",
            pager_pd,client_pd,PAGES,PAGES,mapping.quota,client.pku_key);
    }
    assert!(pager.find_capability(sex_pdx::paging::SLOT_FRAME_POOL as u32).is_none(),"PAGING_BOOT_SLOT_OCCUPIED");
    pager.grant_capability(sex_pdx::paging::SLOT_FRAME_POOL,CapabilityData::FramePool(pool));
    crate::paging::initialize(pool,mappings,pager_pd,pkeys,physical,leaves);
}

pub(crate) fn existing_leaf_address(address: u64) -> Option<u64> {
    use x86_64::registers::control::Cr3;
    let hhdm = crate::HHDM_REQUEST.response()?.offset;
    let mut table = Cr3::read().0.start_address().as_u64();
    for shift in [39,30,21] {
        let entry = unsafe { core::ptr::read_volatile((hhdm + table + ((address >> shift)&511)*8) as *const u64) };
        if entry & 1 == 0 || entry & (1<<7) != 0 { return None; }
        table = entry & 0x000f_ffff_ffff_f000;
    }
    Some(hhdm + table + ((address >> 12)&511)*8)
}
