//! Actual nic::Device and ethernet::Driver on the Realtek r8169/r8125 path, with
//! an explicit width-aware register file, DMA memory, command bits and clock.
//! QEMU has no 8169 model, so this double is the only proof before hardware.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
#[path="../crates/sex-pdx/src/dma_pool.rs"]pub mod dma_pool;
#[path="../crates/sex-pdx/src/pci_command.rs"]pub mod pci_command;
#[path="../servers/sexnet/src/nic.rs"]mod nic;
#[path="../servers/sexnet/src/ethernet.rs"]mod ethernet;
#[path="../servers/sexnet/src/realtek.rs"]mod realtek;
use memory_region as m;use pci_command as c;use realtek as r;
use std::{cell::RefCell,rc::Rc,sync::Mutex};
const SLOT_NIC:u64=19;const SLOT_DMA_REGION:u64=32;
/// Above 4 GiB, so both halves of every ring address matter.
const PHYSICAL:u64=0x1_0010_0000;
static IDENTITY:Mutex<u16>=Mutex::new(0x8168);
static BAR_INFO:Mutex<m::MemoryRegionInfo>=Mutex::new(m::MemoryRegionInfo{region_id:78867,length:256,rights:7,peer_pd:None});
fn memory_region_info(slot:u64)->Result<m::MemoryRegionInfo,u64>{Ok(if slot==19{*BAR_INFO.lock().unwrap()}else{m::MemoryRegionInfo{region_id:2,length:c::DMA_BYTES,rights:15,peer_pd:None}})}
fn memory_region_full_view(slot:u64,_:u8)->Result<m::RegionView,u64>{Ok(if slot==19{let i=*BAR_INFO.lock().unwrap();m::RegionView{region_id:i.region_id,length:i.length,virtual_address:0x200000,dma_address:None}}else{view()})}
fn device_window(slot:u64)->Result<(u64,u64),u64>{memory_region_full_view(slot,0).map(|v|(0,v.length))}
fn device_identity(_:u64)->Result<c::Identity,u64>{Ok(c::Identity{vendor:0x10ec,product:*IDENTITY.lock().unwrap()})}
fn device_command(_:u64,_:u64,_:u64)->Result<c::Reply,u64>{panic!("Live command not mocked")}
fn view()->m::RegionView{m::RegionView{region_id:2,length:c::DMA_BYTES,virtual_address:0x50000000,dma_address:Some(PHYSICAL)}}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]enum Op{Command(u16,u16),Read(u32,u8),Write(u32,u8,u32),Load(u32),Store(u32),CopyIn(u32,usize),CopyOut(u32,usize),Publish,Acquire,Resolve}
#[derive(Clone)]struct Mock(Rc<RefCell<State>>);
struct State{model:c::Model,mmio:u64,command:u16,regs:[u8;256],dma:Vec<u8>,ops:Vec<Op>,
 /// Reads of ChipCmd that still show RST after a reset write; None: stuck.
 reset_reads:Option<u32>,resetting:Option<u32>,eeprom:[u8;6],absent:bool,locked_writes:u32,stuck_lock:bool}
const VERSION_BITS:u32=0x5400_0000;
impl Mock{
 fn new(model:c::Model)->Self{let mut regs=[0u8;256];let eeprom=[0x00,0xe0,0x4c,0x68,0x00,0x01];regs[..6].copy_from_slice(&eeprom);
  // Firmware left interrupts unmasked and RX accept bits on, engines stopped.
  let (mask,phy)=if model==c::Model::Rtl8125{(0x38,0x80)}else{(0x3c,0x6c)};regs[mask]=0xff;regs[0x44]=0x0e;regs[phy]=0x02|0x10;
  Self(Rc::new(RefCell::new(State{model,mmio:256,command:0x103,regs,dma:vec![0xa5;c::DMA_BYTES as usize],ops:vec![],reset_reads:Some(2),resetting:None,eeprom,absent:false,locked_writes:0,stuck_lock:false})))}
 fn word(&self,n:u32)->u64{u64::from_le_bytes(self.0.borrow().dma[n as usize..n as usize+8].try_into().unwrap())}
 fn set(&self,n:u32,v:u64){self.0.borrow_mut().dma[n as usize..n as usize+8].copy_from_slice(&v.to_le_bytes());}
 fn reg(&self,n:usize,width:usize)->u32{let s=self.0.borrow();let mut v=0u32;for i in 0..width{v|=(s.regs[n+i]as u32)<<(8*i);}v}
 fn ops(&self)->Vec<Op>{self.0.borrow().ops.clone()}
 /// The chip finished receiving into descriptor `index`: payload plus FCS.
 fn rx(&self,index:u32,payload:&[u8],flags:u32){let start=8192+index as usize*2048;let raw=payload.len()+4;
  {let mut s=self.0.borrow_mut();s.dma[start..start+payload.len()].copy_from_slice(payload);s.dma[start+payload.len()..start+raw].copy_from_slice(&[0xde,0xad,0xbe,0xef]);}
  let old=self.word(index*16);self.set(index*16,(old&!0xffff_ffffu64)|(((old as u32)&r::EOR)|flags|raw as u32)as u64);}
 fn rx_raw(&self,index:u32,opts1:u32){let old=self.word(index*16);self.set(index*16,(old&!0xffff_ffffu64)|((old as u32&r::EOR)|opts1)as u64);}
 /// The chip finished transmitting descriptor `index`: OWN cleared.
 fn done(&self,index:u32){let old=self.word(4096+index*16);self.set(4096+index*16,old&!(r::OWN as u64));}
 fn width_write(&self,n:u32,width:u8,v:u32){
  let mut s=self.0.borrow_mut();s.ops.push(Op::Write(n,width,v));if s.absent||(s.stuck_lock&&n==0x50){return;}
  let r8125=s.model==c::Model::Rtl8125;let status=if r8125{0x3c}else{0x3e};
  // Writes the r8169 only takes with the configuration registers unlocked.
  let locked=s.regs[0x50]!=0xc0&&matches!(n,0xda|0xe0|0x20..=0x27|0xe4..=0xeb);
  if locked{s.locked_writes+=1;return;}
  for i in 0..width as usize{let a=n as usize+i;let b=(v>>(8*i))as u8;
   if a>=status&&a<status+if r8125{4}else{2}{s.regs[a]&=!b;}else{s.regs[a]=b;}}
  if n==0x37&&v as u8&r::CMD_RESET!=0{
   // Reset reloads the station address and clears engines, masks and rings.
   let eeprom=s.eeprom;let model=s.model;let regs=&mut s.regs;for (i,x) in regs.iter_mut().enumerate(){if !matches!(i,0x6c|0x80){*x=0;}}
   regs[..6].copy_from_slice(&eeprom);regs[0x37]=r::CMD_RESET;let _=model;s.resetting=s.reset_reads;
  }
 }
 fn width_read(&self,n:u32,width:u8)->u32{
  let mut s=self.0.borrow_mut();s.ops.push(Op::Read(n,width));if s.absent{return if width==4{u32::MAX}else{(1u32<<(8*width))-1};}
  if (n..n+width as u32).contains(&0x37){if let Some(left)=s.resetting{if left==0{s.regs[0x37]&=!r::CMD_RESET;s.resetting=None;}else{s.resetting=Some(left-1);}}}
  let mut v=0u32;for i in 0..width as usize{v|=(s.regs[n as usize+i]as u32)<<(8*i);}
  if n==0x40&&width==4{v|=VERSION_BITS;}v
 }
}
impl nic::Hardware for Mock{
 fn command(&mut self,set:u16,clear:u16)->Result<c::Reply,u64>{
  if set&c::BUS_MASTER!=0{
   // Everything the chip may fetch is in place before it can master the bus,
   // and the engines are still stopped.
   assert_eq!(self.reg(0x20,4),(PHYSICAL+4096)as u32);assert_eq!(self.reg(0x24,4),(PHYSICAL>>32)as u32);
   assert_eq!(self.reg(0xe4,4),PHYSICAL as u32);assert_eq!(self.reg(0xe8,4),(PHYSICAL>>32)as u32);
   assert_eq!(self.reg(0xda,2),2048);assert_eq!(self.reg(0x50,1),0);assert_eq!(self.reg(0x37,1)&0x0c,0);
   for i in 0..ethernet::RX_COUNT{assert_eq!(self.word(i*16),(r::OWN|if i==ethernet::RX_COUNT-1{r::EOR}else{0}|2048)as u64);assert_eq!(self.word(i*16+8),PHYSICAL+8192+i as u64*2048);}
   for i in 0..ethernet::TX_COUNT{assert_eq!(self.word(4096+i*16),if i==ethernet::TX_COUNT-1{r::EOR as u64}else{0});assert_eq!(self.word(4096+i*16+8),PHYSICAL+TX_DATA as u64+i as u64*2048);}
  }
  let mut s=self.0.borrow_mut();s.ops.push(Op::Command(set,clear));let before=s.command;s.command=(before|set)&!clear;Ok(c::Reply{before,after:s.command,changed:before!=s.command})}
 fn read(&mut self,n:u32)->u32{assert_eq!(n%4,0);self.width_read(n,4)}
 fn write(&mut self,n:u32,v:u32){assert_eq!(n%4,0);self.width_write(n,4,v)}
 fn model(&self)->c::Model{self.0.borrow().model}
 fn read8(&mut self,n:u32)->u8{self.width_read(n,1)as u8}
 fn write8(&mut self,n:u32,v:u8){self.width_write(n,1,v as u32)}
 fn read16(&mut self,n:u32)->u16{assert_eq!(n%2,0);self.width_read(n,2)as u16}
 fn write16(&mut self,n:u32,v:u16){assert_eq!(n%2,0);self.width_write(n,2,v as u32)}
}
impl ethernet::Hardware for Mock{
 fn mmio_length(&self)->u64{self.0.borrow().mmio}
 fn dma_region(&mut self)->Result<m::RegionView,u64>{self.0.borrow_mut().ops.push(Op::Resolve);Ok(view())}
 fn load64(&mut self,n:u32)->u64{self.0.borrow_mut().ops.push(Op::Load(n));self.word(n)}
 fn store64(&mut self,n:u32,v:u64){self.0.borrow_mut().ops.push(Op::Store(n));self.set(n,v)}
 fn load_bytes(&mut self,n:u32,out:&mut[u8]){let mut s=self.0.borrow_mut();s.ops.push(Op::CopyOut(n,out.len()));out.copy_from_slice(&s.dma[n as usize..n as usize+out.len()]);}
 fn store_bytes(&mut self,n:u32,bytes:&[u8]){let mut s=self.0.borrow_mut();s.ops.push(Op::CopyIn(n,bytes.len()));s.dma[n as usize..n as usize+bytes.len()].copy_from_slice(bytes);}
 fn publish(&mut self){self.0.borrow_mut().ops.push(Op::Publish)}fn acquire(&mut self){self.0.borrow_mut().ops.push(Op::Acquire)}
}
use ethernet::{Driver,Event,Phase,Failure,SendError};
const TX_DATA:usize=8192+ethernet::RX_COUNT as usize*2048;
const MODELS:[c::Model;2]=[c::Model::Rtl8168,c::Model::Rtl8125];
fn prepared(m:&Mock)->[u8;6]{let mut d=nic::Device::new(m.clone(),0);for n in 0..20{d.advance(n);}assert_eq!(d.phase(),nic::Phase::Quiescent,"{:?}",d.failure());d.into_quiescent().unwrap().1}
fn ready(m:&Mock)->Driver<Mock>{let mac=prepared(m);let mut d=Driver::new(m.clone(),mac,100);for n in 100..220{d.advance(n);if d.phase()==Phase::Running{return d;}}panic!("not ready {:?} {:?}",d.phase(),d.failure());}
fn wrote(ops:&[Op],n:u32)->Vec<(u8,u32)>{ops.iter().filter_map(|op|match op{Op::Write(a,w,v)if *a==n=>Some((*w,*v)),_=>None}).collect()}

#[test]fn identity_maps_realtek_parts_and_their_small_register_window(){
 for(product,model)in[(0x8168,c::Model::Rtl8168),(0x8161,c::Model::Rtl8168),(0x8136,c::Model::Rtl8168),(0x8125,c::Model::Rtl8125),(0x8126,c::Model::Rtl8125)]{
  assert_eq!(c::Identity{vendor:0x10ec,product}.model(),Some(model));assert_eq!(model.family(),c::Family::Realtek);assert!(!model.advanced());assert_eq!(model.registers_end(),r::REGISTERS_END);}
 assert_eq!(c::Identity{vendor:0x10ec,product:0x8139}.model(),None);
 let info=*BAR_INFO.lock().unwrap();
 for(length,ok)in[(256,true),(4096,true),(65536,true),(255,false),(65537,false)]{*BAR_INFO.lock().unwrap()=m::MemoryRegionInfo{length,..info};assert_eq!(nic::Live::open().is_ok(),ok,"{}",length);}
 *BAR_INFO.lock().unwrap()=info;
}
#[test]fn prepare_resets_at_byte_width_masks_per_model_and_reads_the_eeprom_address(){
 for model in MODELS{let m=Mock::new(model);let mut d=nic::Device::new(m.clone(),0);
  for n in 0..20{let start=m.ops().len();d.advance(n);assert!(m.ops().len()-start<=16);}
  assert_eq!(d.phase(),nic::Phase::Quiescent);assert_eq!(d.mac,[0x00,0xe0,0x4c,0x68,0x00,0x01]);assert_eq!(d.status,0x12);
  let ops=m.ops();assert_eq!(&ops[..2],&[Op::Command(0,0),Op::Command(c::MEMORY,c::BUS_MASTER)]);assert_eq!(m.0.borrow().command,0x103);
  // One reset, as a byte; ChipCmd is never covered by a wider write.
  assert_eq!(wrote(&ops,0x37).iter().filter(|w|w.1==r::CMD_RESET as u32).count(),1);assert!(wrote(&ops,0x37).iter().all(|w|w.0==1));
  assert!(!ops.iter().any(|op|matches!(op,Op::Write(a,w,_)if *a<0x37&&*a+*w as u32>0x37)));
  if model==c::Model::Rtl8125{assert!(wrote(&ops,0x38).iter().all(|w|*w==(4,0)));assert!(wrote(&ops,0x3c).iter().all(|w|*w==(4,u32::MAX)));assert_eq!(m.reg(0x38,4),0);}
  else{assert!(wrote(&ops,0x3c).iter().all(|w|*w==(2,0)));assert!(wrote(&ops,0x3e).iter().all(|w|*w==(2,0xffff)));assert_eq!(m.reg(0x3c,2),0);assert!(wrote(&ops,0x38).is_empty());}
  assert_eq!(m.reg(0x44,4)&0x3f,0);
  let terminal=ops.len();for n in [20,2000,u64::MAX]{d.advance(n);}assert_eq!(m.ops().len(),terminal);}
}
#[test]fn absent_stuck_or_unaddressed_chips_quarantine_without_bus_mastering(){
 for model in MODELS{for which in 0..4{let m=Mock::new(model);
  match which{0=>m.0.borrow_mut().absent=true,1=>m.0.borrow_mut().reset_reads=Some(u32::MAX),2=>m.0.borrow_mut().eeprom=[0;6],_=>m.0.borrow_mut().eeprom[0]=1}
  let mut d=nic::Device::new(m.clone(),0);for n in 0..2001{d.advance(n);}
  assert_eq!(d.failure(),Some(match which{0=>nic::Failure::Registers,1=>nic::Failure::Deadline,_=>nic::Failure::Address}),"{:?} {}",model,which);
  assert_eq!(d.mac,[0;6]);assert_eq!(m.0.borrow().command&c::BUS_MASTER,0);
  if which==0{assert!(wrote(&m.ops(),0x37).iter().all(|w|w.1==0));}}}
}
#[test]fn already_mastering_chip_is_never_touched(){
 let m=Mock::new(c::Model::Rtl8168);m.0.borrow_mut().command|=c::BUS_MASTER;let mut d=nic::Device::new(m.clone(),0);d.advance(0);
 assert_eq!(d.failure(),Some(nic::Failure::UnknownMaster));assert!(m.ops().iter().all(|op|matches!(op,Op::Command(..))));
}
#[test]fn driver_programs_rings_unlocked_then_masters_then_starts_engines_per_model(){
 for model in MODELS{let m=Mock::new(model);let mac=prepared(&m);let start=m.ops().len();let mut d=Driver::new(m.clone(),mac,100);
  for n in 100..220{let before=m.ops().len();d.advance(n);assert!(m.ops().len()-before<=1024);if d.phase()==Phase::Running{break;}}
  assert_eq!(d.phase(),Phase::Running,"{:?}",d.failure());assert_eq!(m.0.borrow().locked_writes,0);
  let ops=&m.ops()[start..];let pos=|f:&dyn Fn(&Op)->bool|ops.iter().position(|op|f(op)).unwrap();
  let unlock=pos(&|op|*op==Op::Write(0x50,1,0xc0));let ring=pos(&|op|*op==Op::Write(0xe4,4,PHYSICAL as u32));let lock=pos(&|op|*op==Op::Write(0x50,1,0));
  let master=pos(&|op|*op==Op::Command(c::BUS_MASTER,0));let engines=pos(&|op|*op==Op::Write(0x37,1,(r::CMD_TX|r::CMD_RX)as u32));
  let rx_config=pos(&|op|matches!(op,Op::Write(0x44,4,_)));
  assert!(unlock<ring&&ring<lock&&lock<master&&master<engines&&engines<rx_config);
  assert_eq!(m.reg(0x37,1),(r::CMD_TX|r::CMD_RX)as u32);assert_eq!(m.reg(0x40,4)&!VERSION_BITS,r::TX_CONFIG_VALUE);
  assert_eq!(m.reg(0x44,4),if model==c::Model::Rtl8125{(8<<27)|0x70e}else{0xe70e});assert_eq!(m.reg(0xe0,2)&0x60,0);
  assert_eq!(m.0.borrow().command,0x107);assert!(m.0.borrow().dma[ethernet::USED as usize..].iter().all(|&x|x==0xa5));
  assert!(m.0.borrow().dma[8192..ethernet::USED as usize].iter().all(|&x|x==0));}
}
#[test]fn link_waits_on_the_model_phy_status_register(){
 for model in MODELS{let m=Mock::new(model);let phy=if model==c::Model::Rtl8125{0x80}else{0x6c};m.0.borrow_mut().regs[phy]=0x10;
  let mac=prepared(&m);let mut d=Driver::new(m.clone(),mac,100);for n in 100..200{d.advance(n);}assert_eq!(d.phase(),Phase::Link);
  m.0.borrow_mut().regs[phy]=0x12;d.advance(200);assert_eq!(d.phase(),Phase::Running);
  let m=Mock::new(model);let mac=prepared(&m);let mut d=Driver::new(m.clone(),mac,100);m.0.borrow_mut().regs[phy]=0;for n in 100..5200{d.advance(n);}assert_eq!(d.failure(),Some(Failure::Deadline));}
}
#[test]fn locked_or_absent_registers_fail_before_bus_master(){
 // 0: the chip ignores the unlock, so the ring registers keep their old values.
 for which in 0..2{let m=Mock::new(c::Model::Rtl8168);let mac=prepared(&m);let mut d=Driver::new(m.clone(),mac,100);
  for n in 100..200{if d.phase()==Phase::Registers{if which==0{m.0.borrow_mut().stuck_lock=true;}else{m.0.borrow_mut().absent=true;}}d.advance(n);}
  assert_eq!(d.failure(),Some(Failure::Registers));assert_eq!(m.0.borrow().command&c::BUS_MASTER,0);if which==0{assert!(m.0.borrow().locked_writes>0);}}
}
#[test]fn receive_strips_fcs_copies_before_return_and_keeps_end_of_ring(){
 for model in MODELS{let m=Mock::new(model);let mut d=ready(&m);
  for sequence in 1..=2*ethernet::RX_COUNT as u64{let index=((sequence-1)%ethernet::RX_COUNT as u64)as u32;let payload=vec![sequence as u8;if sequence%2==0{1514}else{60}];
   m.rx(index,&payload,r::FIRST|r::LAST);let start=m.ops().len();
   match d.advance(300+sequence){Some(Event::Received{sequence:s,index:i,frame})=>{assert_eq!((s,i),(sequence,index));assert_eq!(&frame.bytes[..frame.length],&payload[..]);},_=>panic!("missing packet {}",sequence)}
   let ops=&m.ops()[start..];let copied=ops.iter().position(|op|matches!(op,Op::CopyOut(..))).unwrap();let address=ops.iter().position(|op|*op==Op::Store(index*16+8)).unwrap();let own=ops.iter().position(|op|*op==Op::Store(index*16)).unwrap();
   assert!(copied<address&&address<own&&ops[address..own].contains(&Op::Publish));assert!(!ops.iter().any(|op|matches!(op,Op::Write(..))));
   assert_eq!(m.word(index*16),(r::OWN|if index==ethernet::RX_COUNT-1{r::EOR}else{0}|2048)as u64);assert_eq!(m.word(index*16+8),PHYSICAL+8192+index as u64*2048);}
  // Still owned by the chip: nothing to do.
  assert!(d.advance(1000).is_none());}
}
#[test]fn errors_fragments_runts_and_oversize_are_dropped_without_payload_read(){
 for model in MODELS{let m=Mock::new(model);let mut d=ready(&m);
  m.rx(0,&[1;60],r::FIRST|r::LAST|r::RX_ERROR);m.rx(1,&[2;60],r::FIRST);m.rx(2,&[3;60],r::LAST);m.rx(3,&[4;9],r::FIRST|r::LAST);m.rx_raw(4,r::FIRST|r::LAST|3);m.rx(5,&[5;1515],r::FIRST|r::LAST);
  for _ in 0..6{let start=m.ops().len();assert!(matches!(d.advance(300),Some(Event::Dropped{..})));assert!(!m.ops()[start..].iter().any(|op|matches!(op,Op::CopyOut(..))));}
  m.rx(6,&[0x42;60],r::FIRST|r::LAST);assert!(matches!(d.advance(300),Some(Event::Received{index:6,..})));assert_eq!(d.dropped,6);}
}
#[test]fn moved_receive_address_quarantines_without_reading_or_returning(){
 let m=Mock::new(c::Model::Rtl8168);let mut d=ready(&m);m.set(8,0x4000);m.rx(0,&[0;60],r::FIRST|r::LAST);let start=m.ops().len();
 assert!(d.advance(300).is_none());assert_eq!(d.failure(),Some(Failure::Descriptor));assert!(!m.ops()[start..].iter().any(|op|matches!(op,Op::CopyOut(..)|Op::Store(..)|Op::Write(..))));
}
#[test]fn transmit_sets_own_after_address_kicks_the_model_poll_register_and_completes_on_own_clear(){
 for model in MODELS{let m=Mock::new(model);let mut d=ready(&m);
  for sequence in 1..=20u64{let payload=vec![sequence as u8;if sequence%2==0{1514}else{14}];let start=m.ops().len();let (seq,index,length,hash)=d.send(&payload,300+sequence).unwrap();
   let padded=payload.iter().copied().chain(std::iter::repeat(0)).take(payload.len().max(60)).collect::<Vec<_>>();assert_eq!((seq,length,hash),(sequence,padded.len(),ethernet::fingerprint(&padded)));
   let begin=TX_DATA+index as usize*2048;assert_eq!(&m.0.borrow().dma[begin..begin+length],&padded[..]);
   assert_eq!(m.word(4096+index*16),(r::OWN|r::FIRST|r::LAST|if index==7{r::EOR}else{0})as u64|length as u64);assert_eq!(m.word(4096+index*16+8),PHYSICAL+(begin as u64));
   let ops=&m.ops()[start..];let address=ops.iter().position(|op|*op==Op::Store(4096+index*16+8)).unwrap();let own=ops.iter().position(|op|*op==Op::Store(4096+index*16)).unwrap();
   let kick=ops.iter().position(|op|matches!(op,Op::Write(..))).unwrap();assert!(address<own&&ops[address..own].contains(&Op::Publish)&&own<kick&&ops[own..kick].contains(&Op::Publish));
   assert_eq!(ops[kick],if model==c::Model::Rtl8125{Op::Write(0x90,2,1)}else{Op::Write(0x38,1,0x40)});
   assert_eq!(d.send(&[0;60],300+sequence),Err(SendError::Busy));assert!(d.advance(300+sequence).is_none());assert!(d.busy());
   m.done(index);assert!(matches!(d.advance(300+sequence),Some(Event::Transmitted{sequence:s,index:i,..})if s==sequence&&i==index));}}
}
#[test]fn moved_transmit_address_timeout_and_backward_clock_keep_custody_quarantined(){
 for which in 0..3{let m=Mock::new(c::Model::Rtl8125);let mut d=ready(&m);d.send(&[0x22;60],300).unwrap();
  match which{0=>{m.set(4096+8,0x4000);m.done(0);d.advance(301);assert_eq!(d.failure(),Some(Failure::Completion));},
   1=>{d.advance(2300);assert_eq!(d.failure(),Some(Failure::Deadline));},_=>{d.advance(299);assert_eq!(d.failure(),Some(Failure::Clock));}}
  let ops=m.ops();m.done(0);for n in 2300..2310{d.advance(n);}assert_eq!(d.send(&[0;60],2310),Err(SendError::Quarantined));assert_eq!(m.ops(),ops);}
}
