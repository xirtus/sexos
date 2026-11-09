// SPDX-License-Identifier: GPL-3.0-only
//! Single-owner fixed Ethernet queues inside the existing BOOT DMA reservation.
//! No app-supplied addresses, mappings, runtime grants or implicit DMA reuse.
//! One layout and one set of phases for every family: legacy descriptors
//! (e1000/e1000e), advanced descriptors with queue-0 registers at 0xC000/0xE000
//! (igb/igc), and Realtek descriptors through crate::realtek.
//!
//! I219 (PCH e1000e) takes the legacy path. Bare-metal testing must check:
//! ULP (Ultra Low Power) left on by firmware is not disabled here, so a PHY
//! that stays in ULP never reports link; the ME/AMT SMBus ownership handover
//! (FWSM, H2ME) is not done beyond CTRL_EXT.DRV_LOAD; IOSFPC/FEXTNVM tweaks
//! for pch_spt+ are not written; and the TIPG value is Linux's copper default.
use sex_pdx::{dma_pool::DmaRegionPool, memory_region as m, pci_command as c};
use crate::nic;
use crate::realtek as rtl;
use core::sync::atomic::{fence,Ordering};
/// Receive descriptors: enough to hold a 64 KiB TCP window (about 45 frames) twice
/// over while the service waits its turn, so a download doesn't overrun the ring.
pub const RX_COUNT:u32=96;
pub const TX_COUNT:u32=8;
pub const BUFFER:u32=2048;
pub const MAX_FRAME:usize=1514;
pub const USED:u32=221184;
const RDBAL:u32=0x2800;const RDBAH:u32=0x2804;const RDLEN:u32=0x2808;
const RDH:u32=0x2810;const RDT:u32=0x2818;const RXDCTL:u32=0x2828;
const TDBAL:u32=0x3800;const TDBAH:u32=0x3804;const TDLEN:u32=0x3808;
const TDH:u32=0x3810;const TDT:u32=0x3818;const TXDCTL:u32=0x3828;
const TIPG:u32=0x410;const GCR:u32=0x5b00;
/// igb/igc queue 0 (advanced descriptors).
const ARDBAL:u32=0xC000;const ARDBAH:u32=0xC004;const ARDLEN:u32=0xC008;const SRRCTL:u32=0xC00C;
const ARDH:u32=0xC010;const ARDT:u32=0xC018;const ARXDCTL:u32=0xC028;
const ATDBAL:u32=0xE000;const ATDBAH:u32=0xE004;const ATDLEN:u32=0xE008;
const ATDH:u32=0xE010;const ATDT:u32=0xE018;const ATXDCTL:u32=0xE028;
const QUEUE_ENABLE:u32=1<<25;
/// 2 KiB packet buffers (BSIZEPACKET in KiB) and advanced one-buffer descriptors.
const SRRCTL_VALUE:u32=(BUFFER/1024)|(1<<25);
const CTRL_EXT:u32=0x18;
/// The driver owns the port, not firmware or the management engine.
const DRV_LOAD:u32=1<<28;
/// I219 (ich8lan): TARC0 bits Linux sets in initialize_hw_bits, and RFCTL's
/// NFS-filter disables that work around descriptor corruption.
const TARC0:u32=0x3840;const TARC0_BITS:u32=(1<<23)|(1<<24)|(1<<26)|(1<<27)|(1<<28)|(1<<29);
const RFCTL:u32=0x5008;const RFCTL_NFS_DISABLE:u32=(1<<6)|(1<<7);
/// igb/igc: a MAC reset drops link and the internal PHY (MDIO address 1) must
/// renegotiate; QEMU's igb only does so on a BMCR auto-negotiation restart, as
/// Linux's igb_copper_link_autoneg issues. One MDIC write: PHY 1, register 0
/// (BMCR) = AN enable | AN restart | full duplex | 1000 Mb/s select bits.
const MDIC:u32=0x20;const MDIC_AUTONEG_RESTART:u32=0x1340|(1<<21)|(1<<26);
/// Advanced TX data descriptor: DTYP data, DCMD EOP|IFCS|RS|DEXT.
const ADV_TX_COMMAND:u64=(0x3<<20)|((0x01|0x02|0x08|0x20)<<24);
const RX_DATA:u32=8192;const TX_DATA:u32=RX_DATA+RX_COUNT*BUFFER;
const RCTL_VALUE:u32=2|(1<<15)|(1<<26);
const TCTL_VALUE:u32=2|8|(15<<4)|(64<<12);
const WAIT_TICKS:u64=5000;const WAIT_TURNS:u32=32768;
const TX_TICKS:u64=2000;const TX_TURNS:u32=8192;

pub trait Hardware:nic::Hardware {
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
/// Queues: igb/igc only, waiting for RXDCTL/TXDCTL to read back enabled.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Phase{Check,Zero,Descriptors,Registers,Enable,Queues,Link,Running,Quarantined}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Failure{Authority,Command,Registers,Deadline,Clock,Descriptor,Completion,Sequence}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum SendError{NotReady,Busy,Length,Quarantined}
#[derive(Clone,Copy)]struct Layout{physical:u64}
impl Layout{
    fn new(view:m::RegionView)->Option<Self>{
        if view.region_id!=c::DMA_REGION_ID||view.length!=c::DMA_BYTES{return None;}
        // Use the actual common subdivision arithmetic, without extending it.
        let mut pool=DmaRegionPool::new(view).ok()?;
        let rx=pool.reserve(4096).ok()?;let tx=pool.reserve(4096).ok()?;
        let rxb=pool.reserve((RX_COUNT*BUFFER) as u64).ok()?;let txb=pool.reserve((TX_COUNT*BUFFER) as u64).ok()?;
        let p=rx.physical_address;let v=rx.virtual_address;
        if tx.physical_address!=p+4096||rxb.physical_address!=p+RX_DATA as u64||txb.physical_address!=p+TX_DATA as u64
            ||tx.virtual_address!=v+4096||rxb.virtual_address!=v+RX_DATA as u64||txb.virtual_address!=v+TX_DATA as u64||pool.used()!=USED as u64{return None;}
        Some(Self{physical:p})
    }
    fn rx_ring(&self)->u64{self.physical}
    fn tx_ring(&self)->u64{self.physical+4096}
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
    fn family(&self)->c::Family{self.model().family()}
    fn realtek(&self)->bool{self.family()==c::Family::Realtek}
    fn advanced(&self)->bool{self.model().advanced()}
    fn rdt(&self)->u32{if self.advanced(){ARDT}else{RDT}}
    fn tdt(&self)->u32{if self.advanced(){ATDT}else{TDT}}
    /// The MMIO prefix this family's registers need.
    fn registers_end(&self)->u64{
        match self.family(){
            c::Family::Realtek=>rtl::REGISTERS_END,
            c::Family::Igb|c::Family::Igc=>self.model().registers_end(),
            _=>(if self.model()==c::Model::I82574{GCR+4}else{nic::RAH+4})as u64,
        }
    }
    /// Values written in Registers and read back in Enable. Advanced queues are
    /// disabled first and their tails stay 0 until Queues sees them enabled.
    /// igc has no TIPG; igb keeps its reset default (Linux writes neither).
    fn registers(&self)->[(u32,u32);13]{let p=self.layout.unwrap().physical;
        if self.advanced(){return [(ARXDCTL,0),(ATXDCTL,0),
            (ARDBAL,p as u32),(ARDBAH,(p>>32)as u32),(ARDLEN,RX_COUNT*16),(SRRCTL,SRRCTL_VALUE),(ARDH,0),(ARDT,0),
            (ATDBAL,(p+4096)as u32),(ATDBAH,((p+4096)>>32)as u32),(ATDLEN,TX_COUNT*16),(ATDH,0),(ATDT,0)];}
        let legacy_e1000e=matches!(self.model(),c::Model::I82574|c::Model::I219);
        [(RDBAL,p as u32),(RDBAH,(p>>32)as u32),(RDLEN,RX_COUNT*16),(RDH,0),(RDT,RX_COUNT-1),
         (TDBAL,(p+4096)as u32),(TDBAH,((p+4096)>>32)as u32),(TDLEN,TX_COUNT*16),(TDH,0),(TDT,0),
         // TXDCTL bit 22 must be set on 82574 and ich8lan parts (I219).
         (RXDCTL,0),(TXDCTL,if legacy_e1000e{1<<22}else{0}),
         (TIPG,match self.model(){c::Model::I82574=>0x00602006,c::Model::I219=>0x00602008,_=>0x0060200a})]}
    fn read_modify(&mut self,offset:u32,set:u32)->bool{
        let value=self.hardware.read(offset);if value==u32::MAX{return false;}
        self.hardware.write(offset,value|set);true
    }
    /// One bounded initialization phase, or at most one TX/RX completion.
    /// The service always handles IPC and yields between calls.
    pub fn advance(&mut self,now:u64)->Option<Event>{
        self.advance_with_receive(now,true)
    }
    /// Receive custody stays with hardware while the private protocol slot is
    /// full. TX completion and timeout processing must still make progress.
    pub fn advance_with_receive(&mut self,now:u64,receive:bool)->Option<Event>{
        if !self.active(){return None;}
        if now<self.last{self.fail(Failure::Clock);return None;}self.last=now;
        if self.phase!=Phase::Running{
            self.turns+=1;if now-self.started>=WAIT_TICKS||self.turns>=WAIT_TURNS{self.fail(Failure::Deadline);return None;}
        }
        match self.phase{
            Phase::Check=>{
                if self.hardware.mmio_length()<self.registers_end()
                    ||self.mac==[0;6]||self.mac[0]&1!=0{self.fail(Failure::Authority);return None;}
                let Ok(view)=self.hardware.dma_region()else{self.fail(Failure::Authority);return None;};
                let Some(layout)=Layout::new(view)else{self.fail(Failure::Authority);return None;};
                let Ok(r)=self.hardware.command(0,0)else{self.fail(Failure::Command);return None;};
                if r.before!=r.after||r.changed||r.after&c::NIC_BITS!=c::MEMORY{self.fail(Failure::Command);return None;}
                if self.realtek(){if !rtl::idle(&mut self.hardware){self.fail(Failure::Registers);return None;}}
                else if self.hardware.read(nic::RCTL)&2!=0||self.hardware.read(nic::TCTL)&2!=0||self.hardware.read(nic::IMS)!=0{self.fail(Failure::Registers);return None;}
                self.layout=Some(layout);self.phase=Phase::Zero;
            },
            Phase::Zero=>{
                // At most one existing 4 KiB page per turn (USED/4096 turns).
                for offset in (self.zero..self.zero+4096).step_by(8){self.hardware.store64(offset,0);}
                self.zero+=4096;if self.zero==USED{self.phase=Phase::Descriptors;}
            },
            Phase::Descriptors if self.realtek()=>{
                // Address first, then opts1: OWN hands a descriptor to the chip.
                let l=self.layout.unwrap();
                for i in 0..RX_COUNT{self.hardware.store64(i*16+8,l.rx(i));self.hardware.store64(i*16,rtl::rx_ready(i,RX_COUNT,BUFFER));}
                for i in 0..TX_COUNT{self.hardware.store64(4096+i*16+8,l.tx(i));self.hardware.store64(4096+i*16,rtl::tx_idle(i,TX_COUNT));}
                self.hardware.publish();self.phase=Phase::Registers;
            },
            Phase::Descriptors=>{
                // Advanced read format is the legacy layout: buffer, then zero
                // (no header buffer); TX keeps DD set while the driver owns it.
                let l=self.layout.unwrap();
                for i in 0..RX_COUNT{self.hardware.store64(i*16,l.rx(i));self.hardware.store64(i*16+8,0);}
                for i in 0..TX_COUNT{self.hardware.store64(4096+i*16,l.tx(i));self.hardware.store64(4096+i*16+8,1<<32);}self.hardware.publish();self.phase=Phase::Registers;
            },
            Phase::Registers if self.realtek()=>{
                let l=self.layout.unwrap();
                if !rtl::configure(&mut self.hardware,l.rx_ring(),l.tx_ring(),BUFFER as u16){self.fail(Failure::Registers);return None;}
                self.phase=Phase::Enable;
            },
            Phase::Registers=>{
                for (offset,value)in self.registers(){self.hardware.write(offset,value);}
                // No multicast acceptance, checksum offload or second queue.
                for n in 0..128{self.hardware.write(0x5200+4*n,0);}
                self.hardware.write(0x5000,0);
                let ctrl=self.hardware.read(nic::CTRL);if ctrl==u32::MAX||ctrl&(1<<26)!=0{self.fail(Failure::Registers);return None;}
                self.hardware.write(nic::CTRL,ctrl|(1<<6));
                if self.model()==c::Model::I82574{let gcr=self.hardware.read(GCR);if gcr==u32::MAX{self.fail(Failure::Registers);return None;}self.hardware.write(GCR,gcr|(1<<22));}
                if self.model()==c::Model::I219&&!(self.read_modify(TARC0,TARC0_BITS)&&self.read_modify(RFCTL,RFCTL_NFS_DISABLE)){self.fail(Failure::Registers);return None;}
                if self.family()!=c::Family::E1000{
                    // CTRL_EXT bit 22 is another ich8lan must-set bit.
                    let set=DRV_LOAD|if self.model()==c::Model::I219{1<<22}else{0};
                    if !self.read_modify(CTRL_EXT,set){self.fail(Failure::Registers);return None;}
                }
                if self.advanced(){
                    self.hardware.write(nic::EIMC,u32::MAX);
                    // The PHY finishes this on its own; Link waits for STATUS.LU
                    // within the initialization deadline.
                    self.hardware.write(MDIC,MDIC_AUTONEG_RESTART);
                }
                self.phase=Phase::Enable;
            },
            Phase::Enable if self.realtek()=>{
                let l=self.layout.unwrap();
                if !rtl::configured(&mut self.hardware,l.rx_ring(),l.tx_ring(),BUFFER as u16){self.fail(Failure::Registers);return None;}
                self.hardware.publish();
                let Ok(r)=self.hardware.command(c::BUS_MASTER,0)else{self.fail(Failure::Command);return None;};
                if r.before&c::NIC_BITS!=c::MEMORY||r.after!=r.before|c::BUS_MASTER||!r.changed{self.fail(Failure::Command);return None;}
                if !rtl::start(&mut self.hardware){self.fail(Failure::Registers);return None;}
                self.phase=Phase::Link;
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
                if self.advanced(){
                    // As Linux igb: RCTL/TCTL enabled, then each queue.
                    self.hardware.write(ARXDCTL,QUEUE_ENABLE);self.hardware.write(ATXDCTL,QUEUE_ENABLE);
                    self.phase=Phase::Queues;return None;
                }
                self.phase=Phase::Link;
            },
            Phase::Queues=>{
                let rx=self.hardware.read(ARXDCTL);let tx=self.hardware.read(ATXDCTL);
                if rx==u32::MAX||tx==u32::MAX{self.fail(Failure::Registers);return None;}
                // Bounded by the initialization deadline; tails move only once
                // the queues are enabled.
                if rx&QUEUE_ENABLE==0||tx&QUEUE_ENABLE==0{return None;}
                self.hardware.write(ARDT,RX_COUNT-1);
                if self.hardware.read(ARDT)!=RX_COUNT-1{self.fail(Failure::Registers);return None;}
                self.phase=Phase::Link;
            },
            Phase::Link if self.realtek()=>{match rtl::link(&mut self.hardware){None=>self.fail(Failure::Registers),Some(true)=>self.phase=Phase::Running,Some(false)=>{}}},
            Phase::Link=>{let status=self.hardware.read(nic::STATUS);if status==u32::MAX{self.fail(Failure::Registers);}else if status&2!=0{self.phase=Phase::Running;}},
            Phase::Running=>{
                if let Some(mut pending)=self.pending{
                    pending.turns+=1;self.pending=Some(pending);
                    if let Some(word)=self.tx_done(pending.index){
                        if self.tx_completion_damaged(pending.index,word){self.fail(Failure::Completion);return None;}
                        // Only this exact validated DD completion permits reuse.
                        self.pending=None;return Some(Event::Transmitted{sequence:pending.sequence,index:pending.index,length:pending.length,hash:pending.hash});
                    }
                    if now-pending.since>=TX_TICKS||pending.turns>=TX_TURNS{self.fail(Failure::Deadline);return None;}
                }
                if !receive{return None;}
                let index=self.rx;
                let Some(Received{length,end,whole,status,errors})=self.rx_status(index)else{return None;};
                let accept=!self.discard&&whole&&(14..=MAX_FRAME).contains(&length);
                self.discard=!end;
                let mut frame=Frame{length:0,bytes:[0;MAX_FRAME]};
                if accept{frame.length=length;self.hardware.load_bytes(RX_DATA+index*BUFFER,&mut frame.bytes[..length]);}
                self.rx_return(index);self.rx=(index+1)%RX_COUNT;
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
        let address=self.layout.unwrap().tx(index);
        if self.realtek(){
            self.hardware.store64(4096+index*16+8,address);self.hardware.publish();
            self.hardware.store64(4096+index*16,rtl::tx_submit(index,TX_COUNT,length));
        }else{
            self.hardware.store64(4096+index*16,address);
            // Legacy: length, CMD EOP|IFCS|RS. Advanced: DTYP data, DCMD with
            // DEXT, and PAYLEN (the whole frame, no offload).
            self.hardware.store64(4096+index*16+8,if self.advanced(){length as u64|ADV_TX_COMMAND|((length as u64)<<46)}else{length as u64|(0x0b<<24)});
        }
        self.pending=Some(Pending{sequence,index,length,hash,since:now,turns:0});self.sent=sequence;
        self.hardware.publish();self.tx=(index+1)%TX_COUNT;
        if self.realtek(){rtl::kick(&mut self.hardware);}else{let tail=self.tdt();self.hardware.write(tail,self.tx);}
        Ok((sequence,index,length,hash))
    }
    /// The word showing the chip finished the pending descriptor: DD (Intel)
    /// or OWN cleared (Realtek).
    fn tx_done(&mut self,index:u32)->Option<u64>{
        if self.realtek(){let word=self.hardware.load64(4096+index*16);return if word as u32&rtl::OWN==0{Some(word)}else{None};}
        let word=self.hardware.load64(4096+index*16+8);
        if (word>>32)&1!=0{Some(word)}else{None}
    }
    /// After acquire: anything but an exact completion keeps custody quarantined.
    /// Advanced write-back overwrites the address and length words, so only DD
    /// (bit 32) is meaningful there; legacy and Realtek leave the address.
    fn tx_completion_damaged(&mut self,index:u32,word:u64)->bool{
        self.hardware.acquire();let address=self.layout.unwrap().tx(index);
        if self.realtek(){return self.hardware.load64(4096+index*16+8)!=address;}
        if self.advanced(){return false;}
        let status=(word>>32)as u8;
        status&0xfe!=0||self.hardware.load64(4096+index*16)!=address
    }
    /// The finished receive descriptor at index, if the chip handed it back.
    fn rx_status(&mut self,index:u32)->Option<Received>{
        let address=self.layout.unwrap().rx(index);
        if self.realtek(){
            let opts1=self.hardware.load64(index*16)as u32;if opts1&rtl::OWN!=0{return None;}self.hardware.acquire();
            if self.hardware.load64(index*16+8)!=address{self.fail(Failure::Descriptor);return None;}
            let (first,last,error)=(opts1&rtl::FIRST!=0,opts1&rtl::LAST!=0,opts1&rtl::RX_ERROR!=0);
            // The length counts the frame check sequence; a shorter one is malformed.
            let raw=(opts1&rtl::LENGTH)as usize;let length=raw.saturating_sub(rtl::CRC);
            return Some(Received{length:if raw<rtl::CRC{0}else{length},end:last,whole:first&&last&&!error&&raw>=rtl::CRC,status:(opts1>>24)as u8,errors:((opts1>>16)&0x3f)as u8});
        }
        let word=self.hardware.load64(index*16+8);
        if self.advanced(){
            // Write-back: status/error in the low 32 bits, length in 32..47.
            // The first word now holds RSS/packet type, not our address.
            let staterr=word as u32;if staterr&1==0{return None;}self.hardware.acquire();
            let length=((word>>32)&0xffff)as usize;let errors=staterr>>20;
            return Some(Received{length,end:staterr&2!=0,whole:staterr&2!=0&&errors==0,status:staterr as u8,errors:((errors|(errors>>8))&0xff)as u8});
        }
        let status=(word>>32)as u8;if status&1==0{return None;}self.hardware.acquire();
        if self.hardware.load64(index*16)!=address{self.fail(Failure::Descriptor);return None;}
        let errors=(word>>40)as u8;
        Some(Received{length:(word&0xffff)as usize,end:status&2!=0,whole:status&2!=0&&errors==0,status,errors})
    }
    /// Give a consumed descriptor back to the chip.
    fn rx_return(&mut self,index:u32){
        let address=self.layout.unwrap().rx(index);
        if self.realtek(){
            self.hardware.store64(index*16+8,address);self.hardware.publish();
            self.hardware.store64(index*16,rtl::rx_ready(index,RX_COUNT,BUFFER));self.hardware.publish();return;
        }
        self.hardware.store64(index*16,address);self.hardware.store64(index*16+8,0);
        self.hardware.publish();let tail=self.rdt();self.hardware.write(tail,index);
    }
}
/// A finished receive descriptor. whole: one undamaged frame in one buffer.
struct Received{length:usize,end:bool,whole:bool,status:u8,errors:u8}

/// Diagnostic fingerprint of actual private bytes, not a transport receipt.
pub fn fingerprint(bytes:&[u8])->u64{bytes.iter().fold(0xcbf29ce484222325,|h,b|(h^*b as u64).wrapping_mul(0x100000001b3))}
