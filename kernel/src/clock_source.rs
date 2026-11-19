// SPDX-License-Identifier: GPL-3.0-only
//! Exact typed BOOT clock mechanics for the named trusted q35 CMOS-UTC
//! profile. Only the index port is written; time/control/IRQ registers are
//! never written and interrupt-clear registerC is never read. This profile
//! selects NMI-enabled indexes, not an inferred/read-back mask (q35 reads
//! index port0x70 as0xff). It does not qualify physical RTC/NMI/clock integrity.
use core::sync::atomic::{AtomicU64,Ordering};
use crate::capability::{CapabilityData,ProtectionDomain};
use sex_pdx::clock_source as w;
static CMOS:spin::Mutex<()>=spin::Mutex::new(());
static SAMPLES:AtomicU64=AtomicU64::new(0);

unsafe fn register(index:u8)->u8{
    let value:u8;
    core::arch::asm!("out dx, al",in("dx")0x70u16,in("al")index,options(nomem,nostack,preserves_flags));
    core::arch::asm!("in al, dx",in("dx")0x71u16,out("al")value,options(nomem,nostack,preserves_flags));
    value
}
unsafe fn snapshot()->w::Raw{
    let a=register(0x0a);let b=register(0x0b);let d=register(0x0d);
    let sec=register(0x00);
    let mut raw=w::Raw{sec,min:register(0x02),hour:register(0x04),
        day:register(0x07),month:register(0x08),year:register(0x09),century:register(0x32),
        status_a:a,status_b:b,status_d:d};
    if register(0x00)!=sec||register(0x0a)!=a{raw.status_a=0x80;}
    if register(0x0b)!=b{raw.status_b=0x80;}
    raw
}

pub fn invoke(pd:&ProtectionDomain,slot:u64,kind:u64,a:u64,b:u64,c:u64)->[u64;5]{
    let denied=[w::DENIED,0,0,0,0];
    let Some(slot)=crate::capability_slot::decode_capability_slot(slot)else{return denied;};
    let Some(CapabilityData::ClockSource(grant))=pd.find_capability(slot).map(|cap|cap.data)else{return denied;};
    let core=crate::core_local::CoreLocal::get();
    let Some(scheduler)=crate::scheduler::SCHEDULERS.get(core.core_id as usize)else{return denied;};
    let task=scheduler.current_task.load(Ordering::Acquire);
    if task.is_null()||task!=pd.main_task.load(Ordering::Acquire){return denied;}
    if unsafe{(*task).context.pd_ptr}!=pd as*const _{return denied;}
    let caller=w::Identity{pd:pd.id,task:unsafe{(*task).id}};
    let mut contended=false;
    let mut result=w::invoke(Some(&grant),caller,kind,[a,b,c],||{
        let Some(_guard)=CMOS.try_lock()else{contended=true;return None;};
        Some(unsafe{(snapshot(),snapshot())})
    });
    if result[0]==w::UNAVAILABLE&&contended{result=[w::BUSY,0,0,0,0];}
    if kind==w::READ&&result[0]==0{
        let n=SAMPLES.fetch_add(1,Ordering::Relaxed)+1;
        crate::serial_println!("[clock.source.snapshot] pd={} task={} slot={} seconds={} attempts={} precision=1 profile=q35_cmos_utc ordinal={}",pd.id,caller.task,slot,result[1],result[2],n);
    }
    result
}

/// Civil time for every task ([`w::civil`]): the last CMOS reading plus the ticks
/// since it, read again once a minute. `(reading ms, tick at reading)`, 0 when none.
static CIVIL:spin::Mutex<(u64,u64)>=spin::Mutex::new((0,0));
/// How far civil time is set from the CMOS reading (ms), by the time keeper.
static OFFSET:core::sync::atomic::AtomicI64=core::sync::atomic::AtomicI64::new(0);
/// The one domain that may set civil time (sexnet, which asks the network); 0: none.
pub static KEEPER:core::sync::atomic::AtomicU32=core::sync::atomic::AtomicU32::new(0);
pub fn civil_ms()->Result<u64,u64>{Ok(cmos_ms()?.saturating_add_signed(OFFSET.load(Ordering::Acquire)))}
/// Civil time becomes `ms`: the CMOS stays as it is, every later reading is offset.
pub fn set_civil(caller:u32,ms:u64)->u64{
    let keeper=KEEPER.load(Ordering::Acquire);
    if keeper==0||caller!=keeper{return w::DENIED;}
    if !(w::MIN_SECONDS*1000..=w::MAX_SECONDS*1000).contains(&ms){return w::INVALID;}
    let Ok(base)=cmos_ms()else{return w::BUSY};
    let offset=ms as i64-base as i64;let before=OFFSET.swap(offset,Ordering::AcqRel);
    crate::serial_println!("[clock.civil.set] pd={} ms={} step_ms={} offset_ms={}",caller,ms,offset-before,offset);
    0
}
fn cmos_ms()->Result<u64,u64>{
    let now=crate::interrupts::TICKS.load(Ordering::Relaxed);
    let mut held=CIVIL.lock();
    if held.0==0||now.saturating_sub(held.1)>=w::civil::RESAMPLE_MS{
        let fresh={
            let Some(_guard)=CMOS.try_lock()else{
                return if held.0!=0{Ok(held.0+now.saturating_sub(held.1))}else{Err(w::BUSY)};
            };
            w::sample(||Some(unsafe{(snapshot(),snapshot())}))
        };
        match fresh{
            Ok(s)=>*held=(s.seconds*1000,now),
            Err(e) if held.0==0=>return Err(e),
            Err(_)=>{}
        }
    }
    Ok(held.0+now.saturating_sub(held.1))
}
