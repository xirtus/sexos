// SPDX-License-Identifier: GPL-3.0-only
//! BOOT policy for the fixed SexNet PCI register layout, not runtime authority.
pub const REGISTERS_END:u64=0x5408;
pub fn supported(vendor:u16,device:u16)->bool{
    vendor==0x8086 && matches!(device,0x100e|0x10d3)
}
pub fn view_length(actual:u64)->Option<u64>{
    if actual<REGISTERS_END{None}else{Some(actual.min(64*1024))}
}
