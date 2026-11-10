// SPDX-License-Identifier: GPL-3.0-only
//! The link frames go out on: a PCI network card this service drives itself
//! (`ethernet::Driver`), or a USB network adapter SexUSB drives, whose frames come and go
//! through the shared rings of `sex_pdx::usb_net`.
use crate::ethernet::{self,Event,Failure,Frame,Phase,SendError};
use sex_pdx::usb_net::Shared;

/// A USB adapter, seen from here: two rings and the adapter's address.
pub struct Usb{shared:Shared,generation:u32,phase:Phase,pub mac:[u8;6],received:u64,sent:u64,dropped:u64}
impl Usb{
    /// The region BOOT shared with SexUSB, while an adapter is plugged in.
    pub fn open()->Option<Usb>{
        use sex_pdx::memory_region::{QUERY_VIEW,READ,WRITE};
        let view=sex_pdx::memory_region_full_view(sex_pdx::usb_net::SLOT,QUERY_VIEW|READ|WRITE).ok()?;
        if view.region_id!=sex_pdx::usb_net::REGION_ID||view.length<sex_pdx::usb_net::BYTES||view.virtual_address==0{return None;}
        let shared=unsafe{Shared::at(view.virtual_address as *mut u8)};
        let (mac,generation)=shared.link()?;
        Some(Usb{shared,generation,phase:Phase::Check,mac,received:0,sent:0,dropped:0})
    }
    /// The same adapter is still plugged in.
    fn present(&self)->bool{self.shared.link().is_some_and(|(_,g)|g==self.generation)}
    fn advance(&mut self,receive:bool)->Option<Event>{
        if !self.present(){self.phase=Phase::Quarantined;return None;}
        if self.phase==Phase::Check{self.phase=Phase::Running;return None;}
        self.dropped=self.shared.rx_dropped() as u64;
        if !receive{return None;}
        let mut frame=Frame{length:0,bytes:[0;ethernet::MAX_FRAME]};
        let length=self.shared.rx_pop(&mut frame.bytes)?;
        frame.length=length;self.received+=1;
        Some(Event::Received{sequence:self.received,index:0,frame})
    }
    fn send(&mut self,packet:&[u8])->Result<(u64,u32,usize,u64),SendError>{
        if self.phase!=Phase::Running{return Err(SendError::NotReady);}
        if packet.is_empty()||packet.len()>ethernet::MAX_FRAME{return Err(SendError::Length);}
        // Ethernet's 60-byte minimum: the adapter may not pad short frames itself.
        let mut padded=[0u8;60];
        let frame=if packet.len()<60{padded[..packet.len()].copy_from_slice(packet);&padded[..]}else{packet};
        if !self.shared.tx_push(frame){return Err(SendError::Busy);}
        self.sent+=1;
        Ok((self.sent,0,frame.len(),ethernet::fingerprint(frame)))
    }
}

#[derive(Clone,Copy,Debug)]
pub enum Kind{Pci(sex_pdx::pci_command::Model),Usb}

pub enum Link<H:ethernet::Hardware>{Pci(ethernet::Driver<H>),Usb(Usb)}
impl<H:ethernet::Hardware> Link<H>{
    pub fn phase(&self)->Phase{match self{Link::Pci(d)=>d.phase(),Link::Usb(u)=>u.phase}}
    pub fn failure(&self)->Option<Failure>{match self{Link::Pci(d)=>d.failure(),Link::Usb(_)=>None}}
    pub fn model(&self)->Kind{match self{Link::Pci(d)=>Kind::Pci(d.model()),Link::Usb(_)=>Kind::Usb}}
    pub fn active(&self)->bool{self.phase()!=Phase::Quarantined}
    pub fn busy(&self)->bool{match self{Link::Pci(d)=>d.busy(),Link::Usb(u)=>u.shared.tx_space()==0}}
    pub fn mac(&self)->[u8;6]{match self{Link::Pci(d)=>d.mac,Link::Usb(u)=>u.mac}}
    pub fn dropped(&self)->u64{match self{Link::Pci(d)=>d.dropped,Link::Usb(u)=>u.dropped}}
    pub fn timing_verified(&self)->bool{match self{Link::Pci(d)=>d.timing_verified,Link::Usb(_)=>true}}
    pub fn advance_with_receive(&mut self,now:u64,receive:bool)->Option<Event>{
        match self{Link::Pci(d)=>d.advance_with_receive(now,receive),Link::Usb(u)=>u.advance(receive)}
    }
    pub fn send(&mut self,packet:&[u8],now:u64)->Result<(u64,u32,usize,u64),SendError>{
        match self{Link::Pci(d)=>d.send(packet,now),Link::Usb(u)=>u.send(packet)}
    }
}
