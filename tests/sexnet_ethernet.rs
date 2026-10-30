//! Actual fixed-queue driver with explicit device/DMA/command doubles.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../crates/sex-pdx/src/dma_pool.rs"]pub mod dma_pool;
#[path="../crates/sex-pdx/src/pci_command.rs"]pub mod pci_command;
#[path="../servers/sexnet/src/nic.rs"]mod nic;
#[path="../servers/sexnet/src/ethernet.rs"]mod ethernet;
#[path="../servers/sexnet/src/arp.rs"]mod arp;
use memory_region as m;use pci_command as c;
const SLOT_NIC:u64=19;const SLOT_DMA_REGION:u64=32;
use std::{cell::RefCell,rc::Rc,collections::BTreeMap,sync::Mutex};
static DMA_INFO:Mutex<m::MemoryRegionInfo>=Mutex::new(m::MemoryRegionInfo{region_id:2,length:c::DMA_BYTES,rights:15,peer_pd:None});
static DMA_VIEW:Mutex<m::RegionView>=Mutex::new(m::RegionView{region_id:2,length:c::DMA_BYTES,virtual_address:0x50000000,dma_address:Some(0x100000)});
fn memory_region_info(slot:u64)->Result<m::MemoryRegionInfo,u64>{Ok(if slot==19{m::MemoryRegionInfo{region_id:78867,length:65536,rights:7,peer_pd:None}}else{*DMA_INFO.lock().unwrap()})}
fn memory_region_full_view(slot:u64,_:u8)->Result<m::RegionView,u64>{Ok(if slot==19{m::RegionView{region_id:78867,length:65536,virtual_address:0x200000,dma_address:None}}else{*DMA_VIEW.lock().unwrap()})}
fn device_identity(_:u64)->Result<c::Identity,u64>{Ok(c::Identity{vendor:0x8086,product:0x10d3})}
fn device_command(_:u64,_:u64,_:u64)->Result<c::Reply,u64>{panic!("Live command not mocked")}
#[derive(Clone,Debug,PartialEq,Eq)]enum Op{Command(u16,u16),Read(u32),Write(u32,u32),Load(u32),Store(u32),CopyIn(u32,usize),CopyOut(u32,usize),Publish,Acquire,Resolve}
#[derive(Clone)]struct Mock(Rc<RefCell<State>>);
struct State{model:c::Model,mmio:u64,view:m::RegionView,command:u16,regs:BTreeMap<u32,u32>,dma:Vec<u8>,ops:Vec<Op>,refuse:bool,uncertain:bool,timing:Option<u32>}
impl Mock{
 fn new(model:c::Model)->Self{Self(Rc::new(RefCell::new(State{model,mmio:65536,view:*DMA_VIEW.lock().unwrap(),command:0x103,regs:BTreeMap::from([(nic::CTRL,0x40),(nic::STATUS,2),(0x5b00,0x1000)]),dma:vec![0xa5;c::DMA_BYTES as usize],ops:vec![],refuse:false,uncertain:false,timing:None})))}
 fn word(&self,n:u32)->u64{u64::from_le_bytes(self.0.borrow().dma[n as usize..n as usize+8].try_into().unwrap())}
 fn set(&self,n:u32,v:u64){self.0.borrow_mut().dma[n as usize..n as usize+8].copy_from_slice(&v.to_le_bytes());}
 fn rx(&self,index:u32,bytes:&[u8],status:u8,errors:u8,length:usize){let mut s=self.0.borrow_mut();let start=8192+index as usize*2048;s.dma[start..start+bytes.len()].copy_from_slice(bytes);drop(s);self.set(index*16+8,length as u64|((status as u64)<<32)|((errors as u64)<<40));}
 fn done(&self,index:u32,status:u8){let old=self.word(4096+index*16+8);self.set(4096+index*16+8,(old&!(255u64<<32))|((status as u64)<<32));}
 fn ops(&self)->Vec<Op>{self.0.borrow().ops.clone()}
}
impl nic::Hardware for Mock{
 fn command(&mut self,set:u16,clear:u16)->Result<c::Reply,u64>{let mut s=self.0.borrow_mut();s.ops.push(Op::Command(set,clear));let before=s.command;
  if set&4!=0{
   assert_eq!(s.regs.get(&0x2808),Some(&128));assert_eq!(s.regs.get(&0x3808),Some(&128));
   for i in 0..8usize{let word=|n:usize|u64::from_le_bytes(s.dma[n..n+8].try_into().unwrap());assert_eq!(word(i*16),0x100000+8192+i as u64*2048);assert_eq!(word(i*16+8),0);assert_eq!(word(4096+i*16),0x100000+24576+i as u64*2048);assert_eq!(word(4096+i*16+8),1<<32);}
   assert!(s.dma[8192..40960].iter().all(|&x|x==0));
  }
  s.command=(before|set)&!clear;if s.uncertain&&set&4!=0{return Err(c::IO)}Ok(c::Reply{before,after:s.command,changed:before!=s.command})}
 fn read(&mut self,n:u32)->u32{let mut s=self.0.borrow_mut();s.ops.push(Op::Read(n));*s.regs.get(&n).unwrap_or(&0)}
 fn write(&mut self,n:u32,v:u32){let mut s=self.0.borrow_mut();s.ops.push(Op::Write(n,v));if n==0x410&&s.timing.is_some(){let timing=s.timing.unwrap();s.regs.insert(n,timing);}else if !(s.refuse&&n==0x2808){s.regs.insert(n,v);}}
}
impl ethernet::Hardware for Mock{
 fn model(&self)->c::Model{self.0.borrow().model}fn mmio_length(&self)->u64{self.0.borrow().mmio}
 fn dma_region(&mut self)->Result<m::RegionView,u64>{let mut s=self.0.borrow_mut();s.ops.push(Op::Resolve);Ok(s.view)}
 fn load64(&mut self,n:u32)->u64{self.0.borrow_mut().ops.push(Op::Load(n));self.word(n)}
 fn store64(&mut self,n:u32,v:u64){self.0.borrow_mut().ops.push(Op::Store(n));self.set(n,v)}
 fn load_bytes(&mut self,n:u32,out:&mut[u8]){let mut s=self.0.borrow_mut();s.ops.push(Op::CopyOut(n,out.len()));out.copy_from_slice(&s.dma[n as usize..n as usize+out.len()]);}
 fn store_bytes(&mut self,n:u32,bytes:&[u8]){let mut s=self.0.borrow_mut();s.ops.push(Op::CopyIn(n,bytes.len()));s.dma[n as usize..n as usize+bytes.len()].copy_from_slice(bytes);}
 fn publish(&mut self){self.0.borrow_mut().ops.push(Op::Publish)}fn acquire(&mut self){self.0.borrow_mut().ops.push(Op::Acquire)}
}
use ethernet::{Driver,Phase,Failure,Event,SendError};
const MAC:[u8;6]=[0x52,0x54,0,0x12,0x34,0x56];
fn ready(m:&Mock)->Driver<Mock>{let mut d=Driver::new(m.clone(),MAC,0);for n in 0..40{d.advance(n);if d.phase()==Phase::Running{return d;}}panic!("not ready {:?} {:?}",d.phase(),d.failure());}
#[test]fn both_model_profiles_initialize_entire_fixed_layout_before_bus_master_without_reserved_queue_enable(){
 for model in [c::Model::I82540,c::Model::I82574]{let m=Mock::new(model);let mut d=Driver::new(m.clone(),MAC,0);
 for n in 0..40{let start=m.ops().len();d.advance(n);assert!(m.ops().len()-start<=512);if d.phase()==Phase::Running{break;}}
 assert_eq!(d.phase(),Phase::Running);let s=m.0.borrow();assert_eq!(s.command,0x107);assert_eq!(s.regs[&0x2818],7);assert_eq!(s.regs[&0x3818],0);assert_eq!(s.regs[&0x2828]&(1<<25),0);assert_eq!(s.regs[&0x3828]&(1<<25),0);
 assert_eq!(s.regs[&0x410],if model==c::Model::I82574{0x602006}else{0x60200a});assert_eq!(s.regs[&0x3828],if model==c::Model::I82574{1<<22}else{0});assert!(s.dma[40960..].iter().all(|&x|x==0xa5));}
}
#[test]fn dma_metadata_unknown_master_short_bar_and_bad_station_fail_before_any_dma_store(){
 for which in 0..7{let m=Mock::new(c::Model::I82574);let mut mac=MAC;{let mut s=m.0.borrow_mut();match which{0=>s.view.region_id=3,1=>s.view.length=4096,2=>s.view.dma_address=None,3=>s.view.virtual_address+=1,4=>s.command|=4,5=>s.mmio=0x5408,_=>mac[0]|=1}}
 let mut d=Driver::new(m.clone(),mac,0);d.advance(0);assert_eq!(d.phase(),Phase::Quarantined);assert!(!m.ops().iter().any(|op|matches!(op,Op::Store(..)|Op::CopyIn(..)|Op::Write(..))));}
}
#[test]fn live_dma_query_requires_exact_own_budget_rights_peer_backing_and_cannot_be_reopened(){
 use ethernet::Hardware;let info=*DMA_INFO.lock().unwrap();let view=*DMA_VIEW.lock().unwrap();
 for bad in [m::MemoryRegionInfo{region_id:3,..info},m::MemoryRegionInfo{length:4096,..info},m::MemoryRegionInfo{rights:7,..info},m::MemoryRegionInfo{peer_pd:Some(9),..info}]{*DMA_INFO.lock().unwrap()=bad;assert!(nic::Live::open().unwrap().dma_region().is_err());}*DMA_INFO.lock().unwrap()=info;
 for bad in [m::RegionView{region_id:3,..view},m::RegionView{length:4096,..view},m::RegionView{virtual_address:1,..view},m::RegionView{dma_address:None,..view},m::RegionView{dma_address:Some(1),..view}]{*DMA_VIEW.lock().unwrap()=bad;assert!(nic::Live::open().unwrap().dma_region().is_err());}*DMA_VIEW.lock().unwrap()=view;
 let mut live=nic::Live::open().unwrap();assert_eq!(live.dma_region(),Ok(view));assert_eq!(live.dma_region(),Err(c::DENIED));
}
#[test]fn register_failure_precedes_bus_master_and_uncertain_enable_never_retries_or_reuses_dma(){
 for uncertain in [false,true]{let m=Mock::new(c::Model::I82574);{let mut s=m.0.borrow_mut();s.refuse=!uncertain;s.uncertain=uncertain;}let mut d=Driver::new(m.clone(),MAC,0);for n in 0..30{d.advance(n);}assert_eq!(d.failure(),Some(if uncertain{Failure::Command}else{Failure::Registers}));let ops=m.ops();for n in 30..60{d.advance(n);}assert_eq!(d.send(&[0;60],100),Err(SendError::Quarantined));assert_eq!(m.ops(),ops);assert_eq!(m.0.borrow().command&4,if uncertain{4}else{0});}
}
#[test]fn transmit_wrap_busy_length_padding_and_exact_completion_preserve_custody(){
 let m=Mock::new(c::Model::I82574);let mut d=ready(&m);let ops=m.ops();for n in [0,13,1515]{assert_eq!(d.send(&vec![1;n],100),Err(SendError::Length));assert_eq!(m.ops(),ops);}
 for sequence in 1..=24u64{let payload=vec![sequence as u8;if sequence%2==0{1514}else{14}];let (seq,index,len,hash)=d.send(&payload,100+sequence).unwrap();assert_eq!((seq,index),(sequence,((sequence-1)%8)as u32));let padded=payload.iter().copied().chain(std::iter::repeat(0)).take(payload.len().max(60)).collect::<Vec<_>>();assert_eq!((len,hash),(padded.len(),ethernet::fingerprint(&padded)));
 let start=24576+index as usize*2048;assert_eq!(&m.0.borrow().dma[start..start+len],&padded);assert!(m.0.borrow().dma[start+len..start+2048].iter().all(|&x|x==0));assert_eq!(m.word(4096+index*16+8),len as u64|(0x0b<<24));
 let ops=m.ops();assert_eq!(d.send(&[0;60],100+sequence),Err(SendError::Busy));assert_eq!(m.ops(),ops);assert!(d.advance(100+sequence).is_none());assert!(d.busy());m.done(index,1);
 assert!(matches!(d.advance(100+sequence),Some(Event::Transmitted{sequence:s,index:i,..})if s==sequence&&i==index));assert!(!d.busy());}
}
#[test]fn receive_wrap_copies_actual_bytes_before_descriptor_return_and_never_uses_returned_address(){
 let m=Mock::new(c::Model::I82540);let mut d=ready(&m);
 for sequence in 1..=24u64{let index=((sequence-1)%8)as u32;let payload=vec![sequence as u8;if sequence%2==0{1514}else{60}];m.rx(index,&payload,3,0,payload.len());let start=m.ops().len();
 match d.advance(100+sequence){Some(Event::Received{sequence:s,index:i,frame})=>{assert_eq!((s,i),(sequence,index));assert_eq!(&frame.bytes[..frame.length],payload);assert!(frame.bytes[frame.length..].iter().all(|&x|x==0));},_=>panic!("missing packet")}
 let ops=&m.ops()[start..];let copied=ops.iter().position(|op|matches!(op,Op::CopyOut(..))).unwrap();let returned=ops.iter().position(|op|matches!(op,Op::Write(0x2818,_))).unwrap();assert!(copied<returned);assert!(ops[copied..returned].contains(&Op::Publish));assert_eq!(m.word(index*16+8),0);}
 m.set(0,u64::MAX);m.rx(0,&[0;60],3,0,60);let start=m.ops().len();assert!(d.advance(200).is_none());assert_eq!(d.failure(),Some(Failure::Descriptor));assert!(!m.ops()[start..].iter().any(|op|matches!(op,Op::CopyOut(..)|Op::Write(..))));
}
#[test]fn malformed_lengths_errors_and_entire_fragment_chain_are_dropped_without_payload_read(){
 let m=Mock::new(c::Model::I82574);let mut d=ready(&m);
 for (index,length,status,errors)in[(0,13,3,0),(1,1515,3,0),(2,60,3,1),(3,60,1,0),(4,60,3,0)]{m.rx(index,&[0x71;60],status,errors,length);let start=m.ops().len();assert!(matches!(d.advance(100),Some(Event::Dropped{..})));assert!(!m.ops()[start..].iter().any(|op|matches!(op,Op::CopyOut(..))));}
 m.rx(5,&[0x42;60],3,0,60);assert!(matches!(d.advance(100),Some(Event::Received{..})));assert_eq!(d.dropped,5);
}
#[test]fn transmit_error_timeout_stopped_clock_and_backward_clock_keep_submitted_memory_quarantined(){
 for which in 0..4{let m=Mock::new(c::Model::I82574);let mut d=ready(&m);d.send(&[0x22;60],100).unwrap();match which{
  0=>{m.done(0,3);d.advance(101);assert_eq!(d.failure(),Some(Failure::Completion));},
  1=>{d.advance(2100);assert_eq!(d.failure(),Some(Failure::Deadline));},
  2=>{for _ in 0..8192{d.advance(100);}assert_eq!(d.failure(),Some(Failure::Deadline));},
  _=>{d.advance(99);assert_eq!(d.failure(),Some(Failure::Clock));}}
 let ops=m.ops();m.done(0,1);for n in 2101..2120{d.advance(n);}assert_eq!(d.send(&[0x33;60],2120),Err(SendError::Quarantined));assert_eq!(m.ops(),ops);assert!(d.busy());}
}
#[test]fn bounded_arp_has_exact_bytes_and_rejects_malformed_spoofed_or_unrelated_frames(){
 let peer=[2,0,0,0,0,1];let mut req=arp::request(peer);req[28..32].copy_from_slice(&arp::GATEWAY);req[38..42].copy_from_slice(&arp::LOCAL);
 let reply=arp::reply(&req,MAC).unwrap();assert_eq!(&reply[..6],&peer);assert_eq!(&reply[6..12],&MAC);assert_eq!(&reply[20..22],&[0,2]);assert_eq!(&reply[22..28],&MAC);assert_eq!(&reply[28..32],&arp::LOCAL);assert_eq!(&reply[32..38],&peer);assert_eq!(&reply[38..42],&arp::GATEWAY);assert!(reply[42..].iter().all(|&x|x==0));
 for length in 0..42{assert!(arp::reply(&req[..length],MAC).is_none());}
 for (index,value)in[(0,0),(6,3),(12,0),(15,2),(16,9),(18,7),(19,6),(21,2),(22,3),(38,192)]{let mut bad=req;bad[index]=value;assert!(arp::reply(&bad,MAC).is_none(),"{}",index);}
 assert_eq!(ethernet::fingerprint(b"hello"),0xa430d84680aabd0b);
}

#[test]fn legacy_unimplemented_timing_does_not_exempt_any_dma_register_or_unknown_nonzero_value(){
 for which in 0..4{let model=if which==2{c::Model::I82574}else{c::Model::I82540};let m=Mock::new(model);m.0.borrow_mut().timing=Some(if which==3{1}else{0});m.0.borrow_mut().refuse=which==1;let mut d=Driver::new(m.clone(),MAC,0);for n in 0..30{d.advance(n);}if which==0{assert_eq!(d.phase(),Phase::Running);assert!(!d.timing_verified);}else{assert_eq!(d.failure(),Some(Failure::Registers));assert_eq!(m.0.borrow().command&4,0);}}
}

#[test]fn protocol_backpressure_holds_rx_without_blocking_exact_tx_completion(){
 let m=Mock::new(c::Model::I82574);let mut d=ready(&m);m.rx(0,&[0x71;60],3,0,60);let start=m.ops().len();assert!(d.advance_with_receive(100,false).is_none());assert!(!m.ops()[start..].iter().any(|op|matches!(op,Op::CopyOut(..)|Op::Write(0x2818,_))));
 d.send(&[0x33;60],100).unwrap();m.done(0,1);assert!(matches!(d.advance_with_receive(101,false),Some(Event::Transmitted{index:0,..})));assert!(matches!(d.advance_with_receive(102,true),Some(Event::Received{index:0,..})));
}
