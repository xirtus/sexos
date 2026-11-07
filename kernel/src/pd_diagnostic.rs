use crate::ipc::DOMAIN_REGISTRY;
use crate::capability::CapabilityData;
use core::sync::atomic::Ordering;

pub fn dump_pd_map(reason: &'static str) {
    crate::serial_println!("[pd.map.begin] reason={}", reason);
    
    let mut count = 0;
    
    for id in 1..crate::program_records::BASE {
        if let Some(pd) = DOMAIN_REGISTRY.get(id) {
            count += 1;
            
            let pkey = pd.pku_key;
            let base_pkru = pd.base_pkru_mask;
            let current_pkru = pd.current_pkru_mask.load(Ordering::Acquire);
            let msg_ring = pd.message_ring as u64;
            let cap_table = pd.cap_table as u64;
            
            let mut entry = 0u64;
            let task_ptr = pd.main_task.load(Ordering::Acquire);
            if !task_ptr.is_null() {
                unsafe {
                    entry = (*task_ptr).context.rip;
                }
            }
            
            crate::serial_println!("[pd.map] id={} name=untracked pkey={} base_pkru={:#x} current_pkru={:#x} entry={:#x} msg_ring={:#x} cap_table={:#x}",
                id, pkey, base_pkru, current_pkru, entry, msg_ring, cap_table
            );
            
            if !pd.cap_table.is_null() {
                unsafe {
                    for (i, slot) in (*pd.cap_table).slots.iter().enumerate() {
                        let cap_ptr = slot.load(Ordering::Acquire);
                        if !cap_ptr.is_null() {
                            let cap = &*cap_ptr;
                            match cap.data {
                                CapabilityData::Memory(mem) => {
                                    crate::serial_println!("[pd.cap] pd={} idx={} kind=memory range={:#x}..{:#x} pkey={} rights={:#x}",
                                        id, i, mem.cheri_cap.base, mem.cheri_cap.base + mem.cheri_cap.length, mem.pku_key, mem.cheri_cap.permissions);
                                }
                                CapabilityData::MemLend(lend) => {
                                    crate::serial_println!("[pd.cap] pd={} idx={} kind=memlend range={:#x}..{:#x} pkey={} rights={:#x}",
                                        id, i, lend.base, lend.base + lend.length, lend.pku_key, lend.permissions);
                                }
                                CapabilityData::DMA(dma) => {
                                    crate::serial_println!("[pd.cap] pd={} idx={} kind=dma range={:#x}..{:#x} pkey={} rights=rw",
                                        id, i, dma.phys_addr, dma.phys_addr + dma.length, dma.pku_key);
                                }
                                CapabilityData::IPC(ipc) => {
                                    crate::serial_println!("[pd.cap] pd={} idx={} kind=ipc target={} rights=call", id, i, ipc.target_pd_id);
                                }
                                CapabilityData::Domain(target_id) => {
                                    crate::serial_println!("[pd.cap] pd={} idx={} kind=domain target={} rights=msg", id, i, target_id);
                                }
                                CapabilityData::Interrupt(intr) => {
                                    crate::serial_println!("[pd.cap] pd={} idx={} kind=interrupt target={} rights=recv", id, i, intr.irq);
                                }
                                _ => {
                                    crate::serial_println!("[pd.cap] pd={} idx={} kind=other", id, i);
                                }
                            }
                        }
                    }
                }
            }
            
            crate::serial_println!("[pd.map.missing] field=elf_ranges reason=untracked");
            crate::serial_println!("[pd.map.missing] field=owned_ranges reason=untracked");
        }
    }
    
    crate::serial_println!("[pd.map.end] count={}", count);
}
