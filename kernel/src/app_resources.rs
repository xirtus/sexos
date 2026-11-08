// SPDX-License-Identifier: GPL-3.0-only
//! Boot-private app payload ownership. Kernel identities remain permanent.
//! Sext owns the retired bytes of BOOT apps. Program rows are the exception: a run's
//! image and heap live in the row's arena, on frames `program_frames` lends per run.
use core::sync::atomic::{AtomicBool,AtomicU32,Ordering};
use crate::capability::{CapabilityData,ProtectionDomain};
use crate::app_resource_state::*;
use spin::Mutex;
use x86_64::VirtAddr;
use crate::scheduler::task_ownership::Phase;
struct App {
    target:u32,task:u32,main:usize,trampoline:usize,pkey:u8,surface:u64,
    pages:alloc::vec::Vec<PrivatePage>,count:usize,captured:bool,ready:bool,
    pool:u64,coordinator:u32,pager:u32,services:[u32;5],acked:u8,
    identity:Incarnation,predecessor:u32,activated:bool,retired:bool,last_hint:[u64;6],hinted:u8,kicked:bool,
    /// Program rows run again. The grants name boot_task; `task` is the current run.
    reusable:bool,boot_task:u32,generation:u32,rebirthing:bool,pristine:u64,entry:u64,stack_top:u64,
    /// Shell took this run's pages because it ignored Close; retire need not find them.
    forced:bool,
    /// The BOOT image's manifest, and the arena a disk program and every run's heap use:
    /// a span of address space only this row maps, with page tables made at BOOT and
    /// frames lent per run.
    boot_pages:[PrivatePage;64],boot_count:usize,arena:u64,arena_pages:usize,
    /// Arena pages the run's image spans (none for the BOOT image, which lives in its
    /// own slot), and pages it was given past them since it started.
    image_pages:usize,grown:usize,
}
impl App {const fn new()->Self{Self{target:0,task:0,main:0,trampoline:0,pkey:0,surface:0,
    pages:alloc::vec::Vec::new(),count:0,captured:false,ready:false,pool:0,
    coordinator:0,pager:0,services:[0;5],acked:0,identity:Incarnation::EMPTY,predecessor:0,activated:false,retired:false,reusable:false,boot_task:0,generation:0,rebirthing:false,pristine:0,entry:0,forced:false,stack_top:0,boot_pages:[PrivatePage::EMPTY;64],boot_count:0,arena:0,arena_pages:0,image_pages:0,grown:0,last_hint:[0;6],hinted:0,kicked:false}}}
impl App {
    /// Sets manifest entry `i`, growing the manifest by one when `i` is its end.
    fn put(&mut self,i:usize,page:PrivatePage){
        if i<self.pages.len(){self.pages[i]=page}else{assert_eq!(i,self.pages.len(),"APP_MANIFEST_GAP");self.pages.push(page)}
    }
    fn in_arena(&self,va:u64)->bool{self.arena!=0&&va>=self.arena&&va<self.arena+(self.arena_pages*4096) as u64}
}
static NEXT_RUN_TASK:AtomicU32=AtomicU32::new(0x4000_0000);
static APPS:Mutex<[App;MAX_APPS]>=Mutex::new([const{App::new()};MAX_APPS]);
/// Syscalls run with interrupts off, so a holder on another core finishes soon.
/// The timer path keeps its plain try_lock: it can wait for the next tick.
macro_rules! apps_lock{()=>{{
    let mut held=None;
    // Its holder may be waiting for this core to answer a shootdown.
    for _ in 0..(1u32<<24){held=APPS.try_lock();if held.is_some(){break}crate::smp::tlb::service();core::hint::spin_loop();}
    held
}}}
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
            let at=app.count;app.put(at,PrivatePage{va,leaf,expected:expected&!0x60});app.count+=1;
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
        app.main=main as usize;app.task=unsafe{(*main).id};app.boot_task=app.task;app.trampoline=pd.trampoline_task.load(Ordering::Acquire) as usize;
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
    let app=&mut apps[index];assert!(app.predecessor==0 && successor!=predecessor,"SUCCESSOR_BIND");
    // Program rows run side by side, each on its own key; others replace their predecessor.
    let program=sex_pdx::app_catalog::app_of(successor)==Some(sex_pdx::app_catalog::App::Program);
    assert!(program || app.pkey==old_key,"SUCCESSOR_KEY");
    let task=unsafe{&*(app.main as *const crate::scheduler::Task)};
    assert!(task.state.snapshot().phase==Phase::Constructed,"SUCCESSOR_ALREADY_ACTIVE");
    app.predecessor=predecessor;app.identity=next;
    DORMANT[index].store(true,Ordering::Release);
    if sex_pdx::app_catalog::app_of(successor)==Some(sex_pdx::app_catalog::App::Program){
        // Keep the loaded image, so a later run starts exactly as this one would.
        let pristine=crate::boot_memory::backing(app.count as u64*4096);
        for (i,page) in app.pages[..app.count].iter().enumerate(){
            crate::boot_memory::copy_frame(page.expected&PHYSICAL_MASK,pristine+i as u64*4096);
        }
        app.pristine=pristine;app.reusable=true;
        app.entry=task.context.rip;app.stack_top=task.context.rsp;
        assert!(app.count<=app.boot_pages.len(),"PROGRAM_BOOT_MANIFEST");
        app.boot_pages[..app.count].copy_from_slice(&app.pages[..app.count]);app.boot_count=app.count;
        app.image_pages=0;app.grown=0;
    }
    retire_private_pages(&app.pages[..app.count],
        |leaf|unsafe{core::ptr::read_volatile(leaf as *const u64)},
        |leaf|unsafe{core::ptr::write_volatile(leaf as *mut u64,0)},
        |va|crate::smp::tlb::flush(va)).expect("SUCCESSOR_RESERVE_LEAVES");
    if app.reusable{
        let (arena,pages)=crate::boot_memory::reserve_arena(app.pkey);
        app.arena=arena;app.arena_pages=pages;
    }
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
/// Compare descriptive catalog data with actual BOOT-owned subjects and held
/// grants before sealing or scheduling. Catalog lookup never grants authority.
pub fn audit_boot_catalog(coordinator:u32){
    boot();
    let rows=&sex_pdx::app_catalog::INCARNATIONS;
    crate::app_catalog_state::validate_layout(rows).expect("BOOT_CATALOG_LAYOUT");
    let apps=APPS.lock();
    assert_eq!(apps.iter().filter(|a|a.ready).count(),rows.len(),"BOOT_CATALOG_COUNT");
    let shell=crate::ipc::DOMAIN_REGISTRY.get(coordinator).expect("BOOT_CATALOG_COORDINATOR");
    let slot=|value:u64|u32::try_from(value).expect("BOOT_CATALOG_SLOT_RANGE");
    for row in rows{
        let index=apps.iter().position(|a|a.ready&&a.target==row.target).expect("BOOT_CATALOG_SUBJECT");
        let app=&apps[index];
        let pd=crate::ipc::DOMAIN_REGISTRY.get(row.target).expect("BOOT_CATALOG_DOMAIN");
        let main=pd.main_task.load(Ordering::Acquire);
        assert!(!main.is_null()&&main as usize==app.main,"BOOT_CATALOG_MAIN");
        let task=unsafe{&*main};
        assert_eq!(app.task,task.id,"BOOT_CATALOG_CAPTURED_TASK");
        let lifecycle=match shell.find_capability(slot(row.lifecycle_slot as u64)).map(|c|c.data){
            Some(CapabilityData::AppLifecycle(g))=>g,_=>panic!("BOOT_CATALOG_LIFECYCLE")};
        let self_catalog=match pd.find_capability(slot(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT as u64)).map(|c|c.data){
            Some(CapabilityData::AppLifecycle(g))=>g,_=>panic!("BOOT_CATALOG_SELF")};
        let domain=|at:u64|match shell.find_capability(slot(at)).map(|c|c.data){
            Some(CapabilityData::Domain(target))=>target,_=>panic!("BOOT_CATALOG_DOMAIN_ROUTE")};
        let activation=row.activation_slot.map(|at|match shell.find_capability(slot(at as u64)).map(|c|c.data){
            Some(CapabilityData::AppActivation(g))=>g,_=>panic!("BOOT_CATALOG_ACTIVATION")});
        // Capture predecessor task independently from the activation grant and
        // from its PD number. Task identity is a separate registered namespace.
        let predecessor_registered_task=if app.predecessor==0 {0} else {
            let predecessor=crate::ipc::DOMAIN_REGISTRY.get(app.predecessor).expect("BOOT_CATALOG_PREDECESSOR_DOMAIN");
            let predecessor_main=predecessor.main_task.load(Ordering::Acquire);
            let predecessor_app=apps.iter().find(|a|a.ready&&a.target==app.predecessor).expect("BOOT_CATALOG_PREDECESSOR_APP");
            assert!(!predecessor_main.is_null()&&predecessor_main as usize==predecessor_app.main,"BOOT_CATALOG_PREDECESSOR_MAIN");
            let predecessor_task=unsafe{&*predecessor_main};
            assert_eq!(predecessor_task.context.pd_id,u64::from(app.predecessor),"BOOT_CATALOG_PREDECESSOR_TASK_DOMAIN");
            assert!(core::ptr::eq(predecessor_task.context.pd_ptr,predecessor as *const _),"BOOT_CATALOG_PREDECESSOR_TASK_CONTEXT");
            assert_eq!(predecessor_app.task,predecessor_task.id,"BOOT_CATALOG_PREDECESSOR_CAPTURED_TASK");
            assert!(predecessor_task.id!=0,"BOOT_CATALOG_PREDECESSOR_TASK_ZERO");
            predecessor_task.id
        };
        let actual=crate::app_catalog_state::Observed{
            target:pd.id,task:task.id,
            task_domain:u32::try_from(task.context.pd_id).expect("BOOT_CATALOG_TASK_DOMAIN_RANGE"),
            task_domain_pointer_matches:core::ptr::eq(task.context.pd_ptr,pd as *const _),
            content_surface:app.surface,
            logical_app:app.identity.logical_app,incarnation:app.identity.number,
            predecessor:app.predecessor,predecessor_registered_task,coordinator:app.coordinator,
            dormant:DORMANT[index].load(Ordering::Acquire),
            dead:app.retired||TERMINAL[index].load(Ordering::Acquire)||task.state.snapshot().exited,
            shell_lifecycle:lifecycle,self_catalog,close_target:domain(row.close_slot),
            route_target:domain(row.route_slot),activation,
        };
        crate::app_catalog_state::validate(row,&actual,coordinator).expect("BOOT_CATALOG_ACTUAL_GRANTS");
        crate::serial_println!("[boot.app.catalog] target={} task={} surface={} logical={} incarnation={} predecessor={} lifecycle={} close={} route={} dormant={} grants_verified=1 data_only=1",
            row.target,task.id,app.surface,app.identity.logical_app,app.identity.number,app.predecessor,
            row.lifecycle_slot,row.close_slot,row.route_slot,actual.dormant as u8);
    }
    crate::serial_println!("[boot.app.catalog.complete] rows={} actual_grants=1 runtime_mint=0",rows.len());
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
    let result=(||{let g=grant(pd,slot)?;let apps=apps_lock!().ok_or(RETRY)?;
        let app=apps.iter().find(|a|a.ready&&a.target==g.target_pd).ok_or(DENIED)?;
        g.authorize(pd.id,app.target,app.boot_task,QUERY)?;
        if kind==1{return Ok([0,app.target as u64,app.task as u64,app.surface,app.coordinator as u64])}
        if kind!=0{return Err(INVALID)}dead(app)?;
        Ok([0,app.target as u64,app.task as u64,REQUIRED as u64,app.acked as u64])
    })();result.unwrap_or_else(|e|[e,0,0,0,0])
}
/// BSP serialized snapshot/preflight/commit. No runtime frame selection,
/// ELF loading, capability minting, or predecessor identity mutation.
pub fn activation(pd:&ProtectionDomain,slot:u64,kind:u64,len:u64)->[u64;5]{
    let result=(||{
        if kind>3{return Err(INVALID)}
        let slot=crate::capability_slot::decode_capability_slot(slot).ok_or(INVALID)?;
        let g=match pd.find_capability(slot).map(|c|c.data){Some(CapabilityData::AppActivation(g))=>g,_=>return Err(INVALID)};
        let mut apps=apps_lock!().ok_or(RETRY)?;
        let oi=apps.iter().position(|a|a.ready&&a.target==g.predecessor).ok_or(DENIED)?;
        let ni=apps.iter().position(|a|a.ready&&a.target==g.successor).ok_or(DENIED)?;
        let old=&apps[oi];let new=&apps[ni];
        g.authorize(pd.id,old.target,old.boot_task,new.target,new.boot_task,old.identity,new.identity)?;
        if new.predecessor!=old.target{return Err(DENIED)}
        let reply_of=|a:&App,state:u64|[0,a.target as u64,a.task as u64,a.surface,state];
        if kind==3{
            // A program from disk, on a row that has not started or has fully ended.
            if !new.reusable{return Err(DENIED)}
            let len=usize::try_from(len).map_err(|_|INVALID)?;
            if !new.activated{
                let ready=if old.reusable{old.activated}else{old.retired&&old.acked==REQUIRED&&dead(old).is_ok()};
                if !ready{return Err(RETRY)}
                let task=apps[ni].boot_task;
                load_program(&mut apps[ni],len,task)?;
                return Ok(reply_of(&apps[ni],1));
            }
            return rebirth(&mut apps,ni,Some(len)).map(|state|reply_of(&apps[ni],state));
        }
        if new.reusable&&new.activated{
            if new.rebirthing&&kind!=2{return Ok(reply_of(new,5))}
            if new.retired&&!new.rebirthing&&kind!=2{return Ok(reply_of(new,4))}
            if kind==2{return rebirth(&mut apps,ni,None).map(|state|reply_of(&apps[ni],state))}
        }
        // Once consumed, replay never revalidates or restores retired leaves.
        if new.activated{return Ok([0,new.target as u64,new.task as u64,new.surface,
            if TERMINAL[ni].load(Ordering::Acquire){3}else{2}])}
        // A program row can start once the row before it has started; it need not have ended.
        let ready=if old.reusable{old.activated}else{old.retired && old.acked==REQUIRED && dead(old).is_ok()};
        let reply=[0,new.target as u64,new.task as u64,new.surface,if ready{1}else{0}];
        if kind==0{return Ok(reply)}
        if kind==2{return Err(DENIED)}
        if !ready{return Err(RETRY)}
        // TLB retirement is valid only when it reaches every core ever started.
        // Fail closed if a core is present that shootdowns do not cover.
        if !crate::smp::shootdown_covers_all()
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
        let id=new.target;
        let count=restore_reserved_pages(&new.pages[..new.count],
            |leaf|unsafe{core::ptr::read_volatile(leaf as *const u64)},
            |leaf,value|unsafe{core::ptr::write_volatile(leaf as *mut u64,crate::program_tenancy::restored(id,value))},
            |_|{})?;
        crate::smp::tlb::flush_pages(count,new.pages[..new.count].iter().map(|p|p.va));
        crate::program_tenancy::adopt(id,&new.pages[..new.count]);
        // All fallible validation is above. IF is masked by the syscall
        // entry. Publish authority and receipt before scheduler publication.
        let new=&mut apps[ni];new.activated=true;
        DORMANT[ni].store(false,Ordering::Release);
        assert!(crate::scheduler::SCHEDULERS[0].activate_task(main),"SUCCESSOR_EXCLUSIVE_COMMIT");
        crate::serial_println!("[app.activation.committed] logical={} incarnation={} predecessor={} successor={} task={} surface={} pages={} runtime_alloc=0 runtime_mint=0 bsp=1",g.logical_app,g.incarnation,g.predecessor,g.successor,g.successor_task,new.surface,count);
        Ok([0,new.target as u64,new.task as u64,new.surface,2])
    })();result.unwrap_or_else(|e|[e,0,0,0,0])
}
/// A new run of a reusable row, in two steps so no service sees it live early.
/// First: fresh task id, zeroed-then-restored image, every service told. Then,
/// once all five have forgotten the old run: map the image and admit the task.
/// Check a program file, lay it out in the row's window, and point the row's task
/// at it. All checks precede the first write. The leaves stay clear until commit.
fn load_program(app:&mut App,len:usize,task:u32)->Result<(),u64>{
    use crate::program_image as image;
    if app.arena==0{return Err(DENIED)}
    let copy=crate::boot_memory::program_image(len).ok_or(INVALID)?;
    let elf:&[u8]=&copy;
    let parsed=image::parse(elf,(app.arena_pages*4096) as u64).map_err(|reason|{
        crate::serial_println!("[app.program.rejected] target={} bytes={} reason={:?}",app.target,len,reason);DENIED})?;
    let stack_low=app.stack_top.checked_sub(65536).ok_or(DENIED)?;
    // The retired run's leaves are clear and its frames returned, so nothing reads the
    // manifest meanwhile. Frames come zeroed, so nothing of another run survives.
    let mut count=0;let mut image_pages=0;
    for i in 0..app.boot_count{let page=app.boot_pages[i];if page.va>=stack_low&&page.va<app.stack_top{app.put(count,page);count+=1;}}
    let stack=count;
    let fail=|app:&mut App,count:usize,why:&str|{
        for page in &app.pages[stack..count]{crate::program_frames::give(page.expected&PHYSICAL_MASK);}
        app.count=0;
        crate::serial_println!("[app.program.rejected] target={} bytes={} reason={}",app.target,len,why);
    };
    for segment in &parsed.segments[..parsed.count]{
        let first=segment.window/4096;let last=(segment.window+segment.memory-1)/4096;
        for index in first..=last{
            let va=app.arena+index*4096;
            let (Some(leaf),Some(frame))=(crate::boot_memory::existing_leaf_address(va),crate::program_frames::take())else{
                fail(app,count,"no_memory");return Err(DENIED)};
            let mut flags=frame|5|((app.pkey as u64)<<59);
            if segment.writable{flags|=2}
            if !segment.executable{flags|=1<<63}
            app.put(count,PrivatePage{va,leaf,expected:flags});count+=1;
        }
        image_pages=image_pages.max(last as usize+1);
    }
    // Arena page index -> frame, for writing the file's bytes and relocations.
    let mut frames=alloc::vec![0u64;image_pages];
    for page in &app.pages[stack..count]{frames[((page.va-app.arena)/4096) as usize]=page.expected&PHYSICAL_MASK;}
    let write=|mut at:u64,mut bytes:&[u8]|->bool{
        while !bytes.is_empty(){
            let Some(&frame)=frames.get((at/4096) as usize)else{return false};
            if frame==0{return false}
            let n=bytes.len().min(4096-(at as usize&4095));
            crate::boot_memory::write_frames(frame+(at&4095),&bytes[..n]);
            at+=n as u64;bytes=&bytes[n..];
        }
        true
    };
    let arena=app.arena;
    for segment in &parsed.segments[..parsed.count]{
        let from=segment.offset as usize;
        if !write(segment.window,&elf[from..from+segment.file as usize]){fail(app,count,"segment");return Err(DENIED)}
    }
    for index in 0..parsed.rela_count{
        let Some((at,value))=image::relocation(elf,&parsed,index,arena)else{fail(app,count,"relocation");return Err(DENIED)};
        if !write(at,&value.to_le_bytes()){fail(app,count,"relocation");return Err(DENIED)}
    }
    app.count=count;app.image_pages=image_pages;app.grown=0;
    let entry=app.arena+parsed.entry;
    let target=crate::ipc::DOMAIN_REGISTRY.get(app.target).ok_or(DENIED)?;
    let main=target.main_task.load(Ordering::Acquire);
    unsafe{(*main).rearm(task,entry,app.stack_top,target);}
    crate::serial_println!("[app.program.loaded] target={} task={} bytes={} pages={} entry={:#x}",app.target,task,len,count,entry);
    Ok(())
}
/// More memory for the calling program's current run: the arena's next pages after its
/// image, on its own key, on zeroed frames lent for this run. Self-only: the caller's
/// domain is the row, and only a live current run may grow. All checks precede the
/// first write, and the pages join the run's manifest, so Close, a forced end and
/// retire take them as well.
pub fn grow(pd:&ProtectionDomain,want:u64)->[u64;3]{
    use sex_pdx::program_memory as m;
    let result=(||{
        if want>m::MAX_PAGES_PER_CALL{return Err(m::INVALID)}
        let mut apps=apps_lock!().ok_or(m::RETRY)?;
        let app=apps.iter_mut().find(|a|a.ready&&a.target==pd.id).ok_or(m::DENIED)?;
        if !app.reusable||!app.activated||app.arena==0||app.retired||app.rebirthing||app.forced{return Err(m::DENIED)}
        let main=pd.main_task.load(Ordering::Acquire);
        if main.is_null()||main as usize!=app.main{return Err(m::DENIED)}
        let task=unsafe{&*main};
        if task.id!=app.task||task.state.snapshot().exited{return Err(m::DENIED)}
        let base=app.arena+(app.image_pages*4096) as u64;
        let (first,want)=(app.image_pages+app.grown,want as usize);
        if want==0{return Ok([0,base,app.grown as u64])}
        if first+want>app.arena_pages{return Err(m::FULL)}
        let mut leaves=[0u64;m::MAX_PAGES_PER_CALL as usize];
        for (i,index) in (first..first+want).enumerate(){
            let leaf=crate::boot_memory::existing_leaf_address(app.arena+(index*4096) as u64).ok_or(m::DENIED)?;
            if unsafe{core::ptr::read_volatile(leaf as *const u64)}!=0{return Err(m::DENIED)}
            leaves[i]=leaf;
        }
        // All or none: the frames are taken before any page is mapped.
        let mut frames=[0u64;m::MAX_PAGES_PER_CALL as usize];
        for i in 0..want{
            match crate::program_frames::take(){Some(f)=>frames[i]=f,None=>{
                for &f in &frames[..i]{crate::program_frames::give(f);}
                return Err(m::FULL)}}
        }
        app.pages.reserve(want);
        for (i,index) in (first..first+want).enumerate(){
            let page=PrivatePage{va:app.arena+(index*4096) as u64,leaf:leaves[i],
                expected:frames[i]|7|((app.pkey as u64)<<59)|(1<<63)};
            let at=app.count;app.put(at,page);app.count+=1;
            // The leaf was clear, so no core holds a translation for it: a local flush is enough.
            crate::program_tenancy::grant(app.target,page);
            x86_64::instructions::tlb::flush(VirtAddr::new(page.va));
        }
        app.grown+=want;
        crate::serial_println!("[app.program.memory] target={} task={} granted={} held={} base={:#x} window_pages={} free_pages={} kernel_heap_used={}",app.target,app.task,want,app.grown,base,app.arena_pages,crate::program_frames::available(),crate::ALLOCATOR.lock().used());
        Ok([0,base,app.grown as u64])
    })();result.unwrap_or_else(|e|[e,0,0])
}
fn rebirth(apps:&mut [App;MAX_APPS],index:usize,image:Option<usize>)->Result<u64,u64>{
    let app=&mut apps[index];
    if !app.rebirthing {
        if !app.retired || app.acked!=REQUIRED {return Err(RETRY)}
        dead(app)?;
        if !crate::smp::shootdown_covers_all()
            || crate::core_local::CoreLocal::get().core_id!=0{return Err(DENIED)}
        app.generation=app.generation.checked_add(1).ok_or(DENIED)?;
        // Distinct from every boot task and every earlier run of any row, for good.
        let task=NEXT_RUN_TASK.fetch_add(1,Ordering::Relaxed);
        if task==u32::MAX{return Err(DENIED)}
        if let Some(len)=image{load_program(app,len,task)?;}
        else{
            let count=app.boot_count;app.pages.clear();app.pages.extend_from_slice(&app.boot_pages[..count]);app.count=count;
            app.image_pages=0;app.grown=0;
            for (i,page) in app.boot_pages[..count].iter().enumerate(){
                crate::boot_memory::copy_frame(app.pristine+i as u64*4096,page.expected&PHYSICAL_MASK);
            }
            let target=crate::ipc::DOMAIN_REGISTRY.get(app.target).ok_or(DENIED)?;
            let main=target.main_task.load(Ordering::Acquire);
            unsafe{(*main).rearm(task,app.entry,app.stack_top,target);}
        }
        app.task=task;app.acked=0;app.hinted=0;app.kicked=false;app.retired=false;app.rebirthing=true;app.forced=false;
        DORMANT[index].store(true,Ordering::Release);
        TERMINAL[index].store(false,Ordering::Release);
        crate::serial_println!("[app.rebirth.begin] target={} task={} generation={}",app.target,task,app.generation);
        return Ok(5);
    }
    if app.acked!=REQUIRED {return Ok(5)}
    let target=crate::ipc::DOMAIN_REGISTRY.get(app.target).ok_or(DENIED)?;
    let main=target.main_task.load(Ordering::Acquire);
    let id=app.target;
    let count=restore_reserved_pages(&app.pages[..app.count],
        |leaf|unsafe{core::ptr::read_volatile(leaf as *const u64)},
        |leaf,value|unsafe{core::ptr::write_volatile(leaf as *mut u64,crate::program_tenancy::restored(id,value))},
        |_|{})?;
    crate::smp::tlb::flush_pages(count,app.pages[..app.count].iter().map(|p|p.va));
    crate::program_tenancy::adopt(id,&app.pages[..app.count]);
    app.rebirthing=false;app.acked=0;
    DORMANT[index].store(false,Ordering::Release);
    assert!(crate::scheduler::SCHEDULERS[0].activate_task(main),"REBIRTH_EXCLUSIVE_COMMIT");
    crate::serial_println!("[app.rebirth.committed] target={} task={} generation={} pages={}",app.target,app.task,app.generation,count);
    Ok(2)
}
/// Shell ends a program run that ignores Close: its pages go, so it faults the next
/// time it runs and is ended there, and its cleanup then runs as for any exit. Only the
/// coordinator holding the row's lifecycle grant may, and only for program rows.
pub fn force_end(pd:&ProtectionDomain,slot:u64)->u64{
    let result=(||{let g=grant(pd,slot)?;let mut apps=apps_lock!().ok_or(RETRY)?;
        let app=apps.iter_mut().find(|a|a.ready&&a.target==g.target_pd).ok_or(DENIED)?;
        g.authorize(pd.id,app.target,app.boot_task,QUERY)?;
        if pd.id!=app.coordinator||!app.reusable{return Err(DENIED)}
        if app.forced||app.retired||app.rebirthing{return Ok(())}
        let id=app.target;crate::program_tenancy::release(id);
        for page in &app.pages[..app.count]{unsafe{core::ptr::write_volatile(page.leaf as *mut u64,0)};}
        crate::smp::tlb::flush_pages(app.count,app.pages[..app.count].iter().map(|p|p.va));
        app.forced=true;
        crate::serial_println!("[app.program.forced] target={} task={} pages={}",app.target,app.task,app.count);
        Ok(())})();result.map_or_else(|e|e,|()|0)
}
pub fn ack(pd:&ProtectionDomain,slot:u64,task:u64)->u64{
    let result=(||{let g=grant(pd,slot)?;let mut apps=apps_lock!().ok_or(RETRY)?;
        let app=apps.iter_mut().find(|a|a.ready&&a.target==g.target_pd).ok_or(DENIED)?;
        g.authorize(pd.id,app.target,app.boot_task,ACK)?;if task!=app.task as u64{return Err(DENIED)}if !app.rebirthing{dead(app)?;}
        let old=app.acked;app.acked|=g.ack_bit;
        if app.acked!=old{crate::serial_println!("[app.service.ack] target={} task={} holder={} role={} acked={} required={}",app.target,app.task,pd.id,g.ack_bit,app.acked,REQUIRED);}
        Ok(())})();result.map_or_else(|e|e,|()|0)
}
pub fn retire(pd:&ProtectionDomain,slot:u64)->[u64;4]{
    let result=(||{let g=grant(pd,slot)?;let mut apps=apps_lock!().ok_or(RETRY)?;
        let index=apps.iter().position(|a|a.ready&&a.target==g.target_pd).ok_or(DENIED)?;
        let app=&mut apps[index];g.authorize(pd.id,app.target,app.boot_task,RETIRE)?;
        if app.retired{return Ok([0,app.pool,app.count as u64*4096,app.task as u64])}
        dead(app)?;
        if app.acked!=REQUIRED || !crate::paging::target_cleanup_complete(app.target)?{return Err(RETRY)}
        TERMINAL[index].store(true,Ordering::Release);
        let id=app.target;crate::program_tenancy::release(id);
        // A forced run's leaves are already clear.
        let count=if app.forced {app.count} else {retire_private_pages(&app.pages[..app.count],
            |leaf|crate::program_tenancy::read(id,leaf),
            |leaf|unsafe{core::ptr::write_volatile(leaf as *mut u64,0)},
            |_|{})?};
        // One shootdown for the whole run, before any frame is reused.
        if !app.forced{crate::smp::tlb::flush_pages(app.count,app.pages[..app.count].iter().map(|p|p.va));}
        if app.reusable{
            // The row keeps its authority and its BOOT frames for the next run; frames
            // lent to this run's arena go back for any program to use.
            for i in 0..app.count{
                let (va,frame)=(app.pages[i].va,app.pages[i].expected&PHYSICAL_MASK);
                if app.in_arena(va){crate::program_frames::give(frame)}else{crate::boot_memory::zero_frame(frame)}
            }
            app.count=0;
            let target=crate::ipc::DOMAIN_REGISTRY.get(app.target).expect("APP_REGISTRY_CHANGED");
            while unsafe{(*target.message_ring).dequeue()}.is_some(){}
            while unsafe{(*target.signal_ring).dequeue()}.is_some(){}
            crate::ipc::messages::IpcReply::erase_queue(&mut target.incoming_replies.lock());
            target.reply_credits.lock().clear();
            app.retired=true;
            crate::serial_println!("[app.private.retired] target={} task={} pages={} reusable=1",app.target,app.task,count);
            return Ok([0,app.pool,0,app.task as u64]);
        }
        // No pointer-owning control object is freed. Late refs remain valid but
        // routing is fenced, and all private-payload consumers acknowledged.
        let target=crate::ipc::DOMAIN_REGISTRY.get(app.target).expect("APP_REGISTRY_CHANGED");
        let mut messages=0;while unsafe{(*target.message_ring).dequeue()}.is_some(){messages+=1;assert!(messages<=256,"APP_DRAIN_PRODUCER");}
        let mut signals=0;while unsafe{(*target.signal_ring).dequeue()}.is_some(){signals+=1;assert!(signals<=32,"APP_SIGNAL_PRODUCER");}
        crate::ipc::messages::IpcReply::erase_queue(&mut target.incoming_replies.lock());
        target.reply_credits.lock().clear();
        // The terminal incarnation's held authority is revoked, then checked
        // empty against the actual table; nothing can refill a sealed table.
        let revoked=unsafe{(*target.cap_table).revoke_all()};
        assert!(unsafe{(*target.cap_table).holds_none(|_|true)},"APP_CAPS_RETAINED");
        crate::serial_println!("[app.caps.revoked] target={} task={} revoked={} remaining=0",app.target,app.task,revoked);
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
        if app.ready && app.rebirthing {
            // Everyone has forgotten the old run: wake the coordinator to commit.
            if app.acked==REQUIRED && app.hinted&32==0 {
                let Some(pd)=crate::ipc::DOMAIN_REGISTRY.get(app.coordinator)else{continue};
                let msg=crate::ipc::messages::MessageType::IpcCall{func_id:sex_pdx::app_lifecycle::OP_APP_REBORN,arg0:app.target as u64,arg1:app.task as u64,arg2:0,caller_pd:0,caller_task:0};
                if unsafe{(*pd.message_ring).enqueue(msg)}.is_ok(){app.hinted|=32;crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire));}
                continue;
            }
            for recipient in 0..5 {
                let bit=1u8<<recipient;
                if app.acked&bit!=0 || (app.hinted&bit!=0 && now.wrapping_sub(app.last_hint[recipient])<16){continue}
                let Some(pd)=crate::ipc::DOMAIN_REGISTRY.get(app.services[recipient])else{continue};
                let msg=crate::ipc::messages::MessageType::IpcCall{func_id:sex_pdx::app_lifecycle::OP_APP_REBORN,arg0:app.target as u64,arg1:app.task as u64,arg2:0,caller_pd:0,caller_task:0};
                app.hinted|=bit;app.last_hint[recipient]=now;
                if unsafe{(*pd.message_ring).enqueue(msg)}.is_ok(){crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire));}
            }
            continue;
        }
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
