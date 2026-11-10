// SPDX-License-Identifier: GPL-3.0-only
//! NVMe enable/disable waits are bounded by CAP.TO in clock ticks, not poll counts.
#[path="../apps/sexdrive/src/controller_ready.rs"]mod controller_ready;
use controller_ready::*;
use std::cell::Cell;

#[test]fn slow_ssd_that_takes_three_seconds_to_stop_is_waited_for(){
    // CAP.TO=15 (7.5 s). RDY drops after 3000 ticks, far beyond any fixed poll count.
    let t=Cell::new(0u64);
    let r=wait(false,15,||if t.get()>=3000{0}else{RDY},||t.get(),||t.set(t.get()+1));
    assert_eq!(r.outcome,Outcome::Ready);assert!(r.elapsed>=2999 && r.elapsed<3001);
}
#[test]fn deadline_is_cap_to_in_half_seconds(){
    let t=Cell::new(0u64);
    let r=wait(true,2,||0,||t.get(),||t.set(t.get()+10));
    assert_eq!(r.outcome,Outcome::Deadline);assert_eq!(r.elapsed,1000);assert_eq!(deadline_ticks(0),500);
}
#[test]fn fatal_status_ends_an_enable_but_not_a_disable(){
    let r=wait(true,15,||CFS,||0,||panic!("no wait after CFS"));
    assert_eq!((r.outcome,r.polls),(Outcome::Fatal,1));
    let n=Cell::new(0);
    let r=wait(false,15,||{n.set(n.get()+1);if n.get()<10{CFS|RDY}else{CFS}},||0,||{});
    assert_eq!(r.outcome,Outcome::Ready);
}
#[test]fn device_gone_from_the_bus_is_never_ready(){
    let t=Cell::new(0u64);
    for want in [true,false]{t.set(0);let r=wait(want,1,||u32::MAX,||t.get(),||t.set(t.get()+100));assert_eq!(r.outcome,Outcome::Deadline);}
}
#[test]fn stalled_clock_still_ends(){
    let r=wait(true,1,||0,||7,||{});
    assert_eq!((r.outcome,r.polls),(Outcome::PollLimit,MAX_POLLS));
}
#[test]fn shutdown_waits_for_shst_complete_within_rtd3e(){
    // Drive reports RTD3E 2.5 s; SHST goes 01 (in progress) then 10 (complete) at 1200 ticks.
    assert_eq!(shutdown_ticks(2_500_000),2_500);assert_eq!(shutdown_ticks(0),1_000);assert_eq!(shutdown_ticks(10_000_000),MAX_SHUTDOWN_TICKS);
    let t=Cell::new(0u64);
    let r=wait_shutdown(2_500,||if t.get()>=1200{RDY|SHST_COMPLETE}else{RDY|(1<<2)},||t.get(),||t.set(t.get()+1));
    assert_eq!(r.outcome,Outcome::Ready);assert!(r.elapsed>=1199&&r.elapsed<1201);
}
#[test]fn shutdown_gives_up_at_the_limit_and_ignores_a_vanished_device(){
    let t=Cell::new(0u64);
    let r=wait_shutdown(1_000,||u32::MAX,||t.get(),||t.set(t.get()+10));
    assert_eq!((r.outcome,r.elapsed),(Outcome::Deadline,1_000));
}
