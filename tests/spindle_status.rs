#![allow(dead_code)]
pub const ERR_SERVICE_NOT_READY:u64=2;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path="../crates/sex-pdx/src/spindle_status.rs"]mod wire;
use wire::*;
#[test]fn pending_status_coalesces_without_changing_identity(){let mut p=Publisher::new(17,17);p.update(State::Loading);let mut old=None;p.poll(|n|{old=Some(n);Err(ERR_SERVICE_NOT_READY)});assert!(p.pending());p.update(State::Saving);let mut r=Receiver::new(17,17);p.poll(|n|{assert_eq!(n.sequence,2);assert!(r.accept(17,true,n.args()));Ok(())});assert!(!p.pending());assert_eq!(r.state(),Some(State::Saving));assert!(!r.accept(17,true,old.unwrap().args()));}
#[test]fn exact_actual_sender_live_identity_sequence_and_enum_are_required(){let n=Notice{target:12,task:12,sequence:1,state:State::Ready};let mut r=Receiver::new(12,12);for(caller,live,args)in[(17,true,n.args()),(12,false,n.args()),(12,true,[17|17<<32,1,1]),(12,true,[12|12<<32,0,1]),(12,true,[12|12<<32,1,99])]{assert!(!r.accept(caller,live,args));}assert!(r.accept(12,true,n.args()));assert!(!r.accept(12,true,n.args()));r.retire();assert!(!r.accept(12,true,Notice{sequence:2,..n}.args()));assert_eq!(r.state(),None);}
#[test]fn permanent_transport_denial_and_close_fence_publication(){let mut p=Publisher::new(12,12);p.update(State::Uncertain);p.poll(|_|Err(99));assert!(!p.pending());p.refresh();p.update(State::Ready);p.poll(|_|panic!());let mut p=Publisher::new(17,17);p.update(State::Loading);p.cancel();p.update(State::Ready);assert!(!p.pending());}
#[test]fn verified_view_refresh_republishes_stable_status_with_a_new_sequence(){let mut p=Publisher::new(12,12);let mut seq=0;p.update(State::Ready);p.poll(|n|{seq=n.sequence;Ok(())});p.update(State::Ready);assert!(!p.pending());p.refresh();p.update(State::Ready);p.poll(|n|{assert_eq!(n.sequence,seq+1);Ok(())});}
#[test]fn probe_labels_have_the_same_exact_live_incarnation_boundary(){
 for state in[State::ProbeRunning,State::ProbeCancelling,State::ProbeUncertain,State::ProbeReady,State::ProbeDenied]{let n=Notice{target:12,task:12,sequence:1,state};let mut r=Receiver::new(12,12);assert!(!r.accept(17,true,n.args()));assert!(!r.accept(12,false,n.args()));assert!(r.accept(12,true,n.args()));r.retire();assert!(!r.accept(12,true,Notice{sequence:2,..n}.args()));}
}
