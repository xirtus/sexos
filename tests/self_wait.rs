// SPDX-License-Identifier: GPL-3.0-only
//! Actual production deadline/inventory/ownership helpers; no x86 stack proof.
#[path="../kernel/src/self_wait.rs"]mod wait;
#[path="../kernel/src/task_ownership.rs"]mod own;
use wait::{Deadline,BootInventory};
use own::{TaskOwnership,Admission,ParkResult,Phase};
use std::sync::Arc;
fn attached()->TaskOwnership{let s=TaskOwnership::new();assert!(s.activate());assert_eq!(s.try_admit(||Ok::<(),()>(())),Admission::Published);assert!(s.begin_dispatch(0));assert!(s.attach(0));s}
#[test]fn interval_bounds_overflow_and_existing_arm_have_no_effect(){let d=Deadline::new();for(n,t)in[(0,0),(0,65),(u64::MAX,1),(u64::MAX-63,64)]{assert_eq!(d.arm(n,t),None);assert_eq!(d.take_due(u64::MAX),None);}assert_eq!(d.arm(100,2),Some(102));assert_eq!(d.arm(100,1),None);assert_eq!(d.take_due(101),None);assert_eq!(d.take_due(102),Some(102));assert_eq!(d.take_due(u64::MAX),None);}
#[test]fn cancel_prevents_old_expiry_and_later_arm_remains_exact(){let d=Deadline::new();assert_eq!(d.arm(20,2),Some(22));d.cancel();assert_eq!(d.take_due(22),None);assert_eq!(d.arm(22,64),Some(86));assert_eq!(d.take_due(85),None);assert_eq!(d.take_due(90),Some(86));}
#[test]fn concurrent_timer_claims_fire_once(){let d=Arc::new(Deadline::new());assert_eq!(d.arm(10,1),Some(11));let handles:Vec<_>=(0..16).map(|_|{let d=d.clone();std::thread::spawn(move||d.take_due(11).is_some()as usize)}).collect();assert_eq!(handles.into_iter().map(|h|h.join().unwrap()).sum::<usize>(),1);}
#[test]fn boot_inventory_is_finite_sealed_and_does_not_accept_a_foreign_pointer(){let table=BootInventory::<u8,2>::new();let(mut a,mut b,mut foreign)=(1u8,2u8,3u8);let(a,b,foreign)=(&mut a as *mut u8,&mut b as *mut u8,&mut foreign as *mut u8);let mut seen=Vec::new();assert!(!table.contains(a));assert!(table.register(0,a));assert!(!table.register(1,a));assert!(!table.register(0,b));assert!(!table.register(2,b));assert!(!table.register(1,std::ptr::null_mut()));table.visit(|p|seen.push(p));assert!(seen.is_empty());assert!(table.register(1,b));assert!(table.seal());assert!(!table.seal());assert!(!table.register(0,foreign));assert!(!table.contains(foreign));assert!(table.contains(a)&&table.contains(b));table.visit(|p|seen.push(p));assert_eq!(seen,vec![a,b]);}
#[test]fn deadline_at_each_switch_boundary_preserves_one_owner_and_pending_wake(){for wake_at in 0..=7{let s=attached();let d=Deadline::new();assert_eq!(d.arm(100,2),Some(102));let mut token=false;for boundary in 0..=7{if boundary==wake_at{assert_eq!(d.take_due(102),Some(102));s.wake();}match boundary{0=>{s.park(0);},1=>{assert!(s.begin_switch_out(0));},2=>{assert!(unsafe{s.release_after_stack_switch(0)});},3=>{token=s.try_admit(||Ok::<(),()>(()))==Admission::Published;},4=>{if !token{token=s.try_admit(||Ok::<(),()>(()))==Admission::Published;}},5=>{if !token{token=s.try_admit(||Ok::<(),()>(()))==Admission::Published;}},6=>{if !token{token=s.try_admit(||Ok::<(),()>(()))==Admission::Published;}if token{assert!(s.begin_dispatch(0));assert!(s.attach(0));}},_=>{}}
 if boundary<2{assert!(matches!(s.snapshot().phase,Phase::Attached|Phase::SwitchingOut));assert_eq!(s.try_admit::<()>(||panic!("published before stack release")),Admission::NotRunnable);}}
 if !token{assert_eq!(s.try_admit(||Ok::<(),()>(())),Admission::Published);assert!(s.begin_dispatch(0));assert!(s.attach(0));}
 assert_eq!(s.snapshot().phase,Phase::Attached);if s.snapshot().pending{assert_eq!(s.park(0),ParkResult::WakeConsumed);}assert!(!s.snapshot().blocked);assert_eq!(d.take_due(102),None);}}
#[test]fn timer_after_exit_does_not_resurrect_terminal_identity(){let s=attached();let d=Deadline::new();assert_eq!(d.arm(10,1),Some(11));s.exit();assert!(d.take_due(11).is_some());s.wake();assert!(s.snapshot().exited);assert_eq!(s.try_admit::<()>(||panic!("dead task admitted")),Admission::NotRunnable);}

#[test]fn deadline_wake_cannot_authorize_unresolved_fault_retry(){let s=attached();let d=Deadline::new();assert_eq!(d.arm(10,1),Some(11));assert_eq!(s.park_fault(0),ParkResult::Blocked);assert!(d.take_due(11).is_some());s.wake();assert!(!s.resume_attached(0));assert!(s.snapshot().fault_blocked);assert_eq!(s.try_admit::<()>(||panic!("unresolved fault admitted")),Admission::NotRunnable);}
#[test]fn claimed_old_expiry_does_not_remove_a_later_arm(){let d=Deadline::new();assert_eq!(d.arm(10,1),Some(11));assert_eq!(d.take_due(11),Some(11));assert_eq!(d.arm(12,2),Some(14));assert_eq!(d.take_due(13),None);assert_eq!(d.take_due(14),Some(14));}

#[test]
fn boot_main_capacity_is_literal_64() {
    // Independent reviewed bound, deliberately not derived from the constant.
    assert_eq!(wait::BOOT_MAIN_CAPACITY, 64);
    let table=BootInventory::<u64,{wait::BOOT_MAIN_CAPACITY}>::new();
    let mut tasks=Box::new([0u64;65]);
    for index in [31,32,63] {
        tasks[index]=10_000+index as u64;
        assert!(table.register(index,&mut tasks[index]));
    }
    assert!(!table.register(64,&mut tasks[64]));
    assert!(!table.register(0,std::ptr::null_mut()));
    assert!(!table.register(0,&mut tasks[31]));
    assert!(!table.register(31,&mut tasks[0]));
    assert!(table.seal());
    assert!(!table.register(0,&mut tasks[0]));
    assert!(!table.seal());
    for index in [31,32,63] { assert!(table.contains(&mut tasks[index])); }
    assert!(!table.contains(&mut tasks[64]));
}

#[test]
fn all_64_registered_pointers_are_visited_once_in_slot_order() {
    let table=BootInventory::<u64,{wait::BOOT_MAIN_CAPACITY}>::new();
    let mut tasks=Box::new([0u64;65]);
    let mut expected=Vec::new();
    for index in 0..64 {
        tasks[index]=70_000+index as u64*10;
        let pointer=&mut tasks[index] as *mut u64;
        assert!(table.register(index,pointer));expected.push(pointer);
    }
    assert!(!table.register(64,&mut tasks[64]));
    table.visit(|_|panic!("unsealed inventory published"));
    assert!(table.seal());
    let mut actual=Vec::new();table.visit(|pointer|actual.push(pointer));
    assert_eq!(actual.len(),64);assert_eq!(actual,expected);
    assert_eq!(actual.iter().copied().collect::<std::collections::HashSet<_>>().len(),64);
    for pointer in actual { assert!(table.contains(pointer)); }
}

#[test]
fn sparse_inventory_visits_only_actual_registered_upper_slots() {
    let table=BootInventory::<u64,{wait::BOOT_MAIN_CAPACITY}>::new();
    let mut tasks=Box::new([101u64,202,303,404]);
    // Register out of order; iteration must follow slots, not insertion order.
    assert!(table.register(63,&mut tasks[2]));
    assert!(table.register(31,&mut tasks[0]));
    assert!(table.register(32,&mut tasks[1]));
    assert!(table.seal());
    let expected=vec![&mut tasks[0] as *mut u64,&mut tasks[1],&mut tasks[2]];
    let mut actual=Vec::new();table.visit(|pointer|{assert!(!pointer.is_null());actual.push(pointer)});
    assert_eq!(actual,expected);
    assert!(!table.contains(&mut tasks[3]));
}

struct TimedOwner { deadline:Deadline, state:TaskOwnership }
#[test]
fn upper_slots_preserve_cancellation_single_wake_and_terminal_custody() {
    for index in [31,32,63] {
        for mode in 0..3 {
            let mut task=Box::new(TimedOwner{deadline:Deadline::new(),state:attached()});
            let pointer=&mut *task as *mut TimedOwner;
            let table=BootInventory::<TimedOwner,{wait::BOOT_MAIN_CAPACITY}>::new();
            assert!(table.register(index,pointer));assert!(table.seal());
            assert_eq!(task.deadline.arm(100,64),Some(164));
            assert_eq!(task.state.park(0),ParkResult::Blocked);
            assert!(task.state.begin_switch_out(0));
            assert!(unsafe{task.state.release_after_stack_switch(0)});
            if mode==1 { task.deadline.cancel(); }
            // Deliberately retain a due deadline for the exit case: even a
            // late claimed timer cannot resurrect this terminal identity.
            if mode==2 { task.state.exit(); }
            let mut fired=0;
            for now in [163,164,164,200] {
                table.visit(|p|unsafe{
                    assert_eq!(p,pointer);
                    if (*p).deadline.take_due(now).is_some(){fired+=1;(*p).state.wake();}
                });
            }
            assert_eq!(fired,if mode==1{0}else{1});
            if mode==0 {
                assert_eq!(task.state.try_admit(||Ok::<(),()>(())),Admission::Published);
                assert_eq!(task.state.try_admit::<()>(||panic!("duplicate token")),Admission::NotRunnable);
                assert!(task.state.begin_dispatch(0));assert!(task.state.attach(0));
                assert_eq!(task.state.snapshot().phase,Phase::Attached);
            } else {
                assert_eq!(task.state.try_admit::<()>(||panic!("cancelled/exited task admitted")),Admission::NotRunnable);
                if mode==2 {assert!(task.state.snapshot().exited);}
            }
        }
    }
}
