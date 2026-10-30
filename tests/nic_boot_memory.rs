//! Actual BOOT NIC view constructor; probe/alias/phase are explicit host mocks.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_println{($($t:tt)*)=>{}}
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../kernel/src/nic_boot.rs"]mod nic_boot;
use memory_region::{RegionGrant,QUERY_VIEW,READ,WRITE};
use std::sync::Mutex;
struct Model{boot:bool,probe:Option<(u64,u64)>,probes:usize,maps:Vec<(u32,u64,u64)>}
static MODEL:Mutex<Model>=Mutex::new(Model{boot:true,probe:None,probes:0,maps:Vec::new()});
struct ProtectionDomain{id:u32}
fn require_boot(){let boot=MODEL.lock().unwrap().boot;assert!(boot,"after BOOT seal");}
fn alias(pd:&ProtectionDomain,physical:u64,length:u64,writable:bool,mmio:bool)->u64{
 require_boot();assert!(writable&&mmio);MODEL.lock().unwrap().maps.push((pd.id,physical,length));0x40000000
}
mod hal{pub mod pci{
 pub struct PciDevice{pub vendor_id:u16,pub device_id:u16}
 impl PciDevice{pub fn probe_boot_bar0(&self)->Option<(u64,u64)>{crate::require_boot();let mut m=crate::MODEL.lock().unwrap();m.probes+=1;m.probe}}
}}
include!(env!("NIC_BOOT_MEMORY_SOURCE"));
fn setup(probe:Option<(u64,u64)>){*MODEL.lock().unwrap()=Model{boot:true,probe,probes:0,maps:vec![]};}
#[test]fn unsupported_controllers_never_probe_or_alias_a_bar(){
 for(vendor_id,device_id)in[(0x1af4,0x1000),(0x10ec,0x8168),(0x8086,0xffff)]{
  setup(Some((0x800000,131072)));assert_eq!(nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id,device_id},19),None);
  let m=MODEL.lock().unwrap();assert_eq!(m.probes,0);assert!(m.maps.is_empty());
 }
}
#[test]fn missing_or_short_probed_bar_is_rejected_before_alias(){
 for probe in [None,Some((0x800000,4096)),Some((0x800000,16384))]{
  setup(probe);assert_eq!(nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x8086,device_id:0x10d3},19),None);
  let m=MODEL.lock().unwrap();assert_eq!(m.probes,1);assert!(m.maps.is_empty());
 }
}
#[test]fn supported_full_prefix_binds_exact_boot_subject_slot_and_never_exposes_dma_address(){
 for length in [32768,65536,131072]{
  setup(Some((0x800000,length)));let g=nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x8086,device_id:0x10d3},19).unwrap();
  assert_eq!(g.region_id,0x10000+13*1024+19);assert_eq!(g.length,length.min(65536));assert_eq!(g.rights,QUERY_VIEW|READ|WRITE);assert_eq!(g.dma_base,None);assert_eq!(g.peer_pd,None);
  assert!(memory_region::resolve_info(Some(&g)).is_ok());let m=MODEL.lock().unwrap();assert_eq!(m.probes,1);assert_eq!(m.maps,[(13,0x800000,length.min(65536))]);
 }
}
#[test]fn sealed_phase_cannot_probe_or_create_a_new_nic_alias(){
 setup(Some((0x800000,131072)));MODEL.lock().unwrap().boot=false;
 let error=std::panic::catch_unwind(||nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x8086,device_id:0x10d3},19));assert!(error.is_err());
 let m=MODEL.lock().unwrap();assert_eq!(m.probes,0);assert!(m.maps.is_empty());
}
