// SPDX-License-Identifier: GPL-3.0-only
//! Read-only metadata from the exact typed BOOT trust grant. This syscall
//! provides no root pointer, network operation, mutable policy or runtime mint.
use core::sync::atomic::Ordering;
use crate::capability::{CapabilityData,ProtectionDomain};
use sex_pdx::trust_source as w;
pub fn invoke(pd:&ProtectionDomain,slot:u64,kind:u64,a:u64,b:u64,c:u64)->[u64;5] {
    let denied=[w::DENIED,0,0,0,0];
    let Some(slot)=crate::capability_slot::decode_capability_slot(slot)else{return denied;};
    let Some(CapabilityData::TrustPolicy(grant))=pd.find_capability(slot).map(|cap|cap.data)else{return denied;};
    let core=crate::core_local::CoreLocal::get();
    let Some(scheduler)=crate::scheduler::SCHEDULERS.get(core.core_id as usize)else{return denied;};
    let task=scheduler.current_task.load(Ordering::Acquire);
    if task.is_null()||task!=pd.main_task.load(Ordering::Acquire){return denied;}
    if unsafe{(*task).context.pd_ptr}!=pd as*const _{return denied;}
    let caller=w::Identity {pd:pd.id,task:unsafe{(*task).id}};
    w::invoke(Some(&grant),caller,kind,[a,b,c])
}
