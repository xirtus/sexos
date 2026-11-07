// SPDX-License-Identifier: GPL-3.0-only
//! Program records: many small programs that hold no routes, made once at BOOT from
//! one template. BOOT creates each domain and task, so no authority appears later;
//! a load only supplies memory from a pool and a fresh task identity.
//!
//! All records share one hardware key, and only the record about to run has its
//! pages present. A sleeping record costs no key, and a key change costs one write
//! per page of the two records involved. One core only: smp.rs parks the others.
use crate::capability::ProtectionDomain;
use crate::scheduler::Task;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool,AtomicU32,AtomicU64,AtomicUsize,Ordering};
use sex_pdx::organs::RECORD_KEY;
use sex_pdx::program_store as store;
use x86_64::VirtAddr;

pub const BASE:u32=1024;
pub const CAPACITY:usize=4096;
const ARENA:u64=0x6000_0000_0000;
const SLICE:u64=2*1024*1024;
const PAGE:u64=4096;
const WINDOW_PAGES:usize=256;
/// A guard page, then the stack, then nothing up to the next record's window.
const STACK_FIRST:usize=WINDOW_PAGES+1;
const STACK_PAGES:usize=4;
const KSTACK_BYTES:usize=16*1024;
const PRESENT:u64=1;
const PTE_FRAME:u64=0x000f_ffff_ffff_f000;
/// Most records one start call makes: this runs with interrupts off.
const START_BATCH:u64=16;
const REAP_PER_TICK:usize=4;
const REAP_SCAN_PER_TICK:usize=64;
const WHEEL_SLOTS:usize=self_wait_slots();
const fn self_wait_slots()->usize{crate::scheduler::self_wait::MAX_TICKS as usize+1}
const WHEEL_WORDS:usize=CAPACITY/64;

#[derive(Clone,Copy,PartialEq,Eq)]
enum State{Free,Live}
struct Record{pd:*const ProtectionDomain,task:*mut Task,leaves:*mut u64,image_pages:u32,state:State,killed:bool}

static COUNT:AtomicUsize=AtomicUsize::new(0);
static mut RECORDS:*mut Record=core::ptr::null_mut();
static mut POOL:*mut u32=core::ptr::null_mut();
static POOL_FREE:AtomicUsize=AtomicUsize::new(0);
static POOL_TOTAL:AtomicUsize=AtomicUsize::new(0);
/// Index + 1 of the record whose pages are present.
static TENANT:AtomicUsize=AtomicUsize::new(0);
static LIVE:AtomicUsize=AtomicUsize::new(0);
static STARTED:AtomicU64=AtomicU64::new(0);
static ENDED:AtomicU64=AtomicU64::new(0);
static STOP:AtomicBool=AtomicBool::new(false);
static SWITCHES:AtomicU64=AtomicU64::new(0);
static SWITCH_CYCLES:AtomicU64=AtomicU64::new(0);
static KSTACK_PEAK:AtomicUsize=AtomicUsize::new(0);
static REAP_CURSOR:AtomicUsize=AtomicUsize::new(0);
static START_CURSOR:AtomicUsize=AtomicUsize::new(0);
static LAST_POLLED:AtomicU64=AtomicU64::new(0);
static NEXT_TASK:AtomicU32=AtomicU32::new(0x8000_0000);
static WHEEL:[[AtomicU64;WHEEL_WORDS];WHEEL_SLOTS]=[const{[const{AtomicU64::new(0)};WHEEL_WORDS]};WHEEL_SLOTS];

/// How many records and how much memory this machine can spare. Small machines keep
/// a token pool, so the same code runs everywhere.
const fn sizing(free_bytes:u64)->(usize,u64){
    let mib=1024*1024;
    if free_bytes<768*mib{return(16,4*mib)}
    let spare=free_bytes-512*mib;
    let pool=if spare>=1024*mib{1024*mib}else if spare>=512*mib{512*mib}else{256*mib};
    let records=pool/(128*1024);
    (if records>CAPACITY as u64{CAPACITY}else{records as usize},pool)
}

pub fn count()->u32{COUNT.load(Ordering::Relaxed) as u32}
pub fn is_record(pd:u64)->bool{pd>=BASE as u64&&pd<BASE as u64+COUNT.load(Ordering::Relaxed) as u64}
fn record(index:usize)->&'static mut Record{unsafe{&mut *RECORDS.add(index)}}
fn leaf(rec:&Record,page:usize)->*mut u64{unsafe{rec.leaves.add(page)}}
fn virtual_page(index:usize,page:usize)->u64{ARENA+index as u64*SLICE+page as u64*PAGE}

/// BOOT only: every domain, task and page table a record will ever use.
pub fn init(shell:u32){
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),crate::ipc::BootPhase::RegistryBuild as u8,"RECORDS_AFTER_BOOT");
    let free=crate::memory::allocator::debug_global_free_frames()*PAGE;
    let (count,pool_bytes)=sizing(free);
    let physical=crate::boot_memory::backing(pool_bytes);
    let frames=(pool_bytes/PAGE) as usize;
    let mut pool=alloc::vec::Vec::<u32>::with_capacity(frames);
    for i in 0..frames{pool.push(((physical+(i as u64)*PAGE)/PAGE) as u32);}
    unsafe{POOL=pool.leak().as_mut_ptr();}
    POOL_FREE.store(frames,Ordering::Release);POOL_TOTAL.store(frames,Ordering::Release);
    let table=Box::into_raw(Box::new(crate::capability::CapabilityTable::new()));
    let signal=Box::into_raw(Box::new(crate::ipc_ring::RingBuffer::new()));
    let message=Box::into_raw(Box::new(crate::ipc_ring::RingBuffer::new()));
    let mut records=alloc::vec::Vec::with_capacity(count);
    for index in 0..count{
        let id=BASE+index as u32;
        let pd=Box::into_raw(Box::new(ProtectionDomain::record(id,RECORD_KEY,table,signal,message)));
        let task=Box::into_raw(Box::new(Task::with_kernel_stack(id,0,0,unsafe{&*pd},true,KSTACK_BYTES)));
        unsafe{(*pd).main_task.store(task,Ordering::Release);}
        crate::ipc::DOMAIN_REGISTRY.insert(id,pd);
        let leaves=crate::boot_memory::reserve_table(virtual_page(index,0),RECORD_KEY,physical);
        records.push(Record{pd,task,leaves,image_pages:0,state:State::Free,killed:false});
    }
    unsafe{RECORDS=records.leak().as_mut_ptr();}
    COUNT.store(count,Ordering::Release);
    let shell=crate::ipc::DOMAIN_REGISTRY.get(shell).expect("RECORDS_NO_SHELL");
    shell.grant_capability(store::RECORD_SLOT_SHELL,crate::capability::CapabilityData::RecordPool);
    crate::serial_println!("[boot.records] capacity={} pool_bytes={} free_bytes={} kstack={} key={}",count,pool_bytes,free,KSTACK_BYTES,RECORD_KEY);
}

fn pool_take()->Option<u64>{
    let free=POOL_FREE.load(Ordering::Relaxed);
    if free==0{return None}
    POOL_FREE.store(free-1,Ordering::Relaxed);
    Some(unsafe{*POOL.add(free-1)} as u64*PAGE)
}
fn pool_give(frame:u64){
    let free=POOL_FREE.load(Ordering::Relaxed);
    assert!(free<POOL_TOTAL.load(Ordering::Relaxed),"RECORD_POOL_DOUBLE_FREE");
    unsafe{*POOL.add(free)=(frame/PAGE) as u32;}
    POOL_FREE.store(free+1,Ordering::Relaxed);
}
/// Every present-able page of a record, in the page table that already records them.
fn for_each_page(rec:&Record,mut f:impl FnMut(usize,*mut u64)){
    for page in (0..rec.image_pages as usize).chain(STACK_FIRST..STACK_FIRST+STACK_PAGES){f(page,leaf(rec,page));}
}
fn set_present(index:usize,present:bool){
    let rec=record(index);
    for_each_page(rec,|page,entry|unsafe{
        let value=core::ptr::read_volatile(entry);
        if value==0{return}
        core::ptr::write_volatile(entry,if present{value|PRESENT}else{value&!PRESENT});
        x86_64::instructions::tlb::flush(VirtAddr::new(virtual_page(index,page)));
    });
}
/// Called before any task runs. Only the record about to run has pages present.
pub fn before_dispatch(pd:u64){
    if !is_record(pd){return}
    let index=(pd-BASE as u64) as usize;
    if record(index).killed{evict();return}
    let tenant=TENANT.load(Ordering::Relaxed);
    if tenant==index+1{return}
    let began=unsafe{core::arch::x86_64::_rdtsc()};
    if tenant!=0{set_present(tenant-1,false);}
    set_present(index,true);
    TENANT.store(index+1,Ordering::Relaxed);
    SWITCHES.fetch_add(1,Ordering::Relaxed);
    SWITCH_CYCLES.fetch_add(unsafe{core::arch::x86_64::_rdtsc()}.wrapping_sub(began),Ordering::Relaxed);
}
fn evict(){
    let tenant=TENANT.swap(0,Ordering::Relaxed);
    if tenant!=0{set_present(tenant-1,false);}
}
/// Copy into a record's frames by its page table, as pages need not be contiguous.
fn write_window(rec:&Record,mut at:u64,mut bytes:&[u8]){
    while !bytes.is_empty(){
        let page=(at/PAGE) as usize;let inside=at%PAGE;
        let frame=unsafe{core::ptr::read_volatile(leaf(rec,page))}&PTE_FRAME;
        let take=bytes.len().min((PAGE-inside) as usize);
        crate::boot_memory::write_frames(frame+inside,&bytes[..take]);
        bytes=&bytes[take..];at+=take as u64;
    }
}
fn free_pages(rec:&mut Record){
    for_each_page(rec,|_,entry|unsafe{
        let value=core::ptr::read_volatile(entry);
        if value!=0{pool_give(value&PTE_FRAME);core::ptr::write_volatile(entry,0);}
    });
    rec.image_pages=0;
}
/// Lays out one record, or leaves it and the pool as they were.
fn start_one(index:usize,elf:&[u8],image:&crate::program_image::Image)->Result<(),u64>{
    let rec=record(index);
    let last=image.segments[..image.count].iter().map(|s|(s.window+s.memory).div_ceil(PAGE)).max().ok_or(1u64)? as usize;
    if last>WINDOW_PAGES{return Err(1)}
    let needed=image.segments[..image.count].iter().map(|s|(s.window+s.memory).div_ceil(PAGE)-s.window/PAGE).sum::<u64>() as usize+STACK_PAGES;
    if POOL_FREE.load(Ordering::Relaxed)<needed{return Err(2)}
    let base=virtual_page(index,0);
    rec.image_pages=last as u32;
    let key=(RECORD_KEY as u64)<<59;
    for segment in &image.segments[..image.count]{
        for page in (segment.window/PAGE)..(segment.window+segment.memory).div_ceil(PAGE){
            let frame=pool_take().expect("RECORD_POOL_COUNTED");
            crate::boot_memory::zero_frame(frame);
            let mut value=frame|4|key;
            if segment.writable{value|=2}
            if !segment.executable{value|=1<<63}
            unsafe{core::ptr::write_volatile(leaf(rec,page as usize),value);}
        }
    }
    for page in STACK_FIRST..STACK_FIRST+STACK_PAGES{
        let frame=pool_take().expect("RECORD_POOL_COUNTED");
        crate::boot_memory::zero_frame(frame);
        unsafe{core::ptr::write_volatile(leaf(rec,page),frame|4|2|key|(1<<63));}
    }
    for segment in &image.segments[..image.count]{
        let from=segment.offset as usize;
        write_window(rec,segment.window,&elf[from..from+segment.file as usize]);
    }
    for relocation in 0..image.rela_count{
        let (at,value)=crate::program_image::relocation(elf,image,relocation,base).ok_or(3u64)?;
        write_window(rec,at,&value.to_le_bytes());
    }
    let id=NEXT_TASK.fetch_add(1,Ordering::Relaxed);
    let stack_top=virtual_page(index,STACK_FIRST+STACK_PAGES)-16;
    unsafe{(*rec.task).rearm(id,base+image.entry,stack_top,&*rec.pd);}
    rec.state=State::Live;rec.killed=false;
    LIVE.fetch_add(1,Ordering::Relaxed);STARTED.fetch_add(1,Ordering::Relaxed);
    assert!(crate::scheduler::SCHEDULERS[0].activate_task(rec.task),"RECORD_ACTIVATE");
    Ok(())
}
fn start(len:u64,count:u64)->[u64;3]{
    let Some(elf)=crate::boot_memory::program_image(len as usize)else{return [1,0,0]};
    let image=match crate::program_image::parse(elf,WINDOW_PAGES as u64*PAGE){
        Ok(image)=>image,
        Err(reason)=>{crate::serial_println!("[records.rejected] bytes={} reason={:?}",len,reason);return [1,0,0]},
    };
    let total=COUNT.load(Ordering::Relaxed);
    let mut made=0;let mut cursor=START_CURSOR.load(Ordering::Relaxed);
    for _ in 0..total{
        if made==count.min(START_BATCH){break}
        cursor=(cursor+1)%total;
        if record(cursor).state!=State::Free{continue}
        match start_one(cursor,elf,&image){Ok(())=>made+=1,Err(_)=>{let rec=record(cursor);free_pages(rec);break}}
    }
    START_CURSOR.store(cursor,Ordering::Relaxed);
    STOP.store(false,Ordering::Relaxed);
    [0,made,LIVE.load(Ordering::Relaxed) as u64]
}
/// End every live record. A record that ignores the request loses its pages and
/// faults the next time it runs, so no program can refuse.
fn stop(force:bool)->[u64;3]{
    STOP.store(true,Ordering::Relaxed);
    let total=COUNT.load(Ordering::Relaxed);let mut marked=0;
    if force{evict();}
    for index in 0..total{
        let rec=record(index);
        if rec.state==State::Live{rec.killed=force;marked+=1;unsafe{crate::scheduler::wake_record(rec.task);}}
    }
    [0,marked,LIVE.load(Ordering::Relaxed) as u64]
}
fn census()->[u64;3]{
    let (started,ended,live)=(STARTED.load(Ordering::Relaxed),ENDED.load(Ordering::Relaxed),LIVE.load(Ordering::Relaxed));
    let (switches,cycles)=(SWITCHES.load(Ordering::Relaxed),SWITCH_CYCLES.load(Ordering::Relaxed));
    crate::serial_println!("[records.census] live={} started={} ended={} capacity={} pool_free={} pool_total={} switches={} switch_cycles={} kstack_peak={} tlb_shootdowns={}",
        live,started,ended,COUNT.load(Ordering::Relaxed),POOL_FREE.load(Ordering::Relaxed),POOL_TOTAL.load(Ordering::Relaxed),switches,cycles,KSTACK_PEAK.load(Ordering::Relaxed),crate::smp::tlb::sent());
    [0,live as u64,POOL_FREE.load(Ordering::Relaxed) as u64]
}
pub fn call(pd:&ProtectionDomain,op:u64,a:u64,b:u64)->[u64;3]{
    if op==store::RECORD_STOP_REQUESTED{return [0,(is_record(pd.id as u64)&&STOP.load(Ordering::Relaxed)) as u64,0]}
    if !matches!(pd.find_capability(store::RECORD_SLOT_SHELL as u32).map(|c|c.data),Some(crate::capability::CapabilityData::RecordPool)){return [1,0,0]}
    x86_64::instructions::interrupts::without_interrupts(||match op{
        store::RECORD_START=>start(a,b),
        store::RECORD_STOP=>stop(a!=0),
        store::RECORD_CENSUS=>census(),
        _=>[1,0,0],
    })
}
/// A record's own sleep deadline: wake it by bucket, never by scanning every record.
pub fn armed(pd:u64,at:u64){
    let index=(pd-BASE as u64) as usize;
    WHEEL[at as usize%WHEEL_SLOTS][index/64].fetch_or(1<<(index%64),Ordering::Relaxed);
}
/// Timer path. Costs nothing while no record lives.
pub fn poll(now:u64){
    if LIVE.load(Ordering::Relaxed)==0{LAST_POLLED.store(now,Ordering::Relaxed);return}
    let last=LAST_POLLED.swap(now,Ordering::Relaxed);
    let from=if now.wrapping_sub(last)>WHEEL_SLOTS as u64{now-WHEEL_SLOTS as u64+1}else{last+1};
    for tick in from..=now{
        for (word_index,word) in WHEEL[tick as usize%WHEEL_SLOTS].iter().enumerate(){
            let mut bits=word.swap(0,Ordering::Relaxed);
            while bits!=0{
                let index=word_index*64+bits.trailing_zeros() as usize;bits&=bits-1;
                if record(index).state==State::Live{unsafe{crate::scheduler::wake_record_if_due(record(index).task,now);}}
            }
        }
    }
    reap();
}
fn reap(){
    let total=COUNT.load(Ordering::Relaxed);let mut cursor=REAP_CURSOR.load(Ordering::Relaxed);let mut freed=0;
    for _ in 0..REAP_SCAN_PER_TICK.min(total){
        cursor=(cursor+1)%total;
        let rec=record(cursor);
        if rec.state!=State::Live{continue}
        let snapshot=unsafe{(*rec.task).state.snapshot()};
        if !snapshot.exited||snapshot.phase!=crate::scheduler::task_ownership::Phase::Dead||snapshot.owner_core.is_some(){continue}
        if TENANT.load(Ordering::Relaxed)==cursor+1{evict();}
        else{set_present(cursor,false);}
        note_kernel_stack(rec);
        free_pages(rec);
        rec.state=State::Free;rec.killed=false;
        unsafe{(*rec.pd).reply_credits.lock().clear();}
        LIVE.fetch_sub(1,Ordering::Relaxed);ENDED.fetch_add(1,Ordering::Relaxed);
        freed+=1;if freed==REAP_PER_TICK{break}
    }
    REAP_CURSOR.store(cursor,Ordering::Relaxed);
}
/// How deep a record's kernel stack has ever gone: bytes past the low end stay zero.
fn note_kernel_stack(rec:&Record){
    let top=unsafe{(*rec.task).kstack_top};
    let low=top-KSTACK_BYTES as u64;
    let bytes=unsafe{core::slice::from_raw_parts(low as *const u8,KSTACK_BYTES)};
    let untouched=bytes.iter().take_while(|b|**b==0).count();
    KSTACK_PEAK.fetch_max(KSTACK_BYTES-untouched,Ordering::Relaxed);
}
