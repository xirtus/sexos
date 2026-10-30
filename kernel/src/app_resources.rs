// SPDX-License-Identifier: GPL-3.0-only
//! Boot-private app payload ownership. Kernel identities remain permanent.
//! Sext owns the retired bytes; this module never chooses or allocates frames at runtime.
use core::sync::atomic::{AtomicBool,AtomicU32,Ordering};
use crate::capability::{CapabilityData,ProtectionDomain};
use crate::app_resource_state::*;
use spin::Mutex;
use x86_64::VirtAddr;
use crate::scheduler::task_ownership::Phase;
struct App {
    target:u32,task:u32,main:usize,trampoline:usize,pkey:u8,surface:u64,
    pages:[PrivatePage;MAX_PAGES],count:usize,captured:bool,ready:bool,
    pool:u64,coordinator:u32,pager:u32,services:[u32;5],acked:u8,
    identity:Incarnation,predecessor:u32,activated:bool,retired:bool,last_hint:[u64;6],hinted:u8,kicked:bool,
}
impl App {const fn new()->Self{Self{target:0,task:0,main:0,trampoline:0,pkey:0,surface:0,
    pages:[PrivatePage::EMPTY;MAX_PAGES],count:0,captured:false,ready:false,pool:0,
    coordinator:0,pager:0,services:[0;5],acked:0,identity:Incarnation::EMPTY,predecessor:0,activated:false,retired:false,last_hint:[0;6],hinted:0,kicked:false}}}
static APPS:Mutex<[App;MAX_APPS]>=Mutex::new([const{App::new()};MAX_APPS]);
static TARGETS:[AtomicU32;MAX_APPS]=[const{AtomicU32::new(0)};MAX_APPS];
static DORMANT:[AtomicBool;MAX_APPS]=[const{AtomicBool::new(false)};MAX_APPS];
static TERMINAL:[AtomicBool;MAX_APPS]=[const{AtomicBool::new(false)};MAX_APPS];
fn boot(){assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),crate::ipc::BootPhase::RegistryBuild as u8,"APP_GENESIS_AFTER_SEAL");}
pub fn plan_boot_apps(targets:&[u32]){
    boot();assert!(targets.len()<=MAX_APPS);let mut apps=APPS.lock();
    for (index,&target) in targets.iter().enumerate(){
        assert!(target!=0 && !targets[..index].contains(&target) && apps[index].target==0,"APP_PLAN_IDENTITY");
        apps[index].target=target;apps[index].identity=Incarnation::original(target);TARGETS[index].store(target,Ordering::Release);
    }
}
fn capture_range(app:&mut App,start:u64,length:u64)->Result<(),&'static str>{
    if length==0{return Ok(())}
    let first=start&!4095;let end=start.checked_add(length-1).ok_or("APP_RANGE_OVERFLOW")?&!4095;
    let mut va=first;
    loop {
        if !app.pages[..app.count].iter().any(|p|p.va==va){
            if app.count==MAX_PAGES{return Err("APP_MANIFEST_CAPACITY")}
            let leaf=crate::boot_memory::existing_leaf_address(va).ok_or("APP_PRIVATE_LEAF_MISSING")?;
            let expected=unsafe{core::ptr::read_volatile(leaf as *const u64)};
            if expected&5!=5 || (expected>>59)&15!=app.pkey as u64{return Err("APP_PRIVATE_PKEY_OR_ACCESS")}
            if app.pages[..app.count].iter().any(|p|p.expected&PHYSICAL_MASK==expected&PHYSICAL_MASK){return Err("APP_PRIVATE_PHYSICAL_ALIAS")}
            app.pages[app.count]=PrivatePage{va,leaf,expected:expected&!0x60};app.count+=1;
        }
        if va==end{break}va=va.checked_add(4096).ok_or("APP_RANGE_OVERFLOW")?;
    }
    Ok(())
}
/// Called only after loader final permissions/relocations and stack creation.
pub fn capture_boot_image(target:u32,pkey:u8,elf:&[u8],base:u64,stack_top:u64)->Result<(),&'static str>{
    boot();let mut apps=APPS.lock();let Some(app)=apps.iter_mut().find(|a|a.target==target)else{return Ok(())};
    if app.captured{return Err("APP_IMAGE_CAPTURE_REPEATED")}
    app.pkey=pkey;
    if elf.len()<core::mem::size_of::<crate::elf::ElfHeader>(){return Err("APP_ELF_HEADER")}
    let header=unsafe{core::ptr::read_unaligned(elf.as_ptr() as *const crate::elf::ElfHeader)};
    let phsize=header.phentsize as usize;
    if phsize<core::mem::size_of::<crate::elf::ProgramHeader>(){return Err("APP_ELF_PH_SIZE")}
    let phstart=usize::try_from(header.phoff).map_err(|_|"APP_ELF_PH_OFFSET")?;
    let phend=phstart.checked_add(phsize.checked_mul(header.phnum as usize).ok_or("APP_ELF_PH_OVERFLOW")?).ok_or("APP_ELF_PH_OVERFLOW")?;
    if phend>elf.len(){return Err("APP_ELF_PH_BOUNDS")}
    let ph=|i:usize|unsafe{core::ptr::read_unaligned(elf.as_ptr().add(phstart+i*phsize) as *const crate::elf::ProgramHeader)};
    let bias=(0..header.phnum as usize).map(ph).filter(|p|p.p_type==crate::elf::PT_LOAD && p.p_memsz!=0).map(|p|p.p_vaddr).min().ok_or("APP_ELF_EMPTY")?;
    for index in 0..header.phnum as usize {let p=ph(index);if p.p_type==crate::elf::PT_LOAD && p.p_memsz!=0{
        let va=base.checked_add(p.p_vaddr.checked_sub(bias).ok_or("APP_ELF_BIAS")?).ok_or("APP_ELF_VA")?;
        capture_range(app,va,p.p_memsz)?;
    }}
    capture_range(app,stack_top.checked_sub(65536).ok_or("APP_STACK_RANGE")?,65536)?;
    app.captured=true;Ok(())
}
pub fn init(pager:u32,coordinator:u32,catalog:&[(u32,u64)],services:[u32;5]){
    boot();assert!(catalog.len()<=MAX_APPS);
    let pager_pd=crate::ipc::DOMAIN_REGISTRY.get(pager).expect("APP_PAGER_MISSING");
    let mut apps=APPS.lock();
    for (slot_index,&(target,surface)) in catalog.iter().enumerate(){
        let index=apps.iter().position(|a|a.target==target).expect("APP_NOT_PLANNED");
        let app=&mut apps[index];assert!(app.captured && !app.ready && app.count!=0,"APP_IMAGE_NOT_READY");
        let pd=crate::ipc::DOMAIN_REGISTRY.get(target).expect("APP_TARGET_MISSING");
        let main=pd.main_task.load(Ordering::Acquire);assert!(!main.is_null(),"APP_NO_MAIN");
        app.main=main as usize;app.task=unsafe{(*main).id};app.trampoline=pd.trampoline_task.load(Ordering::Acquire) as usize;
        app.surface=surface;app.pager=pager;app.coordinator=coordinator;app.services=services;
        let bytes=app.count*4096;let alias=crate::memory::va_allocator::allocate_va(bytes).expect("APP_NO_ALIAS_VA");
        {let mut vas=crate::memory::manager::GLOBAL_VAS.lock();let vas=vas.as_mut().expect("APP_NO_GLOBAL_VAS");
            use x86_64::structures::paging::PageTableFlags as F;
            for (i,page) in app.pages[..app.count].iter().enumerate(){
                vas.map_physical_range(VirtAddr::new(alias+i as u64*4096),page.expected&PHYSICAL_MASK,4096,
                    F::PRESENT|F::USER_ACCESSIBLE|F::WRITABLE|F::NO_EXECUTE,pager_pd.pku_key).expect("APP_ALIAS_MAP_FAILED");
            }
        }
        app.pool=alias;
        let slot=sex_pdx::app_lifecycle::SLOTS[slot_index] as u64;
        for (role,&holder) in services.iter().enumerate(){
            assert!(holder!=0 && holder!=target && !services[..role].contains(&holder),"APP_SERVICE_IDENTITY");
            let pd=crate::ipc::DOMAIN_REGISTRY.get(holder).expect("APP_SERVICE_MISSING");
            assert!(pd.find_capability(slot as u32).is_none(),"APP_GRANT_SLOT_OCCUPIED");
            pd.grant_capability(slot,CapabilityData::AppLifecycle(LifecycleGrant{target_pd:target,task_id:app.task,holder_pd:holder,rights:QUERY|ACK,ack_bit:1<<role}));
        }
        assert!(pager_pd.find_capability(slot as u32).is_none(),"APP_PAGER_SLOT_OCCUPIED");
        pager_pd.grant_capability(slot,CapabilityData::AppLifecycle(LifecycleGrant{target_pd:target,task_id:app.task,holder_pd:pager,rights:QUERY|RETIRE,ack_bit:0}));
        app.ready=true;
        crate::serial_println!("[boot.app.resources] target={} task={} surface={} slot={} private_pages={} private_bytes={} required={} pager={} coordinator={} metadata=retained",target,app.task,surface,slot,app.count,bytes,REQUIRED,pager,coordinator);
    }
    // Every captured physical frame belongs to exactly one private app pool.
    for i in 0..MAX_APPS{for j in 0..i{for page in &apps[i].pages[..apps[i].count]{assert!(!apps[j].pages[..apps[j].count].iter().any(|p|p.expected&PHYSICAL_MASK==page.expected&PHYSICAL_MASK),"APP_CROSS_IMAGE_ALIAS");}}}
}
/// Genesis reserves a fresh image and permanently binds the one activation
/// grant. Clear successor leaves before any predecessor user execution.
pub fn reserve_successor(predecessor:u32,successor:u32,holder:u32,slot:usize){
    boot();let mut apps=APPS.lock();
    let old=apps.iter().find(|a|a.ready&&a.target==predecessor).expect("SUCCESSOR_NO_PREDECESSOR");
    let old_task=old.task;let old_key=old.pkey;
    let next=old.identity.next().expect("SUCCESSOR_INCARNATION_EXHAUSTED");
    let index=apps.iter().position(|a|a.ready&&a.target==successor).expect("SUCCESSOR_NO_IMAGE");
    let app=&mut apps[index];assert!(app.predecessor==0 && successor!=predecessor && app.pkey==old_key,"SUCCESSOR_BIND");
    let task=unsafe{&*(app.main as *const crate::scheduler::Task)};
    assert!(task.state.snapshot().phase==Phase::Constructed,"SUCCESSOR_ALREADY_ACTIVE");
    app.predecessor=predecessor;app.identity=next;
    DORMANT[index].store(true,Ordering::Release);
    retire_private_pages(&app.pages[..app.count],
        |leaf|unsafe{core::ptr::read_volatile(leaf as *const u64)},
        |leaf|unsafe{core::ptr::write_volatile(leaf as *mut u64,0)},
        |va|x86_64::instructions::tlb::flush(VirtAddr::new(va))).expect("SUCCESSOR_RESERVE_LEAVES");
    let coordinator=crate::ipc::DOMAIN_REGISTRY.get(holder).expect("SUCCESSOR_NO_HOLDER");
    assert!(coordinator.find_capability(slot as u32).is_none());
    coordinator.grant_capability(slot as u64,CapabilityData::AppActivation(ActivationGrant{
        holder,logical_app:next.logical_app,incarnation:next.number,predecessor,predecessor_task:old_task,
        successor,successor_task:app.task}));
    crate::serial_println!("[boot.app.successor] logical={} incarnation={} predecessor={} successor={} task={} surface={} key={} dormant=1 leaves={} runtime_mint=0",next.logical_app,next.number,predecessor,successor,app.task,app.surface,app.pkey,app.count);
}
/// Scheduler first-admission fences both BOOT sweeps. Ordinary wakes cannot
/// activate Constructed tasks; the activation commit drops this fence last.
pub fn activation_permitted(target:u32)->bool{
    !TARGETS.iter().enumerate().any(|(i,t)|t.load(Ordering::Acquire)==target && DORMANT[i].load(Ordering::Acquire))
}
fn grant(pd:&ProtectionDomain,slot:u64)->Result<LifecycleGrant,u64>{
    let slot=crate::capability_slot::decode_capability_slot(slot).ok_or(INVALID)?;
    match pd.find_capability(slot).map(|c|c.data){Some(CapabilityData::AppLifecycle(g))=>Ok(g),_=>Err(INVALID)}
}
fn dead(app:&App)->Result<(),u64>{
    let pd=crate::ipc::DOMAIN_REGISTRY.get(app.target).ok_or(DENIED)?;
    let main=pd.main_task.load(Ordering::Acquire);
    if main as usize!=app.main || main.is_null() || unsafe{(*main).id}!=app.task{return Err(DENIED)}
    let s=unsafe{(*main).state.snapshot()};if !s.exited{return Err(LIVE)}
    if s.phase!=Phase::Dead || s.owner_core.is_some(){return Err(RETRY)}
    let tramp=pd.trampoline_task.load(Ordering::Acquire);
    if tramp as usize!=app.trampoline{return Err(DENIED)}
    if !tramp.is_null(){let s=unsafe{(*tramp).state.snapshot()};if !matches!(s.phase,Phase::Constructed|Phase::Dead)||s.owner_core.is_some(){return Err(RETRY)}}
    Ok(())
}
pub fn accepts_ipc(target:u32)->bool{
    !TARGETS.iter().enumerate().any(|(i,t)|t.load(Ordering::Acquire)==target && (TERMINAL[i].load(Ordering::Acquire)||DORMANT[i].load(Ordering::Acquire)))
}
pub fn status(pd:&ProtectionDomain,slot:u64,kind:u64)->[u64;5]{
    let result=(||{let g=grant(pd,slot)?;let apps=APPS.try_lock().ok_or(RETRY)?;
        let app=apps.iter().find(|a|a.ready&&a.target==g.target_pd).ok_or(DENIED)?;
        g.authorize(pd.id,app.target,app.task,QUERY)?;
        if kind==1{return Ok([0,app.target as u64,app.task as u64,app.surface,app.coordinator as u64])}
        if kind!=0{return Err(INVALID)}dead(app)?;
        Ok([0,app.target as u64,app.task as u64,REQUIRED as u64,app.acked as u64])
    })();result.unwrap_or_else(|e|[e,0,0,0,0])
}
/// BSP serialized snapshot/preflight/commit. No runtime frame selection,
/// ELF loading, capability minting, or predecessor identity mutation.
pub fn activation(pd:&ProtectionDomain,slot:u64,kind:u64)->[u64;5]{
    let result=(||{
        if kind>1{return Err(INVALID)}
        let slot=crate::capability_slot::decode_capability_slot(slot).ok_or(INVALID)?;
        let g=match pd.find_capability(slot).map(|c|c.data){Some(CapabilityData::AppActivation(g))=>g,_=>return Err(INVALID)};
        let mut apps=APPS.try_lock().ok_or(RETRY)?;
        let oi=apps.iter().position(|a|a.ready&&a.target==g.predecessor).ok_or(DENIED)?;
        let ni=apps.iter().position(|a|a.ready&&a.target==g.successor).ok_or(DENIED)?;
        let old=&apps[oi];let new=&apps[ni];
        g.authorize(pd.id,old.target,old.task,new.target,new.task,old.identity,new.identity)?;
        if new.predecessor!=old.target{return Err(DENIED)}
        // Once consumed, replay never revalidates or restores retired leaves.
        if new.activated{return Ok([0,new.target as u64,new.task as u64,new.surface,
            if TERMINAL[ni].load(Ordering::Acquire){3}else{2}])}
        let ready=old.retired && old.acked==REQUIRED && dead(old).is_ok();
        let reply=[0,new.target as u64,new.task as u64,new.surface,if ready{1}else{0}];
        if kind==0{return Ok(reply)}
        if !ready{return Err(RETRY)}
        // Local TLB retirement is valid only while exactly the BSP has ever
        // initialized execution. Fail closed if another core is introduced.
        if crate::core_local::INITIALIZED_CORES.load(Ordering::Acquire)!=1
            || crate::core_local::CoreLocal::get().core_id!=0{return Err(DENIED)}
        let target=crate::ipc::DOMAIN_REGISTRY.get(new.target).ok_or(DENIED)?;
        let main=target.main_task.load(Ordering::Acquire);
        let tramp=target.trampoline_task.load(Ordering::Acquire);
        if main as usize!=new.main || main.is_null() || tramp as usize!=new.trampoline{return Err(DENIED)}
        let task=unsafe{&*main};let state=task.state.snapshot();
        if task.id!=new.task || task.context.pd_id!=new.target as u64
            || task.context.pd_ptr!=target as *const _ || task.context.rip==0
            || state.phase!=Phase::Constructed || state.owner_core.is_some() || state.exited{return Err(DENIED)}
        if !tramp.is_null(){let s=unsafe{(*tramp).state.snapshot()};
            if s.phase!=Phase::Constructed || s.owner_core.is_some() || s.exited{return Err(DENIED)}}
        let count=restore_reserved_pages(&new.pages[..new.count],
            |leaf|unsafe{core::ptr::read_volatile(leaf as *const u64)},
            |leaf,value|unsafe{core::ptr::write_volatile(leaf as *mut u64,value)},
            |va|x86_64::instructions::tlb::flush(VirtAddr::new(va)))?;
        // All fallible validation is above. IF is masked by the syscall
        // entry. Publish authority and receipt before scheduler publication.
        let new=&mut apps[ni];new.activated=true;
        DORMANT[ni].store(false,Ordering::Release);
        assert!(crate::scheduler::SCHEDULERS[0].activate_task(main),"SUCCESSOR_EXCLUSIVE_COMMIT");
        crate::serial_println!("[app.activation.committed] logical={} incarnation={} predecessor={} successor={} task={} surface={} pages={} runtime_alloc=0 runtime_mint=0 bsp=1",g.logical_app,g.incarnation,g.predecessor,g.successor,g.successor_task,new.surface,count);
        Ok([0,new.target as u64,new.task as u64,new.surface,2])
    })();result.unwrap_or_else(|e|[e,0,0,0,0])
}
pub fn ack(pd:&ProtectionDomain,slot:u64,task:u64)->u64{
    let result=(||{let g=grant(pd,slot)?;let mut apps=APPS.try_lock().ok_or(RETRY)?;
        let app=apps.iter_mut().find(|a|a.ready&&a.target==g.target_pd).ok_or(DENIED)?;
        g.authorize(pd.id,app.target,app.task,ACK)?;if task!=app.task as u64{return Err(DENIED)}dead(app)?;
        let old=app.acked;app.acked|=g.ack_bit;
        if app.acked!=old{crate::serial_println!("[app.service.ack] target={} task={} holder={} role={} acked={} required={}",app.target,app.task,pd.id,g.ack_bit,app.acked,REQUIRED);}
        Ok(())})();result.map_or_else(|e|e,|()|0)
}
pub fn retire(pd:&ProtectionDomain,slot:u64)->[u64;4]{
    let result=(||{let g=grant(pd,slot)?;let mut apps=APPS.try_lock().ok_or(RETRY)?;
        let index=apps.iter().position(|a|a.ready&&a.target==g.target_pd).ok_or(DENIED)?;
        let app=&mut apps[index];g.authorize(pd.id,app.target,app.task,RETIRE)?;
        if app.retired{return Ok([0,app.pool,app.count as u64*4096,app.task as u64])}
        dead(app)?;
        if app.acked!=REQUIRED || !crate::paging::target_cleanup_complete(app.target)?{return Err(RETRY)}
        TERMINAL[index].store(true,Ordering::Release);
        let count=retire_private_pages(&app.pages[..app.count],
            |leaf|unsafe{core::ptr::read_volatile(leaf as *const u64)},
            |leaf|unsafe{core::ptr::write_volatile(leaf as *mut u64,0)},
            |va|x86_64::instructions::tlb::flush(VirtAddr::new(va)))?;
        // No pointer-owning control object is freed. Late refs remain valid but
        // routing is fenced, and all private-payload consumers acknowledged.
        let target=crate::ipc::DOMAIN_REGISTRY.get(app.target).expect("APP_REGISTRY_CHANGED");
        let mut messages=0;while unsafe{(*target.message_ring).dequeue()}.is_some(){messages+=1;assert!(messages<=256,"APP_DRAIN_PRODUCER");}
        let mut signals=0;while unsafe{(*target.signal_ring).dequeue()}.is_some(){signals+=1;assert!(signals<=32,"APP_SIGNAL_PRODUCER");}
        crate::ipc::messages::IpcReply::erase_queue(&mut target.incoming_replies.lock());
        app.retired=true;
        crate::serial_println!("[app.private.retired] target={} task={} pages={} bytes={} acked={} required={} all_flushed=1 pool_reusable=1 messages={} signals={} metadata=retained runtime_alloc=0",app.target,app.task,count,count*4096,app.acked,REQUIRED,messages,signals);
        Ok([0,app.pool,app.count as u64*4096,app.task as u64])
    })();result.unwrap_or_else(|e|[e,0,0,0])
}
/// Retry each unacknowledged recipient at a bounded 16-tick cadence. Accepted
/// hints can be lost by nested userland waits, so acceptance never substitutes
/// for the service's capability-authenticated ACK.
pub fn poll_exit_notifications(){
    let Some(mut apps)=APPS.try_lock()else{return};
    let now=crate::interrupts::TICKS.load(Ordering::Relaxed);
    for (index,app) in apps.iter_mut().enumerate(){
        if !app.ready || app.retired{continue}
        let main=app.main as *mut crate::scheduler::Task;
        if !unsafe{(*main).state.snapshot().exited}{continue}
        if !app.kicked{for holder in app.services.into_iter().chain(core::iter::once(app.pager)){
            if let Some(pd)=crate::ipc::DOMAIN_REGISTRY.get(holder){crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire));}
        }app.kicked=true;}
        if dead(app).is_err(){continue}TERMINAL[index].store(true,Ordering::Release);
        for recipient in 0..6{
            if recipient<5 && app.acked&(1<<recipient)!=0{continue}
            let bit=1u8<<recipient;
            if app.hinted&bit!=0 && now.wrapping_sub(app.last_hint[recipient])<16{continue}
            let holder=if recipient==5{app.pager}else{app.services[recipient]};
            let Some(pd)=crate::ipc::DOMAIN_REGISTRY.get(holder)else{continue};
            let msg=crate::ipc::messages::MessageType::IpcCall{func_id:sex_pdx::app_lifecycle::OP_APP_EXIT,arg0:app.target as u64,arg1:app.task as u64,arg2:0,caller_pd:0,caller_task:0};
            // Back off full queues too; every unsuccessful attempt remains pending.
            app.hinted|=bit;app.last_hint[recipient]=now;
            if unsafe{(*pd.message_ring).enqueue(msg)}.is_ok(){
                crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire));
                crate::serial_println!("[app.exit.notice] target={} task={} holder={} detached=1 accepted=1",app.target,app.task,holder);
            }
        }
    }
}
