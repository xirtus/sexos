//! SexNet's interrupt-armed sleep bookkeeping (servers/sexnet/src/irq_wait.rs).
#[path="../servers/sexnet/src/irq_wait.rs"]mod irq_wait;
use irq_wait::{NetIrq,Verdict,SILENT_LIMIT};
#[test]fn unarmed_service_never_acknowledges_or_gives_up(){
 let mut n=NetIrq::default();assert!(n.may_arm());assert!(!n.took(1));n.slept(0);
 for _ in 0..100{assert_eq!(n.received(),Verdict::Keep);}assert!(!n.gave_up);
}
#[test]fn taken_bits_acknowledge_and_a_working_interrupt_is_never_declared_silent(){
 let mut n=NetIrq::default();n.armed();assert!(!n.may_arm());assert!(n.took(1));assert!(!n.took(0));assert_eq!(n.wakes,1);
 for _ in 0..100{n.slept(0);assert_eq!(n.received(),Verdict::Keep);}assert!(n.armed&&!n.gave_up);
}
#[test]fn frames_after_timed_out_sleeps_with_no_interrupt_ever_fall_back_to_polling(){
 let mut n=NetIrq::default();n.armed();
 for i in 1..SILENT_LIMIT{n.slept(0);assert_eq!(n.received(),Verdict::Keep);assert_eq!(n.missed,i);
  // A second frame in the same turn is not another miss.
  assert_eq!(n.received(),Verdict::Keep);assert_eq!(n.missed,i);}
 n.slept(0);assert_eq!(n.received(),Verdict::Silent);assert!(!n.armed&&n.gave_up&&!n.may_arm());
 n.reset();assert!(n.gave_up&&!n.armed);
}
#[test]fn a_sleep_ended_by_the_interrupt_is_not_a_miss(){
 let mut n=NetIrq::default();n.armed();n.slept(1);assert_eq!(n.received(),Verdict::Keep);assert_eq!(n.missed,0);
}
