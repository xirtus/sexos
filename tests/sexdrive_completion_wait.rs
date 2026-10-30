// SPDX-License-Identifier: GPL-3.0-only
//! Actual completion policy with finite host CQ/clock/wait callbacks. These
//! tests do not execute device DMA or prove native scheduling authority.
#[path="../apps/sexdrive/src/completion_wait.rs"]mod completion_wait;
use completion_wait::{wait,Failure,MAX_TICKS,MAX_TURNS,POLL_QUANTUM};
use std::cell::Cell;
#[test]fn stale_f_record_becomes_exact_completion_only_after_cooperative_wait(){
 let cqe=Cell::new(1329u32);let ticks=Cell::new(7);let waits=Cell::new(0);
 let report=wait(1345,1,||(0x10002,cqe.get()),||ticks.get(),||{waits.set(waits.get()+1);ticks.set(ticks.get()+1);if waits.get()==3{cqe.set(1345|(1<<16));}Ok(())});
 assert_eq!(waits.get(),3);assert_eq!(report.polls,3*POLL_QUANTUM+1);let got=report.result.unwrap();assert_eq!(got.dw2,0x10002);assert_eq!(got.dw3,1345|(1<<16));
}
#[test]fn ready_and_error_status_cqes_both_require_exact_cid_and_phase(){
 for status in [0,2,0x102]{let mut waits=0;let report=wait(42,1,||(9,42|(1<<16)|(status<<17)),||0,||{waits+=1;Ok(())});assert!(report.result.is_ok());assert_eq!(waits,0);assert_eq!(report.polls,1);}
 for dw3 in [41|(1<<16),42,0]{let tick=Cell::new(0);let r=wait(42,1,||(0,dw3),||tick.get(),||{tick.set(MAX_TICKS);Ok(())});assert_eq!(r.result,Err(Failure::Deadline));assert_eq!(r.polls,POLL_QUANTUM);assert_eq!(r.last_dw3,dw3);}
}
#[test]fn deadline_and_rollback_are_checked_before_observing_after_wait(){
 for rollback in [false,true]{let tick=Cell::new(10);let complete=Cell::new(false);
  let r=wait(42,1,||{assert!(!complete.get(),"no late read after clock fence");(0,26)},||tick.get(),||{complete.set(true);tick.set(if rollback{9}else{10+MAX_TICKS});Ok(())});
  assert_eq!(r.result,Err(if rollback{Failure::ClockRollback}else{Failure::Deadline}));assert_eq!(r.polls,POLL_QUANTUM);
 }
}
#[test]fn stalled_clock_and_coalesced_wakes_have_a_separate_finite_turn_limit(){
 let mut pauses=0;let r=wait(42,1,||(0,26),||10,||{pauses+=1;Ok(())});
 assert_eq!(r.result,Err(Failure::TurnLimit));assert_eq!(r.turns,MAX_TURNS);assert_eq!(r.polls,MAX_TURNS*POLL_QUANTUM);assert_eq!(pauses,MAX_TURNS-1);
}
#[test]fn denied_or_full_style_wait_is_a_failure_without_another_poll(){
 for error in [9,u64::MAX]{let r=wait(42,1,||(0,26),||0,||Err(error));assert_eq!(r.result,Err(Failure::WaitDenied(error)));assert_eq!(r.polls,POLL_QUANTUM);assert_eq!(r.turns,1);}
}
#[test]fn clock_deadline_overflow_never_starts_an_unbounded_observation(){
 let r=wait(42,1,||panic!("must fail before polling"),||u64::MAX-1,||panic!("must not wait"));assert_eq!(r.result,Err(Failure::ClockOverflow));assert_eq!(r.polls,0);
}
