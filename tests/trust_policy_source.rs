// SPDX-License-Identifier: GPL-3.0-only
//! Actual pure source, readonly ABI and retained service machine. Kernel syscall
//! mechanics require the native sealed image; these tests do not pretend a host
//! fixture can establish live task authority or BOOT capability custody.
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[path="../crates/sex-pdx/src/trust_source.rs"]pub mod trust_source;
#[path="../crates/sex-pdx/src/trust_policy.rs"]pub mod trust_policy;
#[path="../servers/sexnet/src/trust_policy.rs"]mod broker;
use trust_source::{Identity,Grant,Info};
const HOLDER:Identity=Identity {pd:13,task:13};
const OWNER:Identity=Identity {pd:15,task:17};
const GRANT:Grant=Grant {holder:HOLDER,consumer:OWNER,profile:1};
fn metadata(owner:Identity)->Info {Info {consumer:owner,profile:trust_config::PROFILE,root_bytes:trust_config::ROOT_DER.len(),version:trust_config::VERSION}}
fn query(grant:Option<&Grant>,caller:Identity,kind:u64,args:[u64;3])->[u64;5]{trust_source::invoke(grant,caller,kind,args)}
fn forbidden()->Result<Info,u64>{panic!("rejected or duplicate request reached held source")}
fn receipt(m:&mut broker::Machine,owner:Identity,seq:u32)->Result<u32,u32>{let (actual,word)=m.pending_reply().unwrap();assert_eq!(actual,owner);m.reply_accepted();trust_policy::decode_reply(word,seq)}
#[test]fn source_exact_holder_and_all_reserved_arguments_precede_metadata(){
    use trust_source::*;
    for caller in [Identity {pd:0,task:13},Identity {pd:13,task:0},OWNER,Identity {pd:13,task:14}]{assert_eq!(query(Some(&GRANT),caller,INFO,[0;3]),[DENIED,0,0,0,0]);}
    assert_eq!(query(None,HOLDER,INFO,[0;3]),[DENIED,0,0,0,0]);
    for grant in [Grant {holder:OWNER,..GRANT},Grant {consumer:Identity {pd:0,task:15},..GRANT},Grant {consumer:Identity {pd:15,task:0},..GRANT}]{assert_eq!(query(Some(&grant),HOLDER,INFO,[0;3]),[DENIED,0,0,0,0]);}
    for kind in [0,2,INFO+(1<<32),u64::MAX]{assert_eq!(query(Some(&GRANT),HOLDER,kind,[0;3]),[DENIED,0,0,0,0]);}
    for at in 0..3{let mut args=[0;3];args[at]=1<<63;assert_eq!(query(Some(&GRANT),HOLDER,INFO,args),[DENIED,0,0,0,0]);}
    for profile in [0,2,u32::MAX]{assert_eq!(query(Some(&Grant {profile,..GRANT}),HOLDER,INFO,[0;3]),[UNAVAILABLE,0,0,0,0]);}
    assert_eq!(query(Some(&Grant {profile:2,..GRANT}),HOLDER,INFO,[1,0,0]),[DENIED,0,0,0,0]);
}
#[test]fn source_canonical_public_metadata_has_no_pointer_or_transport_selector(){
    let raw=query(Some(&GRANT),HOLDER,trust_source::INFO,[0;3]);
    assert_eq!(raw,[0,1,15|(17<<32),trust_config::ROOT_DER.len()as u64,trust_config::VERSION as u64]);
    assert!(!trust_config::ROOT_DER.is_empty());assert_eq!(Info::decode(raw),Ok(metadata(OWNER)));
    for _ in 0..20{assert_eq!(query(Some(&GRANT),HOLDER,trust_source::INFO,[0;3]),raw);}
}
#[test]fn source_decoder_rejects_every_noncanonical_word_and_dirty_error(){
    use trust_source::*;let good=query(Some(&GRANT),HOLDER,INFO,[0;3]);
    for (at,value)in[(0,1),(1,0),(1,1+(1<<32)),(2,15),(2,17<<32),(3,0),(3,good[3]+1),(4,0),(4,good[4]+(1<<32))]{let mut raw=good;raw[at]=value;assert_eq!(Info::decode(raw),Err(INVALID));}
    for error in [DENIED,UNAVAILABLE]{assert_eq!(Info::decode([error,0,0,0,0]),Err(error));for at in 1..5{let mut raw=[error,0,0,0,0];raw[at]=1;assert_eq!(Info::decode(raw),Err(INVALID));}}
    assert_eq!(Info::decode([INVALID,0,0,0,0]),Err(INVALID));
}
#[test]fn readonly_wire_requires_fullwidth_sequence_zero_reserved_and_known_profile(){
    use trust_policy::*;
    for seq in [1,MAX_SEQ]{let r=Request {seq};assert_eq!(Request::decode(r.encode()),Ok(r));assert_eq!(decode_reply(reply(seq,1),seq),Ok(1));for code in [INVALID,DENIED,STALE,BUSY,OFFLINE,EXHAUSTED]{assert_eq!(decode_reply(error(seq,code),seq),Err(code));}}
    for args in [[0,0,0],[MAX_SEQ as u64+1,0,0],[1+(1<<32),0,0],[1,1,0],[1,0,1<<63]]{assert_eq!(Request::decode(args),Err(INVALID));}
    for seq in [0,MAX_SEQ+1]{assert_eq!(decode_reply(reply(1,1),seq),Err(INVALID));assert!(std::panic::catch_unwind(||Request {seq}.encode()).is_err());}
    for word in [0,(0x7Cu64<<56)|1,(0x7Cu64<<56)|(1<<32),(0x7Cu64<<56)|(1<<32)|2,(0xECu64<<56)|(1<<32),(0xECu64<<56)|(1<<32)|9,(0x7Bu64<<56)|(1<<32)|1]{assert_eq!(decode_reply(word,1),Err(INVALID));}
    assert_eq!(decode_reply(reply(1,1),2),Err(STALE));assert!(std::panic::catch_unwind(||error(1,0)).is_err());assert!(std::panic::catch_unwind(||reply(1,2)).is_err());
}
#[test]fn broker_exact_source_consumer_and_defensive_info_checks(){
    use trust_policy::*;let mut m=broker::Machine::new();
    let other=Identity {pd:OWNER.pd,task:OWNER.task+1};
    for (n,(info,expected))in[(metadata(other),DENIED),(Info {profile:2,..metadata(OWNER)},INVALID),(Info {root_bytes:0,..metadata(OWNER)},INVALID),(Info {version:0,..metadata(OWNER)},INVALID),(metadata(Identity {pd:0,task:OWNER.task}),INVALID)].into_iter().enumerate(){let seq=n as u32+1;assert!(m.handle(OWNER,Request {seq},||Ok(info)));assert_eq!(receipt(&mut m,OWNER,seq),Err(expected));}
    assert!(m.handle(OWNER,Request {seq:6},||Info::decode(query(Some(&GRANT),HOLDER,trust_source::INFO,[0;3]))));assert_eq!(receipt(&mut m,OWNER,6),Ok(1));
}
#[test]fn broker_duplicate_pending_and_cached_queries_never_resample(){
    use trust_policy::*;let mut m=broker::Machine::new();let r=Request {seq:10};let mut calls=0;
    assert!(m.handle(OWNER,r,||{calls+=1;Ok(metadata(OWNER))}));let original=m.pending_reply();assert!(m.handle(OWNER,r,forbidden));assert_eq!(m.pending_reply(),original);assert!(!m.handle(OWNER,Request {seq:11},forbidden));assert_eq!(receipt(&mut m,OWNER,10),Ok(1));
    assert!(m.handle(OWNER,r,forbidden));assert_eq!(receipt(&mut m,OWNER,10),Ok(1));assert_eq!(calls,1);
    assert!(m.handle(OWNER,Request {seq:9},forbidden));assert_eq!(receipt(&mut m,OWNER,9),Err(STALE));
    assert!(m.handle(OWNER,Request {seq:11},||{calls+=1;Err(trust_source::UNAVAILABLE)}));assert_eq!(receipt(&mut m,OWNER,11),Err(OFFLINE));
    assert!(m.handle(OWNER,Request {seq:11},forbidden));assert_eq!(receipt(&mut m,OWNER,11),Err(OFFLINE));assert_eq!(calls,2);
}
#[test]fn broker_exact_reincarnation_retirement_preserves_other_owner_fences(){
    use trust_policy::*;let mut m=broker::Machine::new();let next=Identity {task:OWNER.task+1,..OWNER};
    assert!(m.handle(OWNER,Request {seq:1},||Ok(metadata(OWNER))));assert!(m.handle(next,Request {seq:1},||Ok(metadata(next))));
    assert!(!m.retire(Identity {task:99,..OWNER}));assert!(m.retire(OWNER));assert!(!m.retire(OWNER));assert_eq!(receipt(&mut m,next,1),Ok(1));
    assert!(m.handle(next,Request {seq:1},forbidden));assert_eq!(receipt(&mut m,next,1),Ok(1));
    // The caller's lifecycle attestation rejects terminal owners before entry;
    // this pure machine deliberately cannot invent that kernel fact.
}
#[test]fn broker_six_retained_owner_fences_fair_turns_no_eviction(){
    use trust_policy::*;let mut m=broker::Machine::new();
    for n in 1..=6{let owner=Identity {pd:n,task:n+20};assert!(m.handle(owner,Request {seq:1},||Ok(metadata(owner))));}
    assert!(!m.handle(OWNER,Request {seq:1},forbidden));
    for n in 1..=6{assert_eq!(m.pending_reply().unwrap().0,Identity {pd:n,task:n+20});m.reply_deferred();}
    for n in 1..=6{assert_eq!(receipt(&mut m,Identity {pd:n,task:n+20},1),Ok(1));}
    assert!(!m.handle(OWNER,Request {seq:1},forbidden));assert!(m.retire(Identity {pd:1,task:21}));assert!(m.handle(OWNER,Request {seq:1},||Ok(metadata(OWNER))));assert_eq!(receipt(&mut m,OWNER,1),Ok(1));
}
#[test]fn broker_errors_zero_identity_and_finite_sequence_do_not_fall_back(){
    use trust_policy::*;let mut m=broker::Machine::new();
    for owner in [Identity {pd:0,task:17},Identity {pd:15,task:0}]{assert!(!m.handle(owner,Request {seq:1},forbidden));}
    for seq in [0,MAX_SEQ+1]{assert!(!m.handle(OWNER,Request {seq},forbidden));}
    for (n,(source,expected))in[(trust_source::DENIED,DENIED),(trust_source::UNAVAILABLE,OFFLINE),(trust_source::INVALID,INVALID),(42,INVALID)].into_iter().enumerate(){let seq=n as u32+1;assert!(m.handle(OWNER,Request {seq},||Err(source)));assert_eq!(receipt(&mut m,OWNER,seq),Err(expected));}
    assert!(m.handle(OWNER,Request {seq:MAX_SEQ},||Ok(metadata(OWNER))));assert_eq!(receipt(&mut m,OWNER,MAX_SEQ),Ok(1));
    assert!(m.handle(OWNER,Request {seq:1},forbidden));assert_eq!(receipt(&mut m,OWNER,1),Err(STALE));assert!(m.handle(OWNER,Request {seq:MAX_SEQ},forbidden));assert_eq!(receipt(&mut m,OWNER,MAX_SEQ),Ok(1));
}

// Execute the actual kernel entry with a bounded model of its scheduler/capability
// dependencies. The model controls facts; it cannot prove native PKU isolation.
#[allow(dead_code)]mod ipc {pub const MAX_DOMAINS:usize=1024;}
#[allow(dead_code)]#[path="../kernel/src/capability_slot.rs"]mod capability_slot;
mod capability {
    use super::trust_source::Grant;
    use core::sync::atomic::AtomicPtr;
    #[derive(Clone,Copy)]pub enum CapabilityData {TrustPolicy(Grant),WrongKind}
    #[derive(Clone,Copy)]pub struct Capability {pub data:CapabilityData}
    pub struct ProtectionDomain {pub id:u32,pub main_task:AtomicPtr<crate::Task>,pub cap:Option<(u32,CapabilityData)>}
    impl ProtectionDomain {pub fn find_capability(&self,slot:u32)->Option<Capability>{self.cap.filter(|(at,_)|*at==slot).map(|(_,data)|Capability {data})}}
}
struct Context {pd_ptr:*const capability::ProtectionDomain}
struct Task {id:u32,context:Context}
mod scheduler {
    use core::sync::atomic::AtomicPtr;
    pub struct Scheduler {pub current_task:AtomicPtr<crate::Task>}
    pub static SCHEDULERS:[Scheduler;1]=[Scheduler {current_task:AtomicPtr::new(core::ptr::null_mut())}];
}
mod core_local {
    pub struct CoreLocal {pub core_id:u32}
    static CORE:CoreLocal=CoreLocal {core_id:0};
    impl CoreLocal {pub fn get()->&'static Self {&CORE}}
}
#[path="../kernel/src/trust_source.rs"]mod kernel_trust_source;
#[test]fn actual_kernel_entry_checks_full_slot_kind_current_task_main_task_and_context(){
    use core::sync::atomic::{AtomicPtr,Ordering};
    use capability::{CapabilityData,ProtectionDomain};
    let denied=[trust_source::DENIED,0,0,0,0];
    let mut pd=Box::new(ProtectionDomain {id:HOLDER.pd,main_task:AtomicPtr::new(core::ptr::null_mut()),cap:Some((62,CapabilityData::TrustPolicy(GRANT)))});
    let mut task=Box::new(Task {id:HOLDER.task,context:Context {pd_ptr:&*pd}});
    let taskptr=&mut*task as*mut Task;
    let call=|pd:&ProtectionDomain,slot|kernel_trust_source::invoke(pd,slot,trust_source::INFO,0,0,0);
    scheduler::SCHEDULERS[0].current_task.store(core::ptr::null_mut(),Ordering::Release);
    assert_eq!(call(&pd,62),denied);
    scheduler::SCHEDULERS[0].current_task.store(taskptr,Ordering::Release);
    assert_eq!(call(&pd,62),denied);
    pd.main_task.store(taskptr,Ordering::Release);
    assert_eq!(Info::decode(call(&pd,62)),Ok(metadata(OWNER)));
    for slot in [0,1,63,1025,62+(1<<32),62+(1<<63),u64::MAX]{assert_eq!(call(&pd,slot),denied);}
    pd.cap=Some((62,CapabilityData::WrongKind));assert_eq!(call(&pd,62),denied);
    pd.cap=Some((62,CapabilityData::TrustPolicy(GRANT)));
    task.id=HOLDER.task+1;assert_eq!(call(&pd,62),denied);task.id=HOLDER.task;
    task.context.pd_ptr=core::ptr::null();assert_eq!(call(&pd,62),denied);task.context.pd_ptr=&*pd;
    pd.id=HOLDER.pd+1;assert_eq!(call(&pd,62),denied);pd.id=HOLDER.pd;
    for kind in [0,2,1+(1<<32)]{assert_eq!(kernel_trust_source::invoke(&pd,62,kind,0,0,0),denied);}
    assert_eq!(kernel_trust_source::invoke(&pd,62,1,0,1<<63,0),denied);
    assert_eq!(Info::decode(call(&pd,62)),Ok(metadata(OWNER)));
    scheduler::SCHEDULERS[0].current_task.store(core::ptr::null_mut(),Ordering::Release);
}
