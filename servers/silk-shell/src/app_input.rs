// SPDX-License-Identifier: GPL-3.0-only
//! Fixed FIFO retains the exact catalog destination through focus changes.
//! Sending still requires Shell's held Domain route and a fresh live query.
use sex_pdx::{app_key::Key, clipboard::{Action,Participant,Subject}};
pub const CAPACITY:usize=32;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Input { Key(Key), Clipboard(Action) }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Item { pub destination:Participant, pub input:Input }
pub struct Queue { items:[Option<Item>;CAPACITY], head:usize, len:usize }
impl Queue {
    pub const fn new()->Self { Self {items:[None;CAPACITY],head:0,len:0} }
    pub fn pending(&self)->bool { self.len!=0 }
    pub fn push(&mut self,item:Item)->bool {
        if self.len==CAPACITY || !item.destination.subject.valid() || item.destination.route==0 {return false;}
        self.items[(self.head+self.len)%CAPACITY]=Some(item);self.len+=1;true
    }
    pub fn front(&self)->Option<Item>{self.items[self.head]}
    pub fn pop(&mut self){
        if self.len==0{return;}
        unsafe{core::ptr::write_volatile(&mut self.items[self.head],None)};
        self.head=(self.head+1)%CAPACITY;self.len-=1;
    }
    pub fn retire(&mut self,subject:Subject)->usize {
        let original=self.len;
        // Visit exactly the original entries. Survivors retain original order.
        for _ in 0..original {
            let item=self.front().unwrap();self.pop();
            if item.destination.subject!=subject {let retained=self.push(item);debug_assert!(retained);}
        }
        original-self.len
    }
}

/// Admission custody while a read-only kernel liveness query is contended.
/// Overflow fails closed; a sender never receives a fabricated mutation ACK.
pub const INTAKE_DEFER_TURNS:u32=4096;
pub struct Intake { packets:[Option<(Subject,sex_pdx::clipboard::Packet)>;CAPACITY], deferrals:[u32;CAPACITY], head:usize, len:usize }
impl Intake {
    pub const fn new()->Self { Self {packets:[None;CAPACITY],deferrals:[0;CAPACITY],head:0,len:0} }
    pub fn pending(&self)->bool {self.len!=0}
    pub fn push(&mut self,subject:Subject,packet:sex_pdx::clipboard::Packet)->bool {
        if self.len==CAPACITY || !subject.valid() || sex_pdx::clipboard::Request::decode(packet.opcode,packet.args).is_none(){return false}
        let index=(self.head+self.len)%CAPACITY;
        self.packets[index]=Some((subject,packet));self.deferrals[index]=0;self.len+=1;true
    }
    pub fn front(&self)->Option<(Subject,sex_pdx::clipboard::Packet)>{self.packets[self.head]}
    pub fn pop(&mut self){
        if self.len==0{return}
        if let Some((_,packet))=self.packets[self.head].as_mut(){packet.erase();}
        unsafe{core::ptr::write_volatile(&mut self.packets[self.head],None)};
        self.deferrals[self.head]=0;
        self.head=(self.head+1)%CAPACITY;self.len-=1;
    }
    /// A contended read-only LIVE query retains the exact front packet, but
    /// cannot retain private text forever under a frozen clock. Expiry erases
    /// that packet without fabricating an application or mutation receipt.
    pub fn defer(&mut self)->bool {
        if self.len==0{return false}
        self.deferrals[self.head]=self.deferrals[self.head].saturating_add(1);
        if self.deferrals[self.head]>=INTAKE_DEFER_TURNS{self.pop();false}else{true}
    }
    pub fn retire(&mut self,subject:Subject)->usize {
        let original=self.len;
        for _ in 0..original{
            let (owner,mut packet)=self.front().unwrap();let deferrals=self.deferrals[self.head];self.pop();
            if owner!=subject{let retained=self.push(owner,packet);debug_assert!(retained);self.deferrals[(self.head+self.len-1)%CAPACITY]=deferrals;}
            packet.erase();
        }
        original-self.len
    }
}
impl Drop for Intake {fn drop(&mut self){while self.pending(){self.pop();}}}

#[cfg(test)]impl Intake {
    pub(crate) fn storage_erased(&self)->bool {self.packets.iter().all(Option::is_none)&&self.deferrals.iter().all(|&n|n==0)}
}
