#![allow(dead_code)]
extern crate self as spin;
extern crate self as x86_64;
pub struct Mutex<T>(std::sync::Mutex<T>);
impl<T> Mutex<T> {
    pub const fn new(value:T)->Self {Self(std::sync::Mutex::new(value))}
    pub fn lock(&self)->std::sync::MutexGuard<'_,T>{self.0.lock().unwrap()}
    pub fn try_lock(&self)->Option<std::sync::MutexGuard<'_,T>>{self.0.try_lock().ok()}
}
pub struct VirtAddr(u64);
impl VirtAddr {pub fn new(value:u64)->Self{Self(value)}}
pub mod instructions {pub mod tlb {
    pub static FLUSHES: core::sync::atomic::AtomicUsize=core::sync::atomic::AtomicUsize::new(0);
    pub static HOOK: crate::Mutex<Option<Box<dyn Fn(u64)+Send>>> = crate::Mutex::new(None);
    pub fn flush(address:crate::VirtAddr){
        FLUSHES.fetch_add(1,core::sync::atomic::Ordering::Relaxed);
        if let Some(hook)=HOOK.lock().as_ref(){hook(address.0);}
    }
}}
pub struct HhdmRequest; pub struct HhdmResponse {pub offset:u64}
impl HhdmRequest {pub fn response(&self)->Option<HhdmResponse>{Some(HhdmResponse{offset:0})}}
pub static HHDM_REQUEST:HhdmRequest=HhdmRequest;
#[macro_export] macro_rules! serial_println {($($arg:tt)*)=>{{ let _=format_args!($($arg)*); }};}
#[path="../kernel/src/paging_state.rs"] mod paging_state;
#[path="../kernel/src/capability_slot.rs"] mod capability_slot;
mod capability {
    use crate::paging_state::*;
    #[derive(Clone,Copy)] pub enum CapabilityData {FramePool(FramePoolGrant),PagerMapping(PagerMappingGrant),DemandRegion(DemandRegionGrant),Other}
    pub struct Capability {pub data:CapabilityData}
    pub struct ProtectionDomain {pub id:u32,pub entries:Vec<(u32,CapabilityData)>}
    impl ProtectionDomain {pub fn find_capability(&self,slot:u32)->Option<Capability>{self.entries.iter().find(|(s,_)|*s==slot).map(|(_,data)|Capability{data:*data})}}
}
mod ipc {
    pub const MAX_DOMAINS:usize=1024;
    pub enum BootPhase {RegistryBuild=1}
    pub static BOOT_PHASE:core::sync::atomic::AtomicU8=core::sync::atomic::AtomicU8::new(1);
    pub mod pagefault {
        use crate::Mutex;
        #[derive(Clone,Copy)] pub struct FaultRecord {pub task_id:u32,pub token:u64,pub target_pd:u32,pub pager_pd:u32,pub page_addr:u64,pub error_code:u64}
        pub struct Slot {pub record:Option<FaultRecord>,pub claimed:bool,pub completed:u64,pub retries:usize,pub rollbacks:usize,pub wakes:usize}
        pub static SLOT:Mutex<Slot>=Mutex::new(Slot{record:None,claimed:false,completed:0,retries:0,rollbacks:0,wakes:0});
        pub fn query_fault(target:u32,pager:u32,token:u64)->Result<FaultRecord,u64>{
            let slot=SLOT.lock();let r=slot.record.ok_or(1u64)?;
            if r.token!=token || r.target_pd!=target || r.pager_pd!=pager {return Err(3)}
            Ok(r)
        }
        pub fn claim_fault(target:u32,pager:u32,token:u64)->Result<FaultRecord,u64>{let r=query_fault(target,pager,token)?;let mut s=SLOT.lock();if s.claimed{return Err(2)}s.claimed=true;Ok(r)}
        pub fn rollback_fault(_:u32,_:u32,token:u64)->bool{let mut s=SLOT.lock();if !s.claimed||s.record.map(|r|r.token)!=Some(token){return false}s.claimed=false;s.rollbacks+=1;true}
        pub fn complete_fault(_:u32,_:u32,token:u64)->Result<(),u64>{let mut s=SLOT.lock();if s.completed==token{return Ok(())}if s.retries>0{s.retries-=1;return Err(3)}if !s.claimed||s.record.map(|r|r.token)!=Some(token){return Err(1)}s.record=None;s.claimed=false;s.completed=token;s.wakes+=1;Ok(())}
        pub fn reject_fault(target:u32,pager:u32,token:u64)->Result<(),u64>{query_fault(target,pager,token)?;let mut s=SLOT.lock();if s.claimed{return Err(2)}s.record=None;Ok(())}
        pub static RETIRE_ERROR:core::sync::atomic::AtomicU64=core::sync::atomic::AtomicU64::new(0);
        pub static RETIRE_CALLS:core::sync::atomic::AtomicUsize=core::sync::atomic::AtomicUsize::new(0);
        pub fn retire_dead_fault(target:u32,pager:u32,task:u32)->Result<(),u64>{
            RETIRE_CALLS.fetch_add(1,core::sync::atomic::Ordering::Relaxed);
            let error=RETIRE_ERROR.load(core::sync::atomic::Ordering::Relaxed);if error!=0{return Err(error)}
            if crate::scheduler::validate_dead_target(target,pager)?.task_id!=task{return Err(3)}
            let mut s=SLOT.lock();if let Some(r)=s.record {
                if r.target_pd!=target || r.pager_pd!=pager || r.task_id!=task{return Err(3)}
            }
            s.record=None;s.claimed=false;Ok(())
        }
        pub fn set(token:u64,page:u64){set_target(token,15,page)}
        pub fn set_target(token:u64,target_pd:u32,page:u64){let mut s=SLOT.lock();s.record=Some(FaultRecord{task_id:0x8000+target_pd,token,target_pd,pager_pd:14,page_addr:page,error_code:6});s.claimed=false;}
    }
}
mod scheduler {
    use crate::paging_state::*;
    pub struct DeadTarget {pub task_id:u32}
    pub static DEATH:crate::Mutex<[u8;2]>=crate::Mutex::new([0;2]);
    pub static TASK_OVERRIDE:core::sync::atomic::AtomicU32=core::sync::atomic::AtomicU32::new(0);
    pub static ACKS:core::sync::atomic::AtomicUsize=core::sync::atomic::AtomicUsize::new(0);
    pub fn validate_dead_target(target:u32,pager:u32)->Result<DeadTarget,u64>{
        if pager!=14 || ![15,10].contains(&target){return Err(DENIED)}
        match DEATH.lock()[if target==15{0}else{1}] {0=>Err(LIVE),1=>Err(RETRY),_=>{
            let override_id=TASK_OVERRIDE.load(core::sync::atomic::Ordering::Relaxed);
            Ok(DeadTarget{task_id:if override_id==0{0x8000+target}else{override_id}})
        }}
    }
    pub fn acknowledge_dead_cleanup(target:u32,pager:u32,task:u32)->bool{
        if validate_dead_target(target,pager).map(|d|d.task_id)!=Ok(task){return false}
        ACKS.fetch_add(1,core::sync::atomic::Ordering::Relaxed);true
    }
}
#[path="../kernel/src/paging.rs"] mod paging;
use capability::*;
use paging_state::*;
#[repr(align(4096))] struct Frames([u8;PAGES*4096]);

#[test]
fn actual_commit_enforces_zero_unique_bindings_quota_and_exact_retry_without_side_effects() {
    let mut frames=Box::new(Frames([0;PAGES*4096]));
    let mut leaves=Box::new([0u64;PAGES]);
    let mut second_leaves=Box::new([0u64;PAGES]);
    let physical=frames.0.as_mut_ptr() as u64;
    let pool=FramePoolGrant{pool_id:1,base_va:0x5000_0000,length:PAGE*8};
    let map=PagerMappingGrant{range_id:1,pool_id:1,target_pd:15,base_va:0x4000_0000,length:PAGE*8,rights:6,quota:2};
    let second=PagerMappingGrant{range_id:2,target_pd:10,base_va:0x6000_0000,..map};
    let pager=ProtectionDomain{id:14,entries:vec![(34,CapabilityData::FramePool(pool)),(36,CapabilityData::PagerMapping(map)),(37,CapabilityData::PagerMapping(second)),(6,CapabilityData::Other)]};
    let mut addresses=[0;PAGES];for (i,a) in addresses.iter_mut().enumerate(){*a=(&mut leaves[i] as *mut u64) as u64;}
    let mut second_addresses=[0;PAGES];for (i,a) in second_addresses.iter_mut().enumerate(){*a=(&mut second_leaves[i] as *mut u64) as u64;}
    paging::initialize(pool,[map,second],14,[15,10],physical,[addresses,second_addresses]);
    assert_eq!(paging::info(&pager,0x1_0000_0022),[1,0,0,0,0]);
    assert_eq!(paging::info(&pager,6),[1,0,0,0,0]);
    assert_eq!(paging::commit(&pager,36,6,0,1,6),INVALID);
    assert_eq!(paging::commit(&pager,0x1_0000_0024,34,0,1,6),INVALID);
    ipc::pagefault::set(1,map.base_va);
    assert_eq!(paging::query(&pager,36,1),[0,map.base_va,6,15,6]);
    for (index,rights) in [(8,6),(0,0x1_0000_0006),(0,2)] {
        assert_ne!(paging::commit(&pager,36,34,index,1,rights),0);assert_eq!(*leaves,[0;PAGES]);
    }
    frames.0[19]=0xa5;
    assert_eq!(paging::commit(&pager,36,34,0,1,6),DENIED);
    assert_eq!(*leaves,[0;PAGES]);assert_eq!(frames.0[19],0xa5);
    frames.0.fill(0); // actual pager action, not performed by kernel commit
    ipc::pagefault::SLOT.lock().retries=1;
    assert_eq!(paging::commit(&pager,36,34,0,1,6),RETRY);
    let installed=leaves[0];assert_eq!(installed & 0x000f_ffff_ffff_f000,physical);
    assert_eq!((installed>>59)&15,15);assert_ne!(installed&(1<<63),0);
    assert_eq!(paging::reclaim(&pager,36,34,0,1),DENIED); // installed but not finalized
    assert_eq!(leaves[0],installed);
    frames.0[0]=0x7e; // target content must survive exact replay
    assert_eq!(paging::commit(&pager,36,34,0,1,6),0);
    assert_eq!(paging::commit(&pager,36,34,0,1,6),0);
    assert_eq!(paging::commit(&pager,36,34,1,1,6),DENIED);
    assert_eq!(paging::commit(&pager,36,34,0,1,2),DENIED);
    assert_eq!(leaves[0],installed);assert_eq!(frames.0[0],0x7e);
    assert_eq!(ipc::pagefault::SLOT.lock().wakes,1);
    ipc::pagefault::set(2,map.base_va+PAGE);
    assert_eq!(paging::commit(&pager,36,34,0,2,6),DENIED); // same frame, different target page
    assert_eq!(paging::commit(&pager,36,34,1,2,6),0);
    ipc::pagefault::set(3,map.base_va+2*PAGE);
    assert_eq!(paging::commit(&pager,36,34,2,3,6),EXHAUSTED);
    assert_eq!(leaves[2],0);assert_eq!(ipc::pagefault::SLOT.lock().wakes,2);
    assert_eq!(paging::reject(&pager,36,3),0);
    ipc::pagefault::set(4,map.base_va+map.length);
    assert_eq!(paging::query(&pager,36,4),[0,map.base_va+map.length,6,15,6]);
    assert_eq!(paging::commit(&pager,36,34,2,4,6),DENIED);
    assert_eq!(leaves[2],0);
    assert_eq!(paging::reject(&pager,36,4),0); // unsatisfiable fault is never stranded
    assert_eq!(instructions::tlb::FLUSHES.load(core::sync::atomic::Ordering::Relaxed),2);
    // Same relative page index in a different client is independent; the same
    // physical frame remains globally exclusive and quotas are per target.
    ipc::pagefault::set_target(5,10,second.base_va);
    assert_eq!(paging::commit(&pager,37,34,1,5,6),DENIED);
    assert_eq!(paging::commit(&pager,37,34,2,5,6),0);
    assert_eq!((second_leaves[0]>>59)&15,10);
    ipc::pagefault::set_target(6,10,second.base_va+PAGE);
    assert_eq!(paging::commit(&pager,37,34,3,6,6),0);
    ipc::pagefault::set_target(7,10,second.base_va+2*PAGE);
    assert_eq!(paging::commit(&pager,37,34,4,7,6),EXHAUSTED);
    assert_eq!(paging::reject(&pager,37,7),0);

    // All denial cases leave backing, leaf identity, and frame custody intact.
    for (slot,pool_slot,frame,token) in [(37,34,0,1),(36,6,0,1),(36,34,8,1),
        (36,34,u64::MAX,1),(36,34,0,0),(36,34,0,2),(0x1_0000_0024,34,0,1)] {
        assert_ne!(paging::reclaim(&pager,slot,pool_slot,frame,token),0);
        assert_eq!(leaves[0],installed);assert_eq!(frames.0[0],0x7e);
    }
    for changed in [installed ^ PAGE,installed ^ (1<<59),installed ^ 2,
        installed ^ 4,installed ^ (1<<63),installed | (1<<8)] {
        leaves[0]=changed;
        assert_eq!(paging::reclaim(&pager,36,34,0,1),DENIED);
        assert_eq!(leaves[0],changed);
    }
    leaves[0]=installed | 0x60; // hardware Accessed/Dirty changes are allowed
    let old_leaf_address=addresses[0] as usize;
    // At the actual production flush call, the leaf is already absent and the
    // transition lock still prevents an observer from committing/reclaiming.
    *instructions::tlb::HOOK.lock()=Some(Box::new(move |address| {
        assert_eq!(address,map.base_va);
        assert_eq!(unsafe{core::ptr::read_volatile(old_leaf_address as *const u64)},0);
        let probe=ProtectionDomain{id:14,entries:vec![(34,CapabilityData::FramePool(pool)),(36,CapabilityData::PagerMapping(map))]};
        assert_eq!(paging::commit(&probe,36,34,0,1,6),RETRY);
        assert_eq!(paging::reclaim(&probe,36,34,0,1),RETRY);
    }));
    assert_eq!(paging::reclaim(&pager,36,34,0,1),0);
    *instructions::tlb::HOOK.lock()=None;
    assert_eq!(leaves[0],0);assert_eq!(frames.0[0],0x7e); // kernel never zeroes
    let flushes=instructions::tlb::FLUSHES.load(core::sync::atomic::Ordering::Relaxed);
    assert_eq!(paging::reclaim(&pager,36,34,0,1),0);
    assert_eq!(paging::commit(&pager,36,34,0,1,6),DENIED);
    assert_eq!(instructions::tlb::FLUSHES.load(core::sync::atomic::Ordering::Relaxed),flushes);

    // Restore the second client's quota, then reuse the reclaimed primary frame
    // there. Old retries cannot unmap or zero this newly assigned owner.
    assert_eq!(paging::reclaim(&pager,37,34,2,5),0);
    frames.0[..4096].fill(0); // trusted sext zero-before-reexposure
    ipc::pagefault::set_target(8,10,second.base_va);
    assert_eq!(paging::commit(&pager,37,34,0,8,6),0);
    let rebound=second_leaves[0];frames.0[0]=0x91;
    assert_eq!(paging::reclaim(&pager,36,34,0,1),0);
    assert_eq!(paging::commit(&pager,36,34,0,1,6),DENIED);
    assert_eq!(second_leaves[0],rebound);assert_eq!(frames.0[0],0x91);
    // Even a forged mock pending record cannot resurrect a retired token into
    // another frame; production FaultSlot also rejects such generation reuse.
    ipc::pagefault::set(1,map.base_va);
    assert_eq!(paging::commit(&pager,36,34,4,1,6),DENIED);
    assert_eq!(leaves[0],0);
    assert_eq!(paging::reject(&pager,36,1),0);
    assert_eq!(paging::reclaim(&pager,37,34,0,8),0);
    assert_eq!(paging::reclaim(&pager,36,34,0,1),DENIED); // tombstone replaced
    assert_eq!(paging::commit(&pager,36,34,1,2,6),0); // still-live older binding
    assert_eq!(paging::commit(&pager,37,34,3,6,6),0); // older than retired floor8

    // Repeated alternating-client reuse needs no growing kernel ledger. Both
    // per-target quotas are restored, old generation never revives, and every
    // new PTE has the destination's own pkey.
    for token in 9..1009 {
        let (target,slot,grant,pkey)=if token%2==0 {(15,36,map,15)} else {(10,37,second,10)};
        frames.0[..4096].fill(0);
        ipc::pagefault::set_target(token,target,grant.base_va);
        assert_eq!(paging::commit(&pager,slot,34,0,token,6),0);
        let active=if target==15 {leaves[0]} else {second_leaves[0]};
        assert_eq!((active>>59)&15,pkey);
        assert_eq!(paging::reclaim(&pager,slot,34,0,token),0);
        assert_eq!(paging::reclaim(&pager,slot,34,0,token),0);
        assert_eq!(paging::commit(&pager,slot,34,0,token,6),DENIED);
    }
    assert_eq!(leaves[0],0);assert_eq!(second_leaves[0],0);

    // Exit cleanup includes a published read-only mapping whose fault
    // completion returned RETRY; the normal per-frame reclaim still refuses it.
    frames.0[..4096].fill(0);
    ipc::pagefault::set(1009,map.base_va);
    ipc::pagefault::SLOT.lock().record.as_mut().unwrap().error_code=4;
    ipc::pagefault::SLOT.lock().retries=1;
    assert_eq!(paging::commit(&pager,36,34,0,1009,READ as u64),RETRY);
    assert_eq!(leaves[0]&2,0);
    assert_eq!(paging::reclaim(&pager,36,34,0,1009),DENIED);
    let before=*leaves;let other_before=*second_leaves;
    assert_eq!(paging::cleanup(&pager,36,34),[LIVE,0,0]);
    scheduler::DEATH.lock()[0]=1; // Exited but still on outgoing stack
    assert_eq!(paging::cleanup(&pager,36,34),[RETRY,0,0]);
    assert_eq!(*leaves,before);
    scheduler::DEATH.lock()[0]=2; // authoritative dead/detached snapshot
    for (slot,poolslot) in [(6,34),(36,6),(0x1_0000_0024,34),(36,0x1_0000_0022)] {
        assert_eq!(paging::cleanup(&pager,slot,poolslot),[INVALID,0,0]);
    }
    assert_eq!(paging::cleanup(&pager,37,34),[LIVE,0,0]); // different live target
    scheduler::TASK_OVERRIDE.store(0xdead,core::sync::atomic::Ordering::Relaxed);
    assert_eq!(paging::cleanup(&pager,36,34),[DENIED,0,0]);
    scheduler::TASK_OVERRIDE.store(0,core::sync::atomic::Ordering::Relaxed);
    // A later leaf mismatch must abort before clearing the earlier valid leaf.
    leaves[1]^=PAGE;
    assert_eq!(paging::cleanup(&pager,36,34),[DENIED,0,0]);
    assert_eq!(leaves[0],before[0]);leaves[1]=before[1];
    leaves[7]=before[0]; // an unowned unexpected PTE cannot survive successful closure
    assert_eq!(paging::cleanup(&pager,36,34),[DENIED,0,0]);leaves[7]=0;
    assert_eq!(ipc::pagefault::RETIRE_CALLS.load(core::sync::atomic::Ordering::Relaxed),0);
    ipc::pagefault::RETIRE_ERROR.store(RETRY,core::sync::atomic::Ordering::Relaxed);
    assert_eq!(paging::cleanup(&pager,36,34),[RETRY,0,0]);
    assert_eq!(*leaves,before);assert_eq!(*second_leaves,other_before);
    assert!(ipc::pagefault::SLOT.lock().claimed);
    ipc::pagefault::RETIRE_ERROR.store(0,core::sync::atomic::Ordering::Relaxed);
    leaves[0]|=0x60;leaves[1]|=0x60;
    let address0=addresses[0] as usize;let address1=addresses[1] as usize;
    let flush_count=std::sync::Arc::new(core::sync::atomic::AtomicUsize::new(0));
    let count=flush_count.clone();
    *instructions::tlb::HOOK.lock()=Some(Box::new(move |address| {
        let index=count.fetch_add(1,core::sync::atomic::Ordering::Relaxed);
        assert_eq!(address,map.base_va+index as u64*PAGE);
        assert_eq!(unsafe{core::ptr::read_volatile(address0 as *const u64)},0);
        if index==1 {assert_eq!(unsafe{core::ptr::read_volatile(address1 as *const u64)},0);}
        let probe=ProtectionDomain{id:14,entries:vec![(34,CapabilityData::FramePool(pool)),(36,CapabilityData::PagerMapping(map))]};
        assert_eq!(paging::cleanup(&probe,36,34),[RETRY,0,0]);
        assert_eq!(paging::commit(&probe,36,34,0,1009,2),RETRY);
    }));
    let cleaned=[0,3,0x8000+15];
    assert_eq!(paging::cleanup(&pager,36,34),cleaned);
    *instructions::tlb::HOOK.lock()=None;
    assert_eq!(flush_count.load(core::sync::atomic::Ordering::Relaxed),2);
    assert_eq!(*leaves,[0;PAGES]);assert_eq!(*second_leaves,other_before);
    assert!(ipc::pagefault::SLOT.lock().record.is_none());
    assert_eq!(scheduler::ACKS.load(core::sync::atomic::Ordering::Relaxed),1);
    assert_eq!(paging::cleanup(&pager,36,34),cleaned);
    assert_eq!(paging::commit(&pager,36,34,0,1009,2),DENIED);
    assert_eq!(paging::commit(&pager,36,34,1,2,6),DENIED);
    ipc::pagefault::set(2000,map.base_va);
    assert_eq!(paging::query(&pager,36,2000),[DENIED,0,0,0,0]);
    assert_eq!(paging::commit(&pager,36,34,4,2000,6),DENIED); // permanently closed
    ipc::pagefault::SLOT.lock().record=None;
    frames.0[..4096].fill(0);
    ipc::pagefault::set_target(2001,10,second.base_va);
    assert_eq!(paging::commit(&pager,37,34,0,2001,6),0);
    let new_leaf=second_leaves[0];frames.0[0]=0x55;
    let old_flushes=instructions::tlb::FLUSHES.load(core::sync::atomic::Ordering::Relaxed);
    assert_eq!(paging::cleanup(&pager,36,34),cleaned); // cached old result after reuse
    assert_eq!(second_leaves[0],new_leaf);assert_eq!(frames.0[0],0x55);
    assert_eq!(instructions::tlb::FLUSHES.load(core::sync::atomic::Ordering::Relaxed),old_flushes);
    assert_eq!(scheduler::ACKS.load(core::sync::atomic::Ordering::Relaxed),1);

    // Death with an unpublished pending fault and no live mappings still
    // retires the fault and permanently closes the range without a TLB write.
    assert_eq!(paging::reclaim(&pager,37,34,0,2001),0);
    assert_eq!(paging::reclaim(&pager,37,34,3,6),0);
    ipc::pagefault::set_target(3000,10,second.base_va);
    scheduler::DEATH.lock()[1]=2;
    let count=instructions::tlb::FLUSHES.load(core::sync::atomic::Ordering::Relaxed);
    assert_eq!(paging::cleanup(&pager,37,34),[0,0,0x8000+10]);
    assert!(ipc::pagefault::SLOT.lock().record.is_none());
    assert_eq!(instructions::tlb::FLUSHES.load(core::sync::atomic::Ordering::Relaxed),count);
    assert_eq!(paging::commit(&pager,37,34,0,3001,6),DENIED);

}
