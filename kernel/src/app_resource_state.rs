// SPDX-License-Identifier: GPL-3.0-only
//! Pure authority and exact private-image leaf retirement mechanism.
pub const MAX_APPS:usize=sex_pdx::app_catalog::COUNT;
/// Most pages one manifest may list: a program row's largest arena, its image and stack.
pub const MAX_PAGES:usize=(sex_pdx::program_store::WINDOW_BYTES/4096)+64;
pub const QUERY:u8=1;
pub const ACK:u8=2;
pub const RETIRE:u8=4;
pub const REQUIRED:u8=31;
pub const INVALID:u64=1;
pub const RETRY:u64=2;
pub const DENIED:u64=3;
pub const LIVE:u64=5;
/// The run has pages Denim holds: they come back first (the caller tells Denim, and
/// the program sees RETRY).
pub const RECALL:u64=7;
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
    for page in pages{
        // A page Denim holds keeps its exact marker; any other must still be the page lent.
        let leaf=read(page.leaf);
        let exact=if sex_pdx::denim::is_out(page.expected){leaf==page.expected}
            else{page.expected&5==5 && leaf&!0x60==page.expected&!0x60};
        if page.va==0 || page.va%4096!=0 || page.leaf==0 || !exact {return Err(DENIED)}
    }
    if !distinct(pages){return Err(DENIED)}
    for page in pages {clear(page.leaf);flush(page.va);}
    Ok(pages.len())
}

/// No two pages share an address, a leaf or a frame. Sorting makes this O(n log n), so
/// a program with a big heap ends as fast as a small one.
pub fn distinct(pages:&[PrivatePage])->bool{
    let unique=|mut keys:alloc::vec::Vec<u64>|{keys.sort_unstable();keys.windows(2).all(|w|w[0]!=w[1])};
    unique(pages.iter().map(|p|p.va).collect())
        && unique(pages.iter().map(|p|p.leaf).collect())
        && unique(pages.iter().filter(|p|!sex_pdx::denim::is_out(p.expected)).map(|p|p.expected&PHYSICAL_MASK).collect())
}
/// A successor restores only BOOT-reserved, non-present exact leaves. All
/// checks precede publication; backing never comes from a retired workspace.
pub fn restore_reserved_pages(pages:&[PrivatePage],mut read:impl FnMut(u64)->u64,
    mut write:impl FnMut(u64,u64),mut flush:impl FnMut(u64))->Result<usize,u64>{
    if pages.is_empty() || pages.len()>MAX_PAGES{return Err(INVALID)}
    for p in pages{
        if p.va==0 || p.va%4096!=0 || p.leaf==0 || p.expected&5!=5 || read(p.leaf)!=0{return Err(DENIED)}
    }
    if !distinct(pages){return Err(DENIED)}
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
/// Exact immutable BOOT-owned bytes. Never names a user-provided buffer.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct ImageSource {address:usize,length:usize}
impl ImageSource {
    /// Caller owns these private, immutable bytes for the entire boot lifetime.
    pub(crate) unsafe fn boot_private(address:usize,length:usize)->Self{Self{address,length}}
    pub fn matches(self,image:&[u8])->bool{
        if self.address==0 || self.length==0 || image.len()!=self.length{return false}
        // Only BOOT constructs this descriptor, from retained kernel-private backing.
        unsafe{core::slice::from_raw_parts(self.address as *const u8,self.length)==image}
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct ActivationGrant {
    pub image_source:Option<ImageSource>,
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
/// Denim takes a heap page: its leaf, if it still holds the page lent (accessed and
/// dirty aside) and that is writable data on the row's key, becomes the row's marker.
/// `swap(old,new)` is the leaf's compare-exchange. Returns the frame, which the caller
/// may read only after every core dropped the translation.
pub fn take_out(page:&mut PrivatePage,target:u32,pkey:u8,leaf:u64,swap:impl FnOnce(u64,u64)->bool)->Result<u64,u64>{
    use sex_pdx::denim as d;
    if d::is_out(page.expected) || !d::heap_data(leaf,pkey) || leaf&!0x60!=page.expected&!0x60{return Err(DENIED)}
    let marker=d::out_marker(target);
    if !swap(leaf,marker){return Err(RETRY)}
    let frame=page.expected&PHYSICAL_MASK;page.expected=marker;
    Ok(frame)
}
/// Denim brings a page back into `frame` (already holding its bytes): only where the
/// row's marker still is. Returns the leaf to write.
pub fn bring_in(page:&mut PrivatePage,target:u32,pkey:u8,leaf:u64,frame:u64)->Result<u64,u64>{
    use sex_pdx::denim as d;
    let marker=d::out_marker(target);
    if page.expected!=marker || leaf!=marker || frame&!PHYSICAL_MASK!=0 || frame==0{return Err(DENIED)}
    page.expected=frame|7|((pkey as u64)<<59)|(1<<63);
    Ok(page.expected)
}
