//! Actual production state machine; explicit register/command/clock doubles.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/pci_command.rs"] pub mod pci_command;
#[path="../crates/sex-pdx/src/memory_region.rs"] pub mod memory_region;
#[path="../crates/sex-pdx/src/dma_pool.rs"] pub mod dma_pool;
#[path="../servers/sexnet/src/ethernet.rs"] mod ethernet;
const SLOT_NIC:u64=19;
const SLOT_DMA_REGION:u64=32;
fn device_identity(_:u64)->Result<pci_command::Identity,u64>{Ok(pci_command::Identity{vendor:0x8086,product:0x10d3})}
use std::sync::Mutex;
use memory_region::{MemoryRegionInfo,RegionView};
static INFO:Mutex<MemoryRegionInfo>=Mutex::new(MemoryRegionInfo{region_id:78867,length:65536,rights:7,peer_pd:None});
static VIEW:Mutex<RegionView>=Mutex::new(RegionView{region_id:78867,length:65536,virtual_address:0x200000,dma_address:None});
fn memory_region_info(_:u64)->Result<MemoryRegionInfo,u64>{Ok(*INFO.lock().unwrap())}
fn memory_region_full_view(_:u64,_:u8)->Result<RegionView,u64>{Ok(*VIEW.lock().unwrap())}
fn device_command(_:u64,_:u64,_:u64)->Result<pci_command::Reply,u64>{panic!("Live command not mocked in these tests")}
#[path="../servers/sexnet/src/nic.rs"] mod nic;
use nic::{Device,Hardware,Phase,Failure};
use pci_command::{Reply,MEMORY,BUS_MASTER};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::cell::RefCell;
#[derive(Clone,Debug,PartialEq,Eq)]enum Op{Command(u16,u16),Read(u32),Write(u32,u32)}
#[derive(Clone)]struct Mock{inner:Rc<RefCell<State>>}
struct State{command:u16,registers:BTreeMap<u32,u32>,ops:Vec<Op>,reset_stuck:bool,disable_stuck:bool,command_error:Option<usize>,command_count:usize,changed_before:bool}
impl Mock{fn new()->Self{Self{inner:Rc::new(RefCell::new(State{command:0x101,registers:BTreeMap::from([(nic::CTRL,0x40),(nic::STATUS,0x80080783),(nic::RAL,0x12005452),(nic::RAH,0x80005634)]),ops:vec![],reset_stuck:false,disable_stuck:false,command_error:None,command_count:0,changed_before:false}))}}
 fn operations(&self)->Vec<Op>{self.inner.borrow().ops.clone()}}
impl Hardware for Mock{
 fn command(&mut self,set:u16,clear:u16)->Result<Reply,u64>{let mut s=self.inner.borrow_mut();s.ops.push(Op::Command(set,clear));s.command_count+=1;
  if s.command_error==Some(s.command_count){return Err(pci_command::IO)}
  if s.changed_before&&set!=0{s.command|=BUS_MASTER;}
  let before=s.command;s.command=(before|set)&!clear;Ok(Reply{before,after:s.command,changed:before!=s.command})}
 fn read(&mut self,offset:u32)->u32{let mut s=self.inner.borrow_mut();s.ops.push(Op::Read(offset));*s.registers.get(&offset).unwrap_or(&0)}
 fn write(&mut self,offset:u32,value:u32){let mut s=self.inner.borrow_mut();s.ops.push(Op::Write(offset,value));
  if offset==nic::IMC{s.registers.insert(nic::IMS,0);}
  else if offset==nic::CTRL&&!s.reset_stuck{s.registers.insert(offset,value&!(1<<26));}
  else{let stuck=s.disable_stuck&&matches!(offset,nic::RCTL|nic::TCTL);s.registers.insert(offset,if stuck{2}else{value});}}}
fn finish(d:&mut Device<Mock>){for n in 0..10{d.advance(n)}assert_eq!(d.phase(),Phase::Quiescent);}
#[test]fn ordered_reset_is_bounded_per_turn_with_real_mac_and_no_bus_master_or_queue_writes(){
 let m=Mock::new();let mut d=Device::new(m.clone(),0);
 for n in 0..10{let before=m.operations().len();d.advance(n);assert!(m.operations().len()-before<=8);}
 assert_eq!(d.phase(),Phase::Quiescent);assert_eq!(d.mac,[0x52,0x54,0,0x12,0x34,0x56]);assert_eq!(d.status,0x80080783);
 assert_eq!(m.inner.borrow().command,0x103);let ops=m.operations();
 assert_eq!(&ops[..2],&[Op::Command(0,0),Op::Command(MEMORY,BUS_MASTER)]);
 assert_eq!(ops.iter().filter(|op|matches!(op,Op::Write(n,_)if *n==nic::CTRL)).count(),1);
 assert_eq!(ops.iter().filter(|op|matches!(op,Op::Write(n,_)if *n==nic::IMC)).count(),2);
 assert!(ops.iter().all(|op|match op{Op::Command(set,_)=>set&BUS_MASTER==0,Op::Write(n,_)=>[nic::IMC,nic::RCTL,nic::TCTL,nic::CTRL].contains(n),_=>true}));
 let terminal=ops.len();for n in [10,2000,u64::MAX]{d.advance(n)}assert_eq!(m.operations().len(),terminal);
}
#[test]fn settling_has_no_hardware_access_before_clock_deadline(){
 let m=Mock::new();let mut d=Device::new(m.clone(),10);d.advance(10);let count=m.operations().len();
 d.advance(10);d.advance(11);assert_eq!(m.operations().len(),count);assert_eq!(d.phase(),Phase::Drain);
 d.advance(12);assert_eq!(d.phase(),Phase::Reset);let count=m.operations().len();d.advance(12);d.advance(13);assert_eq!(m.operations().len(),count);d.advance(14);assert_eq!(d.phase(),Phase::Verify);d.advance(15);assert_eq!(d.phase(),Phase::Quiescent);
}
#[test]fn already_mastering_or_changed_command_never_touches_mmio(){
 for race in [false,true]{let m=Mock::new();if race{m.inner.borrow_mut().changed_before=true}else{m.inner.borrow_mut().command|=BUS_MASTER;}
 let mut d=Device::new(m.clone(),0);d.advance(0);assert_eq!(d.phase(),Phase::Quarantined);assert!(m.operations().iter().all(|op|matches!(op,Op::Command(..))));
 assert_eq!(d.failure(),Some(if race{Failure::Command}else{Failure::UnknownMaster}));}
}
#[test]fn uncertain_command_is_never_retried_and_no_mmio_follows(){
 for failure_at in [1,2,3]{let m=Mock::new();m.inner.borrow_mut().command_error=Some(failure_at);let mut d=Device::new(m.clone(),0);
 for n in 0..10{d.advance(n)}assert_eq!(d.failure(),Some(Failure::Command));let before=m.operations();for n in 10..30{d.advance(n)}assert_eq!(m.operations(),before);assert_eq!(m.inner.borrow().command_count,failure_at);
 if failure_at<=2{assert!(before.iter().all(|op|matches!(op,Op::Command(..))));}}
}
#[test]fn stopped_clock_stuck_reset_and_backward_clock_terminate_without_reset_retry(){
 let m=Mock::new();let mut d=Device::new(m.clone(),0);for _ in 0..8193{d.advance(0)}assert_eq!(d.failure(),Some(Failure::Deadline));assert_eq!(m.operations().iter().filter(|op|matches!(op,Op::Write(n,_)if *n==nic::CTRL)).count(),0);
 let m=Mock::new();m.inner.borrow_mut().reset_stuck=true;let mut d=Device::new(m.clone(),0);for n in 0..2001{d.advance(n)}assert_eq!(d.failure(),Some(Failure::Deadline));assert_eq!(m.operations().iter().filter(|op|matches!(op,Op::Write(n,_)if *n==nic::CTRL)).count(),1);
 let m=Mock::new();let mut d=Device::new(m.clone(),100);d.advance(100);let count=m.operations().len();d.advance(99);assert_eq!(d.failure(),Some(Failure::Clock));assert_eq!(m.operations().len(),count);
}
#[test]fn failed_disable_or_missing_device_is_quarantined_before_reset(){
 for missing in [false,true]{let m=Mock::new();if missing{m.inner.borrow_mut().registers.insert(nic::CTRL,u32::MAX);}else{m.inner.borrow_mut().disable_stuck=true;}
 let mut d=Device::new(m.clone(),0);for n in 0..10{d.advance(n)}assert_eq!(d.failure(),Some(Failure::Registers));assert!(!m.operations().iter().any(|op|matches!(op,Op::Write(n,_)if *n==nic::CTRL)));}
}
#[test]fn invalid_station_addresses_cannot_become_quiescent_receipts(){
 for (lo,hi)in[(0,0x80000000),(0x12005453,0x80005634),(0x12005452,0x5634),(u32::MAX,u32::MAX)]{
 let m=Mock::new();m.inner.borrow_mut().registers.insert(nic::RAL,lo);m.inner.borrow_mut().registers.insert(nic::RAH,hi);let mut d=Device::new(m.clone(),0);for n in 0..10{d.advance(n)}assert_eq!(d.failure(),Some(Failure::Address));assert_eq!(d.mac,[0;6]);}
}
#[test]fn live_constructor_requires_exact_current_boot_bar_identity_rights_peer_span_and_no_dma(){
 let info=*INFO.lock().unwrap();let view=*VIEW.lock().unwrap();assert!(nic::Live::open().is_ok());
 for bad in [MemoryRegionInfo{region_id:2,..info},MemoryRegionInfo{rights:15,..info},MemoryRegionInfo{rights:3,..info},MemoryRegionInfo{peer_pd:Some(9),..info},MemoryRegionInfo{length:0x5404,..info},MemoryRegionInfo{length:65537,..info}]{*INFO.lock().unwrap()=bad;assert!(nic::Live::open().is_err());}*INFO.lock().unwrap()=info;
 for bad in [RegionView{region_id:2,..view},RegionView{length:4096,..view},RegionView{virtual_address:1,..view},RegionView{virtual_address:memory_region::USER_VA_END-4096,..view},RegionView{dma_address:Some(0x4000),..view}]{*VIEW.lock().unwrap()=bad;assert!(nic::Live::open().is_err());}*VIEW.lock().unwrap()=view;
}
