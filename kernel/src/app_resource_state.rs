// SPDX-License-Identifier: GPL-3.0-only
//! Pure authority and exact private-image leaf retirement mechanism.
pub const MAX_APPS:usize=5;
pub const MAX_PAGES:usize=512;
pub const QUERY:u8=1;
pub const ACK:u8=2;
pub const RETIRE:u8=4;
pub const REQUIRED:u8=31;
pub const INVALID:u64=1;
pub const RETRY:u64=2;
pub const DENIED:u64=3;
pub const LIVE:u64=5;
pub const PHYSICAL_MASK:u64=0x000f_ffff_ffff_f000;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct LifecycleGrant {pub target_pd:u32,pub task_id:u32,pub holder_pd:u32,pub rights:u8,pub ack_bit:u8}
impl LifecycleGrant {
    pub fn authorize(self,holder:u32,target:u32,task:u32,right:u8)->Result<(),u64>{
        if self.holder_pd!=holder || self.target_pd!=target || self.task_id!=task
            || self.rights&right!=right {return Err(DENIED)}
        if right==ACK && (self.ack_bit==0 || self.ack_bit&!REQUIRED!=0 || !self.ack_bit.is_power_of_two()){return Err(DENIED)}
        Ok(())
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct PrivatePage {pub va:u64,pub leaf:u64,pub expected:u64}
impl PrivatePage {pub const EMPTY:Self=Self{va:0,leaf:0,expected:0};}
pub fn final_elf_flags(flags:u64,pkey:u8)->u64 {
    assert!(pkey<16);(flags&!(15u64<<59))|((pkey as u64)<<59)
}
/// All fallible checks precede the first mutation. No page can be handed off
/// before every clear and local invalidation has completed. Only A/D may vary.
pub fn retire_private_pages(pages:&[PrivatePage],mut read:impl FnMut(u64)->u64,
    mut clear:impl FnMut(u64),mut flush:impl FnMut(u64))->Result<usize,u64>{
    if pages.is_empty() || pages.len()>MAX_PAGES{return Err(INVALID)}
    for (index,page) in pages.iter().enumerate(){
        if page.va==0 || page.va%4096!=0 || page.leaf==0 || page.expected&5!=5
            || read(page.leaf)&!0x60 != page.expected&!0x60 {return Err(DENIED)}
        for old in &pages[..index]{
            if old.va==page.va || old.leaf==page.leaf
                || old.expected&PHYSICAL_MASK==page.expected&PHYSICAL_MASK{return Err(DENIED)}
        }
    }
    for page in pages {clear(page.leaf);flush(page.va);}
    Ok(pages.len())
}

/// A successor restores only BOOT-reserved, non-present exact leaves. All
/// checks precede publication; backing never comes from a retired workspace.
pub fn restore_reserved_pages(pages:&[PrivatePage],mut read:impl FnMut(u64)->u64,
    mut write:impl FnMut(u64,u64),mut flush:impl FnMut(u64))->Result<usize,u64>{
    if pages.is_empty() || pages.len()>MAX_PAGES{return Err(INVALID)}
    for (i,p) in pages.iter().enumerate(){
        if p.va==0 || p.va%4096!=0 || p.leaf==0 || p.expected&5!=5 || read(p.leaf)!=0
            || pages[..i].iter().any(|old|old.va==p.va || old.leaf==p.leaf
                || old.expected&PHYSICAL_MASK==p.expected&PHYSICAL_MASK){return Err(DENIED)}
    }
    for p in pages{write(p.leaf,p.expected);flush(p.va);}
    Ok(pages.len())
}
/// Identity lineage fixed by BOOT before the graph seal. The ordinal never
/// grants access by itself; activation also checks both exact subjects/tasks.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Incarnation { pub logical_app:u32, pub number:u32 }
impl Incarnation {
    pub const EMPTY:Self=Self{logical_app:0,number:0};
    pub fn original(target:u32)->Self { Self{logical_app:target,number:1} }
    pub fn next(self)->Option<Self> {
        if self.logical_app==0 || self.number==0{return None;}
        Some(Self{logical_app:self.logical_app,number:self.number.checked_add(1)?})
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct ActivationGrant {
    pub holder:u32,pub logical_app:u32,pub incarnation:u32,
    pub predecessor:u32,pub predecessor_task:u32,pub successor:u32,pub successor_task:u32,
}
impl ActivationGrant {
    pub fn authorize(self,holder:u32,predecessor:u32,predecessor_task:u32,successor:u32,successor_task:u32,old:Incarnation,new:Incarnation)->Result<(),u64>{
        if self.holder!=holder || holder==0 || holder==predecessor || holder==successor
            || self.logical_app==0 || old.logical_app!=self.logical_app || new.logical_app!=self.logical_app
            || old.next()!=Some(new) || self.incarnation!=new.number
            || (old.number==1)!=(predecessor==self.logical_app) || successor==self.logical_app
            || predecessor==0 || successor==0 || predecessor==successor
            || predecessor_task==0 || successor_task==0 || predecessor_task==successor_task
            || self.predecessor!=predecessor || self.predecessor_task!=predecessor_task
            || self.successor!=successor || self.successor_task!=successor_task{return Err(DENIED)}
        Ok(())
    }
}
