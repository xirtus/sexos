//! Actual BOOT NIC view constructor; probe/alias/phase are explicit host mocks.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_println{($($t:tt)*)=>{}}
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../crates/sex-pdx/src/pci_command.rs"]pub mod pci_command;
#[path="../kernel/src/nic_boot.rs"]mod nic_boot;
use memory_region::{RegionGrant,QUERY_VIEW,READ,WRITE};
use std::sync::Mutex;
/// probe answers BAR0 (Intel); bars answers other BAR indices (Realtek BAR2/BAR1).
struct Model{boot:bool,probe:Option<(u64,u64)>,probes:usize,maps:Vec<(u32,u64,u64)>,bars:Vec<(u8,u64,u64)>,asked:Vec<u8>,shared:bool}
static MODEL:Mutex<Model>=Mutex::new(Model{boot:true,probe:None,probes:0,maps:Vec::new(),bars:Vec::new(),asked:Vec::new(),shared:false});
struct ProtectionDomain{id:u32}
fn require_boot(){let boot=MODEL.lock().unwrap().boot;assert!(boot,"after BOOT seal");}
fn alias(pd:&ProtectionDomain,physical:u64,length:u64,writable:bool,mmio:bool)->u64{
 require_boot();assert!(writable&&mmio);MODEL.lock().unwrap().maps.push((pd.id,physical,length));0x40000000
}
mod hal{pub mod pci{
 pub struct PciDevice{pub vendor_id:u16,pub device_id:u16,pub bus:u8,pub dev:u8,pub func:u8}
 pub fn page_exclusive(_:u64,_:u8,_:u8,_:u8,_:u8)->bool{!crate::MODEL.lock().unwrap().shared}
 impl PciDevice{pub fn probe_boot_bar_any(&self,index:u8)->Option<(u64,u64)>{crate::require_boot();let mut m=crate::MODEL.lock().unwrap();m.probes+=1;m.asked.push(index);
  if index==0{m.probe}else{m.bars.iter().find(|b|b.0==index).map(|b|(b.1,b.2))}}}
}}
include!(env!("NIC_BOOT_MEMORY_SOURCE"));
fn setup(probe:Option<(u64,u64)>){*MODEL.lock().unwrap()=Model{boot:true,probe,probes:0,maps:vec![],bars:vec![],asked:vec![],shared:false};}
#[test]fn unsupported_controllers_never_probe_or_alias_a_bar(){
 for(vendor_id,device_id)in[(0x1af4,0x1000),(0x10ec,0x8139),(0x8086,0xffff)]{
  setup(Some((0x800000,131072)));assert_eq!(nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id,device_id,bus:0,dev:2,func:0},19),None);
  let m=MODEL.lock().unwrap();assert_eq!(m.probes,0);assert!(m.maps.is_empty());
 }
}
#[test]fn missing_or_short_probed_bar_is_rejected_before_alias(){
 for probe in [None,Some((0x800000,4096)),Some((0x800000,16384))]{
  setup(probe);assert_eq!(nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x8086,device_id:0x10d3,bus:0,dev:2,func:0},19),None);
  let m=MODEL.lock().unwrap();assert_eq!(m.probes,1);assert!(m.maps.is_empty());
 }
}
#[test]fn supported_full_prefix_binds_exact_boot_subject_slot_and_never_exposes_dma_address(){
 for length in [32768,65536,131072]{
  setup(Some((0x800000,length)));let g=nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x8086,device_id:0x10d3,bus:0,dev:2,func:0},19).unwrap();
  assert_eq!(g.region_id,0x10000+13*1024+19);assert_eq!(g.length,length.min(65536));assert_eq!(g.rights,QUERY_VIEW|READ|WRITE);assert_eq!(g.dma_base,None);assert_eq!(g.peer_pd,None);
  assert!(memory_region::resolve_info(Some(&g)).is_ok());let m=MODEL.lock().unwrap();assert_eq!(m.probes,1);assert_eq!(m.maps,[(13,0x800000,length.min(65536))]);
 }
}
#[test]fn sealed_phase_cannot_probe_or_create_a_new_nic_alias(){
 setup(Some((0x800000,131072)));MODEL.lock().unwrap().boot=false;
 let error=std::panic::catch_unwind(||nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x8086,device_id:0x10d3,bus:0,dev:2,func:0},19));assert!(error.is_err());
 let m=MODEL.lock().unwrap();assert_eq!(m.probes,0);assert!(m.maps.is_empty());
}
#[test]fn realtek_leases_its_register_window_from_bar2_then_bar1_never_the_io_bar0(){
 for(bars,expect,asked)in[(vec![(2u8,0x900000u64,4096u64),(1,0xa00000,256)],Some((0x900000u64,4096u64)),vec![2u8]),(vec![(1,0xa00000,4096)],Some((0xa00000,4096)),vec![2,1]),(vec![(2,0x900000,0x80),(1,0xa00000,4096)],None,vec![2]),(vec![],None,vec![2,1])]{
  setup(Some((0xc000,256)));MODEL.lock().unwrap().bars=bars;
  let g=nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x10ec,device_id:0x8168,bus:0,dev:2,func:0},19);
  let m=MODEL.lock().unwrap();assert_eq!(m.asked,asked);
  match expect{Some((physical,length))=>{let g=g.unwrap();assert_eq!(g.length,length);assert_eq!(m.maps,[(13,physical,length)]);},None=>{assert!(g.is_none());assert!(m.maps.is_empty());}}
 }
}
#[test]fn advanced_intel_controllers_need_their_queue_registers_inside_the_view(){
 for(device_id,length,ok)in[(0x10c9u16,0xe000u64,false),(0x10c9,65536,true),(0x1533,131072,true),(0x15f3,0x8000,false),(0x15f3,65536,true),(0x15b8,32768,true)]{
  setup(Some((0x800000,length)));let g=nic_bar0(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x8086,device_id,bus:0,dev:2,func:0},19);assert_eq!(g.is_some(),ok,"{:#x}",device_id);
 }
}
#[test]fn sub_page_register_window_leases_its_whole_page_only_when_nothing_else_decodes_there(){
 for shared in [false,true]{
  setup(Some((0xc000,256)));{let mut m=MODEL.lock().unwrap();m.bars=vec![(2,0x900100,256)];m.shared=shared;}
  let got=nic_lease(&ProtectionDomain{id:13},&hal::pci::PciDevice{vendor_id:0x10ec,device_id:0x8169,bus:0,dev:2,func:0},19);
  let m=MODEL.lock().unwrap();
  if shared{assert!(got.is_none());assert!(m.maps.is_empty());}
  else{let(g,window)=got.unwrap();assert_eq!(g.length,4096);assert_eq!(window,(0x100,256));assert_eq!(m.maps,[(13,0x900000,4096)]);}
 }
}
