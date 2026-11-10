// SPDX-License-Identifier: GPL-3.0-only
//! BOOT policy for the SexNet PCI register layouts, not runtime authority.
use sex_pdx::pci_command::{Family, Identity, Model};
pub fn model(vendor:u16,device:u16)->Option<Model>{ Identity{vendor,product:device}.model() }
pub fn supported(vendor:u16,device:u16)->bool{ model(vendor,device).is_some() }
/// The fixed register prefix a driver of this controller touches.
pub fn registers_end(vendor:u16,device:u16)->u64{
    model(vendor,device).map_or(u64::MAX,|m|m.registers_end())
}
/// Length of the view leased for this controller: its register prefix, capped at 64 KiB.
pub fn model_view_length(vendor:u16,device:u16,actual:u64)->Option<u64>{
    let end=registers_end(vendor,device);
    if actual<end{None}else{Some(actual.min(64*1024))}
}
/// The memory BAR holding the registers. Intel parts use BAR0. Realtek parts put
/// I/O ports in BAR0 and the register window in BAR2 (PCIe) or BAR1 (old PCI 8169).
pub fn bars(vendor:u16,device:u16)->&'static [u8]{
    match model(vendor,device).map(|m|m.family()) {
        Some(Family::Realtek)=>&[2,1],
        _=>&[0],
    }
}
