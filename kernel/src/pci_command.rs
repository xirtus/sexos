// SPDX-License-Identifier: GPL-3.0-only
//! Transition mechanism for immutable device-command authority, not a PCI API.
use crate::capability::{CapabilityData,ProtectionDomain};
use sex_pdx::{pci_command as w,memory_region as m};

fn held(pd:&ProtectionDomain,slot:u64,set:u64,clear:u64)->Option<(u32,w::Update)> {
    held_device(pd,slot,set,clear).map(|(slot,update,_)|(slot,update))
}

fn held_device(pd:&ProtectionDomain,slot:u64,set:u64,clear:u64)->Option<(u32,w::Update,crate::capability::PciCapData)> {
    let slot=crate::capability_slot::decode_capability_slot(slot)?;
    let CapabilityData::Pci(device)=pd.find_capability(slot)?.data else {return None;};
    let update=w::authorize(device.command.as_ref(),pd.id,set,clear).ok()?;
    let g=update.grant();
    if (g.bus,g.device,g.function,g.vendor_id,g.device_id)!=(device.bus,device.dev,device.func,device.vendor_id,device.device_id) {return None;}
    let view=device.bar0_view?;
    let expected=pd.id.checked_mul(1024).and_then(|n|n.checked_add(0x10000)).and_then(|n|n.checked_add(slot));
    if expected!=Some(view.region_id) || m::resolve_info(Some(&view)).is_err()
        || view.rights!=m::QUERY_VIEW|m::READ|m::WRITE || view.dma_base.is_some() || view.peer_pd.is_some()
        || view.length>65536 {return None;}
    // A virtio lease is sized by its register layout, a NIC's by its fixed registers.
    let complete=match device.virtio {
        Some(layout)=>crate::virtio_boot::supported(device.vendor_id,device.device_id)&&layout.valid(view.length),
        None=>match crate::device_lease::Kind::of_class(device.class) {
            Some(kind)=>view.length>=kind.registers_end(),
            None=>view.length>=crate::nic_boot::registers_end(device.vendor_id,device.device_id),
        },
    };
    if !complete {return None;}
    Some((slot,update,device))
}

pub fn identity(pd:&ProtectionDomain,slot:u64,a:u64,b:u64,c:u64,d:u64)->[u64;5] {
    let fail=[w::DENIED,0,0,0,0];
    if a==w::QUERY_WINDOW && b|c|d==0 { return window(pd,slot); }
    if a==w::QUERY_SUBSYSTEM && b|c|d==0 { return subsystem(pd,slot); }
    if a|b|c|d!=0{return fail;}
    let Some((slot,update))=held(pd,slot,0,0)else{return fail;};
    let grant=update.grant();
    crate::serial_println!("[pci.device.identity] pd={} slot={} vendor={} product={} runtime_mint=0",pd.id,slot,grant.vendor_id,grant.device_id);
    [0,grant.vendor_id as u64,grant.device_id as u64,0,0]
}

/// Where the held virtio device's register blocks sit inside its leased view.
pub fn virtio_layout(pd:&ProtectionDomain,slot:u64,page:u64,a:u64,b:u64,c:u64)->[u64;5]{
    let fail=[w::DENIED,0,0,0,0];
    if a|b|c!=0{return fail;}
    let Some((_,_,device))=held_device(pd,slot,0,0)else{return fail;};
    let Some(words)=device.virtio.and_then(|layout|layout.page(page))else{return fail;};
    [0,words[0],words[1],words[2],words[3]]
}

/// A USB network adapter's frames come through BOOT's shared region with SexUSB, not a
/// PCI lease: holding that exact region also lets the network owner read its settings.
fn usb_link(pd:&ProtectionDomain,slot:u64)->bool{
    slot==sex_pdx::usb_net::SLOT&&matches!(pd.find_capability(slot as u32).map(|c|c.data),
        Some(CapabilityData::Region(g)) if g.region_id==sex_pdx::usb_net::REGION_ID&&g.length==sex_pdx::usb_net::BYTES)
}

/// Read BOOT data only after the exact held device lease passes sex-pdx checks.
pub fn network_config(pd:&ProtectionDomain,slot:u64,a:u64,b:u64,c:u64,d:u64)->[u64;5]{
    let fail=[w::DENIED,0,0,0,0];
    if a|b|c|d!=0||(held(pd,slot,0,0).is_none()&&!usb_link(pd,slot)){return fail;}
    let Some(words)=crate::boot_network::query(pd.id)else{return fail;};
    crate::serial_println!("[pci.network.config] pd={} slot={} origin=boot_descriptor runtime_mint=0",pd.id,slot);
    [0,words[0],words[1],words[2],words[3]]
}

pub fn invoke(pd:&ProtectionDomain,slot:u64,set:u64,clear:u64,reserved0:u64,reserved1:u64)->[u64;5] {
    let fail=[w::DENIED,0,0,0,0];
    if reserved0!=0 || reserved1!=0 { return fail; }
    let Some((slot,update,device))=held_device(pd,slot,set,clear)else{return fail;};
    let (region_id,bytes)=if device.virtio.is_some(){(sex_pdx::virtio::DMA_REGION_ID,sex_pdx::virtio::DMA_BYTES)}
        else if let Some(kind)=crate::device_lease::Kind::of_class(device.class){kind.dma()}
        else{(w::DMA_REGION_ID,w::DMA_BYTES)};
    // Only bus-master enable needs DMA authority; querying or disabling a
    // controller remains possible when DMA metadata is absent or invalid.
    let dma_slot=crate::device_lease::Kind::of_class(device.class).map_or(sex_pdx::SLOT_DMA_REGION,|k|k.dma_slot());
    if update.needs_dma() {
        let dma=match pd.find_capability(dma_slot as u32).map(|c|c.data) {
            Some(CapabilityData::Region(g))=>g,_=>return fail,
        };
        if dma.region_id!=region_id || dma.length!=bytes || dma.peer_pd.is_some()
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

/// When the kernel restarts a service, devices it held stop DMA first: the new run
/// finds them idle, as at BOOT, and proves custody again before enabling them.
pub fn quiesce(pd:&ProtectionDomain){
    for slot in 0..1024u32 {
        let Some(CapabilityData::Pci(device))=pd.find_capability(slot).map(|c|c.data)else{continue};
        let Ok(update)=w::authorize(device.command.as_ref(),pd.id,0,w::BUS_MASTER as u64)else{continue};
        match crate::hal::pci::apply_command(update) {
            Ok(r)=>{crate::serial_println!("[pci.command.quiesce] pd={} slot={} before={} after={}",pd.id,slot,r.before,r.after);}
            Err(error)=>{crate::serial_println!("[pci.command.quiesce] pd={} slot={} error={}",pd.id,slot,error);}
        }
    }
}

/// Where the held device's registers sit inside its leased view: the whole view,
/// or for a BAR smaller than a page, its offset and length inside that page.
/// Subsystem vendor and device (config offset 0x2c) of the exact held device.
fn subsystem(pd:&ProtectionDomain,slot:u64)->[u64;5]{
    let Some((_,_,device))=held_device(pd,slot,0,0)else{return [w::DENIED,0,0,0,0];};
    let id=unsafe{crate::hal::pci::pci_config_read(device.bus,device.dev,device.func,0x2c)};
    [0,(id&0xffff) as u64,(id>>16) as u64,0,0]
}

fn window(pd:&ProtectionDomain,slot:u64)->[u64;5]{
    let Some((_,_,device))=held_device(pd,slot,0,0)else{return [w::DENIED,0,0,0,0];};
    let Some(view)=device.bar0_view else{return [w::DENIED,0,0,0,0];};
    let (offset,length)=match device.window { (0,0)=>(0,view.length), (o,l)=>(o as u64,l as u64) };
    [0,offset,length,0,0]
}
