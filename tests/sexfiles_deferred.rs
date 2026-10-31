#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path = "../crates/sex-pdx/src/document.rs"] pub mod document;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"]
pub mod app_lifecycle;
#[path = "../servers/sexfiles/src/defer_queue.rs"]
mod queue;
use app_lifecycle::OP_APP_EXIT;
#[test]
fn full_ordinary_backlog_cannot_consume_kernel_terminal_capacity() {
    let mut q = queue::Deferred::new();
    let requests: Vec<_> = (0..8).map(|i| (0x4D, 9, i, 0, 0)).collect();
    for &m in &requests {
        assert!(q.push(m));
    }
    assert!(!q.push((0x4D, 16, 99, 0, 0)));
    for pd in [9, 12, 15, 16, 17, 18] {
        assert!(q.push((OP_APP_EXIT, 0, pd, pd, 0)));
    }
    assert_eq!(q.len(), 14);
    // Replayed hints coalesce and cannot crowd out another terminal subject.
    for _ in 0..100 {
        assert!(q.push((OP_APP_EXIT, 0, 9, 9, 0)));
    }
    for pd in [9, 12, 15, 16, 17, 18] {
        assert_eq!(q.pop(), Some((OP_APP_EXIT, 0, pd, pd, 0)));
    }
    for m in requests {
        assert_eq!(q.pop(), Some(m));
    }
    assert_eq!(q.pop(), None);
}
#[test]
fn forged_or_wide_exit_payloads_have_no_priority_and_queue_is_reusable() {
    let mut q = queue::Deferred::new();
    let fake = (OP_APP_EXIT, 3, 9, 9, 0);
    assert!(q.push(fake));
    assert!(q.push((OP_APP_EXIT, 0, 16, 16, 0)));
    assert_eq!(q.pop(), Some((OP_APP_EXIT, 0, 16, 16, 0)));
    assert_eq!(q.pop(), Some(fake));
    for m in [
        (OP_APP_EXIT, 0, 0, 9, 0),
        (OP_APP_EXIT, 0, 9, 0, 0),
        (OP_APP_EXIT, 0, 1u64 << 32, 9, 0),
        (OP_APP_EXIT, 0, 9, 9, 1),
    ] {
        assert!(q.push(m));
        assert!(q.push((OP_APP_EXIT, 0, 9, 9, 0)));
        assert_eq!(q.pop(), Some((OP_APP_EXIT, 0, 9, 9, 0)));
        assert_eq!(q.pop(), Some(m));
    }
    assert_eq!(q.len(), 0);
}

#[test]
fn ordinary_pressure_cannot_steal_any_incarnation_document_window(){
 let mut q=queue::Deferred::new();let mut expected=vec![];
 for i in 0..8{let m=(0x99,12,i,0,0);assert!(q.push(m));expected.push(m);}
 for seq in 1..=4{for pd in [9,16,18]{let r=document::Request{seq,kind:document::READ,object:3,offset:(seq-1)*4,a:4,b:0};let m=(document::OP,pd,r.header(),r.a,r.b);assert!(q.push(m));expected.push(m);}}
 assert_eq!(q.len(),20);
 for pd in [9,16,18]{let r=document::Request{seq:5,kind:document::READ,object:3,offset:16,a:4,b:0};assert!(!q.push((document::OP,pd,r.header(),r.a,r.b)));}
 assert!(!q.push((0x99,12,8,0,0)));
 for pd in [9,12,15,16,17,18]{assert!(q.push((OP_APP_EXIT,0,pd,pd,0)));}
 assert_eq!(q.len(),26);
 for pd in [9,12,15,16,17,18]{assert_eq!(q.pop(),Some((OP_APP_EXIT,0,pd,pd,0)));}
 for m in expected{assert_eq!(q.pop(),Some(m));assert!(q.push(m));assert_eq!(q.len(),20);}
 // The second pass preserves the same FIFO and restores all capacity.
 for _ in 0..20{assert!(q.pop().is_some());}assert_eq!(q.pop(),None);
}
#[test]
fn foreign_and_malformed_document_messages_cannot_take_reserved_capacity(){
 let mut q=queue::Deferred::new();let r=document::Request{seq:1,kind:document::INFO,object:3,offset:0,a:0,b:0};
 for pd in 20..28{assert!(q.push((document::OP,pd,r.header(),0,0)));}
 assert!(!q.push((document::OP,9,0,0,0)));
 for pd in [9,16,18]{assert!(q.push((document::OP,pd,r.header(),0,0)));}
}
#[test]
fn exact_abort_has_independent_capacity_after_all_data_and_ordinary_slots(){
 let mut q=queue::Deferred::new();let mut expected=vec![];
 for i in 0..8{let m=(0x99,12,i,0,0);assert!(q.push(m));expected.push(m);}
 for pd in [9,16,18]{for seq in 1..=4{let r=document::Request{seq,kind:document::WRITE,object:3,offset:(seq-1)*16,a:0,b:0};let m=(document::OP,pd,r.header(),0,0);assert!(q.push(m));expected.push(m);}}
 for pd in [9,16,18]{let r=document::Request{seq:5,kind:document::ABORT,object:0,offset:0,a:0,b:0};let m=(document::OP,pd,r.header(),0,0);assert!(q.push(m));assert!(q.push(m));expected.push(m);}
 assert_eq!(q.len(),23);
 for pd in [9,12,15,16,17,18]{assert!(q.push((OP_APP_EXIT,0,pd,pd,0)));}assert_eq!(q.len(),29);
 for pd in [9,12,15,16,17,18]{assert_eq!(q.pop(),Some((OP_APP_EXIT,0,pd,pd,0)));}
 for m in expected{assert_eq!(q.pop(),Some(m));}assert_eq!(q.pop(),None);
}
#[test]
fn newer_abort_replaces_only_its_own_cancel_slot_at_latest_fifo_position(){
 let mut q=queue::Deferred::new();let abort=|pd,seq|(document::OP,pd,document::Request{seq,kind:document::ABORT,object:0,offset:0,a:0,b:0}.header(),0,0);
 assert!(q.push(abort(9,5)));let ordinary=(0x99,12,0,0,0);assert!(q.push(ordinary));assert!(q.push(abort(16,5)));
 assert!(q.push(abort(9,6)));assert_eq!(q.len(),3);assert!(!q.push(abort(9,4)));
 assert_eq!(q.pop(),Some(ordinary));assert_eq!(q.pop(),Some(abort(16,5)));assert_eq!(q.pop(),Some(abort(9,6)));assert_eq!(q.pop(),None);
 assert!(q.push(abort(9,7)));assert_eq!(q.pop(),Some(abort(9,7)));
}
