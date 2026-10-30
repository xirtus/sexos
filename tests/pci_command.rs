//! Actual authorizer, typed kernel invocation and port mechanism. Only capability
//! tables, interrupt/lock primitives and config-space ports are host doubles.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[macro_export]macro_rules! serial_println{($($a:tt)*)=>{}}
pub const SLOT_DMA_REGION:u64=32;
#[path="../crates/sex-pdx/src/pci_command.rs"]pub mod pci_command;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../kernel/src/capability_slot.rs"]mod capability_slot;
#[path="../kernel/src/nic_boot.rs"]mod nic_boot;
#[path="../kernel/src/pci_command.rs"]mod mechanism;
mod ipc{pub const MAX_DOMAINS:usize=1024;}
use pci_command as w;
use memory_region as m;
use std::{cell::Cell,collections::BTreeMap,sync::{Mutex,atomic::{AtomicUsize,Ordering}}};
mod capability{
 use super::*;
 #[derive(Clone,Copy)]pub struct PciCapData{pub bus:u8,pub dev:u8,pub func:u8,pub vendor_id:u16,pub device_id:u16,pub bar0_view:Option<m::RegionGrant>,pub command:Option<w::Grant>}
 #[derive(Clone,Copy)]pub enum CapabilityData{Pci(PciCapData),Region(m::RegionGrant),Domain(u32)}
 pub struct Capability{pub data:CapabilityData}
 pub struct ProtectionDomain{pub id:u32,pub entries:Vec<(u32,CapabilityData)>,pub lookups:Cell<usize>}
 impl ProtectionDomain{pub fn find_capability(&self,slot:u32)->Option<Capability>{self.lookups.set(self.lookups.get()+1);self.entries.iter().find(|(s,_)|*s==slot).map(|(_,data)|Capability{data:*data})}}
}
mod spin{
 pub struct Mutex<T>(std::sync::Mutex<T>);
 impl<T>Mutex<T>{pub const fn new(value:T)->Self{Self(std::sync::Mutex::new(value))}pub fn lock(&self)->std::sync::MutexGuard<'_,T>{self.0.lock().unwrap()}}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum Op{Address(u32),Read32(u32),Write16(u32,u16),Write32(u32,u32)}
struct Model{selected:u32,config:BTreeMap<u32,u32>,events:Vec<Op>,refuse:bool,accept_mask:u16}
static MODEL:Mutex<Option<Model>>=Mutex::new(None);
static CRITICAL:AtomicUsize=AtomicUsize::new(0);
mod x86_64{pub mod instructions{
 pub mod interrupts{
  pub fn without_interrupts<R>(f:impl FnOnce()->R)->R{crate::CRITICAL.fetch_add(1,crate::Ordering::SeqCst);struct Guard;impl Drop for Guard{fn drop(&mut self){crate::CRITICAL.fetch_sub(1,crate::Ordering::SeqCst);}}let _g=Guard;f()}
 }
 pub mod port{
  use crate::*;
  pub trait Value:Copy{fn read(port:u16)->Self;fn write(port:u16,value:Self);}
  pub struct Port<T>{port:u16,_t:std::marker::PhantomData<T>}
  impl<T:Value>Port<T>{pub fn new(port:u16)->Self{Self{port,_t:std::marker::PhantomData}}pub unsafe fn read(&mut self)->T{assert!(CRITICAL.load(Ordering::SeqCst)>0);T::read(self.port)}pub unsafe fn write(&mut self,value:T){assert!(CRITICAL.load(Ordering::SeqCst)>0);T::write(self.port,value)}}
  impl Value for u32{
   fn read(port:u16)->Self{assert_eq!(port,0xcfc);let mut m=MODEL.lock().unwrap();let m=m.as_mut().unwrap();let a=m.selected;m.events.push(Op::Read32(a));*m.config.get(&a).unwrap_or(&u32::MAX)}
   fn write(port:u16,v:Self){let mut m=MODEL.lock().unwrap();let m=m.as_mut().unwrap();if port==0xcf8{m.selected=v;m.events.push(Op::Address(v));return}assert_eq!(port,0xcfc);let a=m.selected;m.events.push(Op::Write32(a,v));let old=*m.config.get(&a).unwrap_or(&0);m.config.insert(a,if a&0xfc==4{(old&0xffff0000&!(v&0xffff0000))|(v&0xffff)}else{v});}
  }
  impl Value for u16{
   fn read(_:u16)->Self{panic!("unexpected16-bit read")}
   fn write(port:u16,v:Self){assert_eq!(port,0xcfc);let mut m=MODEL.lock().unwrap();let m=m.as_mut().unwrap();let a=m.selected;m.events.push(Op::Write16(a,v));if !m.refuse{let old=*m.config.get(&a).unwrap();let mask=m.accept_mask as u32;m.config.insert(a,(old&!mask)|(v as u32&mask));}}
  }
 }
}}
mod hal{pub mod pci{
 use crate::{spin,x86_64};static PCI_CONFIG_LOCK:spin::Mutex<()>=spin::Mutex::new(());
 include!(env!("PCI_COMMAND_PORT_SOURCE"));
}}
use capability::*;
fn grant()->w::Grant{w::Grant{subject:13,bus:0,device:2,function:0,vendor_id:0x8086,device_id:0x10d3,allowed:w::NIC_BITS}}
fn address(offset:u32)->u32{0x8000_1000|offset}
fn pd()->ProtectionDomain{
 let view=m::RegionGrant{region_id:0x10000+13*1024+19,virtual_base:0x40000000,length:65536,rights:7,dma_base:None,peer_pd:None};
 let dma=m::RegionGrant{region_id:2,virtual_base:0x50000000,length:256*1024,rights:15,dma_base:Some(0x100000),peer_pd:None};
 ProtectionDomain{id:13,lookups:Cell::new(0),entries:vec![(19,CapabilityData::Pci(PciCapData{bus:0,dev:2,func:0,vendor_id:0x8086,device_id:0x10d3,bar0_view:Some(view),command:Some(grant())})),(32,CapabilityData::Region(dma)),(6,CapabilityData::Domain(3))]}
}
fn setup(command:u16){*MODEL.lock().unwrap()=Some(Model{selected:0,config:BTreeMap::from([(address(0),0x10d3_8086),(address(4),0xa5a5_0000|command as u32)]),events:vec![],refuse:false,accept_mask:u16::MAX});assert_eq!(CRITICAL.load(Ordering::SeqCst),0);}
fn invoke(pd:&ProtectionDomain,slot:u64,set:u64,clear:u64)->[u64;5]{mechanism::invoke(pd,slot,set,clear,0,0)}
fn untouched(){assert!(MODEL.lock().unwrap().as_ref().unwrap().events.is_empty());}
fn mutate_pci(pd:&mut ProtectionDomain,f:impl FnOnce(&mut PciCapData)){let CapabilityData::Pci(p)=&mut pd.entries[0].1 else{panic!()};f(p);}
#[test]fn full_width_subject_and_mask_validation_precedes_all_ports(){
 for(slot,set,clear,a,b)in[(0,0,0,0,0),(1025,0,0,0,0),((1u64<<32)|19,0,0,0,0),(19,1,0,0,0),(19,2,2,0,0),(19,1u64<<32,0,0,0),(19,0,u64::MAX,0,0),(19,0,0,1,0),(19,0,0,0,1),(6,0,0,0,0),(32,0,0,0,0)]{
  setup(0);let p=pd();assert_eq!(mechanism::invoke(&p,slot,set,clear,a,b),[w::DENIED,0,0,0,0]);untouched();
  if slot==0||slot>1024||a!=0||b!=0{assert_eq!(p.lookups.get(),0);}
 }
}
#[test]fn missing_wrong_subject_device_or_bar_permission_rejects_before_ports(){
 for case in 0..9{setup(0);let mut p=pd();mutate_pci(&mut p,|d|match case{
  0=>d.command=None,1=>d.command.as_mut().unwrap().subject=12,2=>d.command.as_mut().unwrap().device=3,
  3=>d.bar0_view=None,4=>d.bar0_view.as_mut().unwrap().rights=3,5=>d.bar0_view.as_mut().unwrap().region_id+=1,
  6=>d.bar0_view.as_mut().unwrap().peer_pd=Some(3),7=>d.bar0_view.as_mut().unwrap().length=4096,
  _=>d.command.as_mut().unwrap().allowed=0xffff,
 });assert_eq!(invoke(&p,19,2,0),[w::DENIED,0,0,0,0]);untouched();}
}
#[test]fn bus_master_enable_requires_exact_own_dma_region_but_disable_and_query_do_not(){
 for case in 0..6{
  setup(4);let mut p=pd();if case==0{p.entries.remove(1);}else{let CapabilityData::Region(g)=&mut p.entries[1].1 else{panic!()};match case{1=>g.region_id=3,2=>g.length=4096,3=>g.rights=7,4=>g.peer_pd=Some(2),_=>g.dma_base=Some(1)}}
  assert_eq!(invoke(&p,19,4,0),[w::DENIED,0,0,0,0]);untouched();
  assert_eq!(invoke(&p,19,0,0),[0,4,4,0,0]);assert_eq!(invoke(&p,19,0,4),[0,4,0,1,0]);
 }
}
#[test]fn command_updates_only_granted_bits_with16bit_writes_preserving_status(){
 setup(0x0141);let p=pd();assert_eq!(invoke(&p,19,6,0),[0,0x141,0x147,1,0]);
 assert_eq!(invoke(&p,19,0,4),[0,0x147,0x143,1,0]);
 let m=MODEL.lock().unwrap();let m=m.as_ref().unwrap();assert_eq!(m.config[&address(4)],0xa5a5_0143);
 assert_eq!(m.events.iter().filter(|v|matches!(v,Op::Write16(_, _))).count(),2);assert!(!m.events.iter().any(|v|matches!(v,Op::Write32(_, _))));
 assert!(m.events.iter().all(|e|match e{Op::Address(a)|Op::Read32(a)=>*a==address(0)||*a==address(4),Op::Write16(a,_)=>*a==address(4),_=>false}));
}
#[test]fn repeated_enable_and_read_only_query_do_not_rewrite_configuration(){
 setup(6);let p=pd();for _ in 0..100{assert_eq!(invoke(&p,19,6,0),[0,6,6,0,0]);assert_eq!(invoke(&p,19,0,0),[0,6,6,0,0]);}
 assert!(!MODEL.lock().unwrap().as_ref().unwrap().events.iter().any(|e|matches!(e,Op::Write16(_,_)|Op::Write32(_,_))));
}
#[test]fn changed_hardware_or_failed_readback_returns_zero_outputs_without_silent_retry(){
 setup(0);MODEL.lock().unwrap().as_mut().unwrap().config.insert(address(0),0x1234_8086);
 assert_eq!(invoke(&pd(),19,6,0),[w::IO,0,0,0,0]);assert!(!MODEL.lock().unwrap().as_ref().unwrap().events.iter().any(|v|matches!(v,Op::Write16(_,_))));
 setup(0);MODEL.lock().unwrap().as_mut().unwrap().refuse=true;assert_eq!(invoke(&pd(),19,6,0),[w::IO,0,0,0,0]);assert_eq!(MODEL.lock().unwrap().as_ref().unwrap().events.iter().filter(|v|matches!(v,Op::Write16(_,_))).count(),1);assert_eq!(CRITICAL.load(Ordering::SeqCst),0);
}
#[test]fn pure_grants_reject_malformed_or_cross_subject_authority(){
 assert_eq!(w::authorize(None,13,0,0),Err(w::DENIED));
 for g in [w::Grant{subject:0,..grant()},w::Grant{device:32,..grant()},w::Grant{function:8,..grant()},w::Grant{vendor_id:0xffff,..grant()},w::Grant{allowed:0,..grant()}]{assert_eq!(w::authorize(Some(&g),13,0,0),Err(w::DENIED));}
 assert_eq!(w::authorize(Some(&grant()),14,0,0),Err(w::DENIED));
 let limited=w::Grant{allowed:w::MEMORY,..grant()};assert!(w::authorize(Some(&limited),13,2,0).is_ok());assert_eq!(w::authorize(Some(&limited),13,4,0),Err(w::DENIED));
}
#[test]fn partial_hardware_change_is_uncertain_and_can_be_inspected_without_retry(){
 setup(0);MODEL.lock().unwrap().as_mut().unwrap().accept_mask=w::MEMORY;
 let p=pd();assert_eq!(invoke(&p,19,6,0),[w::IO,0,0,0,0]);assert_eq!(invoke(&p,19,0,0),[0,2,2,0,0]);
 let m=MODEL.lock().unwrap();let m=m.as_ref().unwrap();assert_eq!(m.config[&address(4)],0xa5a5_0002);assert_eq!(m.events.iter().filter(|e|matches!(e,Op::Write16(_,_))).count(),1);
}

#[test]fn boot_identity_uses_exact_held_authority_without_any_port_access(){
 setup(0);let p=pd();assert_eq!(mechanism::identity(&p,19,0,0,0,0),[0,0x8086,0x10d3,0,0]);untouched();
 for (slot,a,b,c,d)in[(0,0,0,0,0),(19|(1u64<<32),0,0,0,0),(6,0,0,0,0),(32,0,0,0,0),(19,1,0,0,0),(19,0,1,0,0),(19,0,0,1,0),(19,0,0,0,1)]{assert_eq!(mechanism::identity(&p,slot,a,b,c,d),[w::DENIED,0,0,0,0]);untouched();}
 for which in 0..5{let mut p=pd();mutate_pci(&mut p,|d|match which{0=>d.command=None,1=>d.command.as_mut().unwrap().subject=12,2=>d.device_id=0x100e,3=>d.bar0_view.as_mut().unwrap().rights=3,_=>d.bar0_view.as_mut().unwrap().peer_pd=Some(9)});assert_eq!(mechanism::identity(&p,19,0,0,0,0),[w::DENIED,0,0,0,0]);untouched();}
}
