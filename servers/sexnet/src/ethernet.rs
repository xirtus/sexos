// SPDX-License-Identifier: GPL-3.0-only
//! Single-owner fixed Ethernet queues inside the existing BOOT DMA reservation.
//! No app-supplied addresses, mappings, runtime grants or implicit DMA reuse.
use sex_pdx::{dma_pool::DmaRegionPool, memory_region as m, pci_command as c};
use crate::nic;
use core::sync::atomic::{fence,Ordering};
pub const COUNT:u32=8;
pub const BUFFER:u32=2048;
pub const MAX_FRAME:usize=1514;
pub const USED:u32=40960;
const RDBAL:u32=0x2800;const RDBAH:u32=0x2804;const RDLEN:u32=0x2808;
const RDH:u32=0x2810;const RDT:u32=0x2818;const RXDCTL:u32=0x2828;
const TDBAL:u32=0x3800;const TDBAH:u32=0x3804;const TDLEN:u32=0x3808;
const TDH:u32=0x3810;const TDT:u32=0x3818;const TXDCTL:u32=0x3828;
const TIPG:u32=0x410;const GCR:u32=0x5b00;
const RX_DATA:u32=8192;const TX_DATA:u32=24576;
const RCTL_VALUE:u32=2|(1<<15)|(1<<26);
const TCTL_VALUE:u32=2|8|(15<<4)|(64<<12);
const WAIT_TICKS:u64=5000;const WAIT_TURNS:u32=32768;
const TX_TICKS:u64=2000;const TX_TURNS:u32=8192;

pub trait Hardware:nic::Hardware {
    fn model(&self)->c::Model;
    fn mmio_length(&self)->u64;
    /// Must authenticate the current holder's pre-existing Region2 first.
    fn dma_region(&mut self)->Result<m::RegionView,u64>;
    fn load64(&mut self,offset:u32)->u64;
    fn store64(&mut self,offset:u32,value:u64);
    fn load_bytes(&mut self,offset:u32,out:&mut[u8]);
    fn store_bytes(&mut self,offset:u32,bytes:&[u8]);
    fn publish(&mut self){fence(Ordering::SeqCst);}
    fn acquire(&mut self){fence(Ordering::Acquire);}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Phase{Check,Zero,Descriptors,Registers,Enable,Link,Running,Quarantined}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Failure{Authority,Command,Registers,Deadline,Clock,Descriptor,Completion,Sequence}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum SendError{NotReady,Busy,Length,Quarantined}
#[derive(Clone,Copy)]struct Layout{physical:u64}
impl Layout{
    fn new(view:m::RegionView)->Option<Self>{
        if view.region_id!=c::DMA_REGION_ID||view.length!=c::DMA_BYTES{return None;}
        // Use the actual common subdivision arithmetic, without extending it.
        let mut pool=DmaRegionPool::new(view).ok()?;
        let rx=pool.reserve(4096).ok()?;let tx=pool.reserve(4096).ok()?;
        let rxb=pool.reserve(16384).ok()?;let txb=pool.reserve(16384).ok()?;
        let p=rx.physical_address;let v=rx.virtual_address;
        if tx.physical_address!=p+4096||rxb.physical_address!=p+RX_DATA as u64||txb.physical_address!=p+TX_DATA as u64
            ||tx.virtual_address!=v+4096||rxb.virtual_address!=v+RX_DATA as u64||txb.virtual_address!=v+TX_DATA as u64||pool.used()!=USED as u64{return None;}
        Some(Self{physical:p})
    }
    fn rx(&self,index:u32)->u64{self.physical+RX_DATA as u64+index as u64*BUFFER as u64}
    fn tx(&self,index:u32)->u64{self.physical+TX_DATA as u64+index as u64*BUFFER as u64}
}
pub struct Frame{pub length:usize,pub bytes:[u8;MAX_FRAME]}
pub enum Event{Received{sequence:u64,index:u32,frame:Frame},Dropped{index:u32,length:usize,status:u8,errors:u8},Transmitted{sequence:u64,index:u32,length:usize,hash:u64}}
#[derive(Clone,Copy)]struct Pending{sequence:u64,index:u32,length:usize,hash:u64,since:u64,turns:u32}
pub struct Driver<H:Hardware>{hardware:H,phase:Phase,failure:Option<Failure>,layout:Option<Layout>,started:u64,last:u64,turns:u32,
    zero:u32,rx:u32,tx:u32,discard:bool,pending:Option<Pending>,pub mac:[u8;6],pub sent:u64,pub received:u64,pub dropped:u64,pub timing_verified:bool}
impl<H:Hardware> Driver<H>{
    pub fn new(hardware:H,mac:[u8;6],now:u64)->Self{Self{hardware,phase:Phase::Check,failure:None,layout:None,started:now,last:now,turns:0,
        zero:0,rx:0,tx:0,discard:false,pending:None,mac,sent:0,received:0,dropped:0,timing_verified:false}}
    pub fn phase(&self)->Phase{self.phase}
    pub fn failure(&self)->Option<Failure>{self.failure}
    pub fn model(&self)->c::Model{self.hardware.model()}
    pub fn active(&self)->bool{self.phase!=Phase::Quarantined}
    pub fn busy(&self)->bool{self.pending.is_some()}
    fn fail(&mut self,reason:Failure){self.failure=Some(reason);self.phase=Phase::Quarantined;}
    fn registers(&self)->[(u32,u32);13]{let p=self.layout.unwrap().physical;
        [(RDBAL,p as u32),(RDBAH,(p>>32)as u32),(RDLEN,COUNT*16),(RDH,0),(RDT,COUNT-1),
         (TDBAL,(p+4096)as u32),(TDBAH,((p+4096)>>32)as u32),(TDLEN,COUNT*16),(TDH,0),(TDT,0),
         (RXDCTL,0),(TXDCTL,if self.model()==c::Model::I82574{1<<22}else{0}),
         (TIPG,if self.model()==c::Model::I82574{0x00602006}else{0x0060200a})]}
    /// One bounded initialization phase, or at most one TX/RX completion.
    /// The service always handles IPC and yields between calls.
    pub fn advance(&mut self,now:u64)->Option<Event>{
        if !self.active(){return None;}
        if now<self.last{self.fail(Failure::Clock);return None;}self.last=now;
        if self.phase!=Phase::Running{
            self.turns+=1;if now-self.started>=WAIT_TICKS||self.turns>=WAIT_TURNS{self.fail(Failure::Deadline);return None;}
        }
        match self.phase{
            Phase::Check=>{
                if self.hardware.mmio_length()<(if self.model()==c::Model::I82574{GCR+4}else{nic::RAH+4})as u64
                    ||self.mac==[0;6]||self.mac[0]&1!=0{self.fail(Failure::Authority);return None;}
                let Ok(view)=self.hardware.dma_region()else{self.fail(Failure::Authority);return None;};
                let Some(layout)=Layout::new(view)else{self.fail(Failure::Authority);return None;};
                let Ok(r)=self.hardware.command(0,0)else{self.fail(Failure::Command);return None;};
                if r.before!=r.after||r.changed||r.after&c::NIC_BITS!=c::MEMORY{self.fail(Failure::Command);return None;}
                if self.hardware.read(nic::RCTL)&2!=0||self.hardware.read(nic::TCTL)&2!=0||self.hardware.read(nic::IMS)!=0{self.fail(Failure::Registers);return None;}
                self.layout=Some(layout);self.phase=Phase::Zero;
            },
            Phase::Zero=>{
                // At most one existing4KiB page per turn, ten turns total.
                for offset in (self.zero..self.zero+4096).step_by(8){self.hardware.store64(offset,0);}
                self.zero+=4096;if self.zero==USED{self.phase=Phase::Descriptors;}
            },
            Phase::Descriptors=>{
                let l=self.layout.unwrap();for i in 0..COUNT{
                    self.hardware.store64(i*16,l.rx(i));self.hardware.store64(i*16+8,0);
                    self.hardware.store64(4096+i*16,l.tx(i));self.hardware.store64(4096+i*16+8,1<<32);
                }self.hardware.publish();self.phase=Phase::Registers;
            },
            Phase::Registers=>{
                for (offset,value)in self.registers(){self.hardware.write(offset,value);}
                // No multicast acceptance, checksum offload or second queue.
                for n in 0..128{self.hardware.write(0x5200+4*n,0);}
                self.hardware.write(0x5000,0);
                let ctrl=self.hardware.read(nic::CTRL);if ctrl==u32::MAX||ctrl&(1<<26)!=0{self.fail(Failure::Registers);return None;}
                self.hardware.write(nic::CTRL,ctrl|(1<<6));
                if self.model()==c::Model::I82574{let gcr=self.hardware.read(GCR);if gcr==u32::MAX{self.fail(Failure::Registers);return None;}self.hardware.write(GCR,gcr|(1<<22));}
                self.phase=Phase::Enable;
            },
            Phase::Enable=>{
                for(offset,value)in self.registers(){
                    let observed=self.hardware.read(offset);
                    // QEMU11.0.2's legacy e1000 omits TIPG from both its read
                    // and write tables. Record that zero readback as unverified
                    // timing, never as a verified configuration. No DMA base,
                    // length, head, tail or descriptor check has an exception.
                    // Physical timing qualification remains a separate gate.
                    if offset==TIPG {
                        self.timing_verified=observed==value;
                        if self.model()==c::Model::I82540&&observed==0{continue;}
                    }
                    if observed!=value{self.fail(Failure::Registers);return None;}
                }
                self.hardware.publish();
                let Ok(r)=self.hardware.command(c::BUS_MASTER,0)else{self.fail(Failure::Command);return None;};
                if r.before&c::NIC_BITS!=c::MEMORY||r.after!=r.before|c::BUS_MASTER||!r.changed{self.fail(Failure::Command);return None;}
                self.hardware.write(nic::TCTL,TCTL_VALUE);self.hardware.write(nic::RCTL,RCTL_VALUE);
                if self.hardware.read(nic::TCTL)&TCTL_VALUE!=TCTL_VALUE||self.hardware.read(nic::RCTL)&RCTL_VALUE!=RCTL_VALUE{self.fail(Failure::Registers);return None;}
                self.phase=Phase::Link;
            },
            Phase::Link=>{let status=self.hardware.read(nic::STATUS);if status==u32::MAX{self.fail(Failure::Registers);}else if status&2!=0{self.phase=Phase::Running;}},
            Phase::Running=>{
                if let Some(mut pending)=self.pending{
                    pending.turns+=1;self.pending=Some(pending);
                    let status=(self.hardware.load64(4096+pending.index*16+8)>>32)as u8;
                    if status&1!=0{
                        self.hardware.acquire();
                        if status&0xfe!=0||self.hardware.load64(4096+pending.index*16)!=self.layout.unwrap().tx(pending.index){self.fail(Failure::Completion);return None;}
                        // Only this exact validated DD completion permits reuse.
                        self.pending=None;return Some(Event::Transmitted{sequence:pending.sequence,index:pending.index,length:pending.length,hash:pending.hash});
                    }
                    if now-pending.since>=TX_TICKS||pending.turns>=TX_TURNS{self.fail(Failure::Deadline);return None;}
                }
                let index=self.rx;let word=self.hardware.load64(index*16+8);let status=(word>>32)as u8;
                if status&1==0{return None;}self.hardware.acquire();
                if self.hardware.load64(index*16)!=self.layout.unwrap().rx(index){self.fail(Failure::Descriptor);return None;}
                let length=(word&0xffff)as usize;let errors=(word>>40)as u8;
                let accept=!self.discard&&status&2!=0&&errors==0&&(14..=MAX_FRAME).contains(&length);
                self.discard=status&2==0;
                let mut frame=Frame{length:0,bytes:[0;MAX_FRAME]};
                if accept{frame.length=length;self.hardware.load_bytes(RX_DATA+index*BUFFER,&mut frame.bytes[..length]);}
                self.hardware.store64(index*16,self.layout.unwrap().rx(index));self.hardware.store64(index*16+8,0);
                self.hardware.publish();self.hardware.write(RDT,index);self.rx=(index+1)%COUNT;
                return if accept{let Some(sequence)=self.received.checked_add(1)else{self.fail(Failure::Sequence);return None;};self.received=sequence;Some(Event::Received{sequence,index,frame})}
                else{self.dropped=self.dropped.saturating_add(1);Some(Event::Dropped{index,length,status,errors})}
            },
            Phase::Quarantined=>{},
        }
        None
    }
    pub fn send(&mut self,packet:&[u8],now:u64)->Result<(u64,u32,usize,u64),SendError>{
        if self.phase==Phase::Quarantined{return Err(SendError::Quarantined);}
        if self.phase!=Phase::Running{return Err(SendError::NotReady);}
        if self.pending.is_some(){return Err(SendError::Busy);}
        if !(14..=MAX_FRAME).contains(&packet.len()){return Err(SendError::Length);}
        if now<self.last{self.fail(Failure::Clock);return Err(SendError::Quarantined);}
        let Some(sequence)=self.sent.checked_add(1)else{self.fail(Failure::Sequence);return Err(SendError::Quarantined);};
        self.last=now;
        let index=self.tx;let length=packet.len().max(60);let data=TX_DATA+index*BUFFER;
        for offset in (data..data+BUFFER).step_by(8){self.hardware.store64(offset,0);}
        self.hardware.store_bytes(data,packet);let mut hash=0xcbf29ce484222325u64;
        for n in 0..length{hash=(hash^if n<packet.len(){packet[n]as u64}else{0}).wrapping_mul(0x100000001b3);}
        self.hardware.store64(4096+index*16,self.layout.unwrap().tx(index));
        self.hardware.store64(4096+index*16+8,length as u64|(0x0b<<24));
        self.pending=Some(Pending{sequence,index,length,hash,since:now,turns:0});self.sent=sequence;
        self.hardware.publish();self.tx=(index+1)%COUNT;self.hardware.write(TDT,self.tx);
        Ok((sequence,index,length,hash))
    }
}

/// Diagnostic fingerprint of actual private bytes, not a transport receipt.
pub fn fingerprint(bytes:&[u8])->u64{bytes.iter().fold(0xcbf29ce484222325,|h,b|(h^*b as u64).wrapping_mul(0x100000001b3))}
