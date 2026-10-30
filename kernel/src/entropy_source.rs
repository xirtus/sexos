// SPDX-License-Identifier: GPL-3.0-only
//! Bounded hardware mechanics behind an exact typed BOOT grant. Seed health,
//! client custody and any cryptographic construction belong to the service.
use core::sync::atomic::{AtomicU64,Ordering};
use crate::capability::{CapabilityData,ProtectionDomain};
use sex_pdx::entropy_source as w;

fn supported()->bool{
    // RDSEED without the feature would #UD. No raw instruction is reached
    // unless both the maximum CPUID leaf and its exact feature bit exist.
    unsafe{core::arch::x86_64::__cpuid(0).eax>=7
        && core::arch::x86_64::__cpuid_count(7,0).ebx&(1<<18)!=0}
}
fn sample()->Option<u64>{
    let word:u64;let carry:u8;
    unsafe{core::arch::asm!("rdseed {word}","setc {carry}",word=out(reg)word,carry=out(reg_byte)carry,options(nomem,nostack));}
    if carry==1{Some(word)}else{None}
}
static SUCCESS:AtomicU64=AtomicU64::new(0);

pub fn invoke(pd:&ProtectionDomain,slot:u64,kind:u64,a:u64,b:u64,c:u64)->[u64;5]{
    let denied=[w::DENIED,0,0,0,0];
    let Some(slot)=crate::capability_slot::decode_capability_slot(slot)else{return denied;};
    let Some(CapabilityData::EntropySource(grant))=pd.find_capability(slot).map(|cap|cap.data)else{return denied;};
    let core=crate::core_local::CoreLocal::get();
    let Some(scheduler)=crate::scheduler::SCHEDULERS.get(core.core_id as usize)else{return denied;};
    let task=scheduler.current_task.load(Ordering::Acquire);
    if task.is_null()||task!=pd.main_task.load(Ordering::Acquire){return denied;}
    if unsafe{(*task).context.pd_ptr}!=pd as*const _{return denied;}
    let caller=w::Identity{pd:pd.id,task:unsafe{(*task).id}};
    let result=w::invoke(Some(&grant),caller,kind,[a,b,c],supported,sample);
    if kind==w::READ&&result[0]==0{
        let count=SUCCESS.fetch_add(1,Ordering::Relaxed)+1;
        if count<=4||count.is_power_of_two(){
            crate::serial_println!("[entropy.source.sample] pd={} task={} slot={} source=rdseed attempts={} count={} private=1",pd.id,caller.task,slot,result[2],count);
        }
    }
    result
}
