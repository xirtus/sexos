// Child of the full production module, appended by the model gate only.
use super::*;
use crate::{capability::{ProtectionDomain,CapabilityData},wire::{PeerLife as L,PeerState}};
fn state(holder:&ProtectionDomain)->PeerState{peer_state(holder,6).unwrap()}
#[test]
fn cap_scoped_epoch_lifecycle_matrix_and_real_same_task_rearm(){
 let holder=ProtectionDomain::new(12,1200);let peer=ProtectionDomain::new(3,3003);let other=ProtectionDomain::new(99,9900);
 holder.caps.lock().extend([(6,CapabilityData::Domain(3)),(8,CapabilityData::Memory),(9,CapabilityData::Domain(99))]);
 let _second=ProtectionDomain::new(4,4004);
 capture(3,3);capture(4,4);capture(0,0);
 assert_eq!(SERVICE_IDS[0].load(Ordering::Acquire),3);assert_eq!(SERVICE_IDS[1].load(Ordering::Acquire),4);
 RESTARTS[0].store(2,Ordering::Release);RESTARTS[1].store(7,Ordering::Release);
 assert_eq!((restarts_of(3),restarts_of(4),restarts_of(0),restarts_of(99)),(2,7,0,0));
 {let _held=SERVICE[0].lock();assert_eq!(restarts_of(3),2,"same-task startup must not get a false first-boot count while restart lock is held");assert_eq!(peer_state(holder,6),Err(crate::wire::RETRY));assert_eq!(crate::peer_call(3,0,0),(crate::wire::OK,2,0));assert_eq!(crate::peer_call(12,6,0),(crate::wire::RETRY,0,0));}
 RESTARTS[0].store(0,Ordering::Release);
 assert_eq!(state(holder),PeerState{epoch:1,life:L::Starting});
 assert_eq!(crate::peer_call(12,6,0),(crate::wire::OK,1,L::Starting as u64));
 assert_eq!(crate::peer_call(12,0,99),(crate::wire::OK,0,0),"legacy self ignores reserved buf as before");
 assert_eq!(crate::peer_call(0,0,0),(crate::wire::OK,0,0));assert_eq!(crate::peer_call(0,6,0),(crate::wire::DENIED,0,0));
 for (caller,slot,buf,status) in [(12,6,1,crate::wire::INVALID),(12,(1u64<<32)|6,0,crate::wire::INVALID),(12,8,0,crate::wire::DENIED),(12,3,0,crate::wire::DENIED),(12,9,0,crate::wire::NOT_FOUND),(777,6,0,crate::wire::DENIED)]{
  assert_eq!(crate::peer_call(caller,slot,buf),(status,0,0),"syscall envelope must fail closed");
 }

 for raw in [0,1025,u32::MAX as u64+1,(1u64<<32)|6,u64::MAX]{assert_eq!(peer_state(holder,raw),Err(crate::wire::INVALID));}
 assert_eq!(peer_state(holder,3),Err(crate::wire::DENIED),"a raw PD is not a held route");
 assert_eq!(peer_state(holder,8),Err(crate::wire::DENIED));assert_eq!(peer_state(holder,9),Err(crate::wire::NOT_FOUND));
 let main=peer.main_task.load(Ordering::Acquire);unsafe{assert!((*main).state.activate());}
 assert_eq!(state(holder),PeerState{epoch:1,life:L::Live});
 {let _held=SERVICE[0].lock();assert_eq!(peer_state(holder,6),Err(crate::wire::RETRY));}
 request_end(3);assert_eq!(state(holder).life,L::Ending);assert!(END_REQUESTS.load(Ordering::Acquire)&(1<<3)!=0,"query consumed the end request");assert!(end_requested(3));
 unsafe{
  assert!(matches!((*main).state.try_admit(||Ok::<(),()>(())),crate::task_ownership::Admission::Published));assert!((*main).state.begin_dispatch(0));assert!((*main).state.attach(0));
  (*main).state.park_fault(0);
 }
 assert_eq!(state(holder).life,L::FaultBlocked);
 unsafe{(*main).state.exit();}assert_eq!(state(holder).life,L::Exited);
 unsafe{(*main).state=crate::task_ownership::TaskOwnership::new();(*main).state.activate();(*main).state.exit();}
 assert_eq!(state(holder).life,L::Dead);
 // poll_one is the real production restart path, with mapper/device effects doubled.
 let old_id=unsafe{(*main).id};peer.incoming_replies.lock().push_back(0x1234);peer.reply_credits.lock().push(7);
 poll_one(0);assert_eq!(unsafe{(*main).id},old_id);assert_eq!(old_id,3003);
 assert_eq!(state(holder),PeerState{epoch:2,life:L::Live});assert!(peer.incoming_replies.lock().is_empty());assert!(peer.reply_credits.lock().is_empty());
 unsafe{let pd=(*main).context.pd_ptr;(*main).context.pd_ptr=other;assert_eq!(peer_state(holder,6),Err(crate::wire::INVALID));(*main).context.pd_ptr=pd;let pd_id=(*main).context.pd_id;(*main).context.pd_id=99;assert_eq!(peer_state(holder,6),Err(crate::wire::INVALID));(*main).context.pd_id=pd_id;let id=(*main).id;(*main).id=0;assert_eq!(peer_state(holder,6),Err(crate::wire::INVALID));(*main).id=id;}
 peer.main_task.store(std::ptr::null_mut(),Ordering::Release);assert_eq!(peer_state(holder,6),Err(crate::wire::INVALID));peer.main_task.store(main,Ordering::Release);
 unsafe{(*main).state.exit();}RESTARTS[0].store(MAX_RESTARTS,Ordering::Release);LOOP[0].store(MAX_RESTARTS,Ordering::Release);poll_one(0);assert_eq!(state(holder),PeerState{epoch:9,life:L::GaveUp});assert!(!state(holder).live());
}
#[test]fn decoder_refuses_unknown_and_zero_epoch(){for life in 0..10{assert_eq!(PeerState::decode(0,life),Err(crate::wire::INVALID));}for life in [0,8,255,u64::MAX]{assert_eq!(PeerState::decode(1,life),Err(crate::wire::INVALID));}}
