//! Actual BOOT command constructor; phase and discovered controller are doubles.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_println{($($a:tt)*)=>{}}
pub const SLOT_NIC:u64=19;
#[path="../crates/sex-pdx/src/pci_command.rs"]pub mod pci_command;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../kernel/src/nic_boot.rs"]mod nic_boot;
use memory_region::{RegionGrant,QUERY_VIEW,READ,WRITE};
use std::sync::atomic::{AtomicBool,Ordering};
static BOOT:AtomicBool=AtomicBool::new(true);
fn require_boot(){assert!(BOOT.load(Ordering::SeqCst),"sealed");}
pub struct ProtectionDomain{id:u32}
mod hal{pub mod pci{pub struct PciDevice{pub bus:u8,pub dev:u8,pub func:u8,pub vendor_id:u16,pub device_id:u16}}}
include!(env!("NIC_COMMAND_BOOT_SOURCE"));
fn device()->hal::pci::PciDevice{hal::pci::PciDevice{bus:2,dev:7,func:1,vendor_id:0x8086,device_id:0x10d3}}
fn view(pd:u32)->RegionGrant{RegionGrant{region_id:0x10000+pd*1024+19,virtual_base:0x40000000,length:65536,rights:7,dma_base:None,peer_pd:None}}
#[test]fn exact_boot_holder_device_and_two_bit_mask_are_not_inferred_from_the_bar(){
 BOOT.store(true,Ordering::SeqCst);
 for id in [13,27]{let g=nic_command(&ProtectionDomain{id},&device(),19,view(id));assert_eq!(g,pci_command::Grant{subject:id,bus:2,device:7,function:1,vendor_id:0x8086,device_id:0x10d3,allowed:6});}
}
#[test]fn unsupported_wrong_slot_or_inconsistent_bar_cannot_sanction_command_authority(){
 BOOT.store(true,Ordering::SeqCst);
 let pd=ProtectionDomain{id:13};let bad=hal::pci::PciDevice{vendor_id:0x1af4,device_id:0x1000,..device()};
 assert!(std::panic::catch_unwind(||nic_command(&pd,&bad,19,view(13))).is_err());
 for slot in [0,18,20,(1u64<<32)|19]{assert!(std::panic::catch_unwind(||nic_command(&pd,&device(),slot,view(13))).is_err());}
 for g in [view(14),RegionGrant{rights:3,..view(13)},RegionGrant{length:4096,..view(13)},RegionGrant{peer_pd:Some(3),..view(13)},RegionGrant{dma_base:Some(0x100000),rights:15,..view(13)}]{assert!(std::panic::catch_unwind(||nic_command(&pd,&device(),19,g)).is_err());}
}
#[test]fn sealing_prevents_new_command_authority(){
 BOOT.store(false,Ordering::SeqCst);assert!(std::panic::catch_unwind(||nic_command(&ProtectionDomain{id:13},&device(),19,view(13))).is_err());BOOT.store(true,Ordering::SeqCst);
}
