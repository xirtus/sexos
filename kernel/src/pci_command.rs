// SPDX-License-Identifier: GPL-3.0-only
//! Transition mechanism for immutable device-command authority, not a PCI API.
use crate::capability::{CapabilityData,ProtectionDomain};
use sex_pdx::{pci_command as w,memory_region as m};

pub fn invoke(pd:&ProtectionDomain,slot:u64,set:u64,clear:u64,reserved0:u64,reserved1:u64)->[u64;5] {
    let fail=[w::DENIED,0,0,0,0];
    if reserved0!=0 || reserved1!=0 { return fail; }
    let Some(slot)=crate::capability_slot::decode_capability_slot(slot) else {return fail;};
    let Some(CapabilityData::Pci(device))=pd.find_capability(slot).map(|c|c.data) else {return fail;};
    let Ok(update)=w::authorize(device.command.as_ref(),pd.id,set,clear) else {return fail;};
    let g=update.grant();
    if (g.bus,g.device,g.function,g.vendor_id,g.device_id)!=(device.bus,device.dev,device.func,device.vendor_id,device.device_id) {return fail;}
    let Some(view)=device.bar0_view else {return fail;};
    let expected=pd.id.checked_mul(1024).and_then(|n|n.checked_add(0x10000)).and_then(|n|n.checked_add(slot));
    if expected!=Some(view.region_id) || m::resolve_info(Some(&view)).is_err()
        || view.rights!=m::QUERY_VIEW|m::READ|m::WRITE || view.dma_base.is_some() || view.peer_pd.is_some()
        || view.length<crate::nic_boot::REGISTERS_END || view.length>65536 {return fail;}
    // Only bus-master enable needs DMA authority; querying or disabling a
    // controller remains possible when DMA metadata is absent or invalid.
    if update.needs_dma() {
        let dma=match pd.find_capability(sex_pdx::SLOT_DMA_REGION as u32).map(|c|c.data) {
            Some(CapabilityData::Region(g))=>g,_=>return fail,
        };
        if dma.region_id!=w::DMA_REGION_ID || dma.length!=w::DMA_BYTES || dma.peer_pd.is_some()
            || m::resolve_view(Some(&dma),m::ViewRequest {offset:0,length:dma.length,
                rights:m::QUERY_VIEW|m::READ|m::WRITE|m::DMA_ADDRESS}).is_err() {return fail;}
    }
    match crate::hal::pci::apply_command(update) {
        Ok(r)=>{
            crate::serial_println!("[pci.command.apply] pd={} slot={} before={} after={} set={} clear={} changed={}",pd.id,slot,r.before,r.after,set,clear,r.changed as u8);
            [0,r.before as u64,r.after as u64,r.changed as u64,0]
        },
        Err(error)=>[error,0,0,0,0],
    }
}
