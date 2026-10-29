extern crate self as sex_pdx;
use std::cell::RefCell;
use std::collections::VecDeque;
#[path="../apps/kaleidoscope/src/service_call.rs"] mod service_call;
#[derive(Clone,Copy)]
pub struct PdxMessage {pub type_id:u64,pub caller_pd:u32,pub arg0:u64,pub arg1:u64,pub arg2:u64}
#[derive(Default)]
struct Transport { full_attempts:usize, yields:usize, accepted:usize, blocking_reads:usize, pending:bool, pending_caller:u32, send_error:u64, calls:Vec<(u64,u64,[u64;3])>, queued:VecDeque<PdxMessage>, completed:usize }
thread_local! { static TRANSPORT:RefCell<Transport>=RefCell::new(Transport::default()); }
pub fn pdx_call(slot:u64,opcode:u64,a:u64,b:u64,c:u64)->(u64,u64) { TRANSPORT.with(|state| {
    let mut state=state.borrow_mut();
    assert!(!state.pending,"second request would overwrite the retained reply slot");
    state.calls.push((slot,opcode,[a,b,c]));
    if state.send_error!=0 {return(state.send_error,0)}
    if state.full_attempts!=0 {state.full_attempts-=1;return(ERR_SERVICE_NOT_READY,0)}
    state.accepted+=1;
    if matches!(opcode,0xEC|0xFB|0xEB) {return(0,0)}
    state.pending=true;
    let (caller,value)=match slot {6=>(3,a),5=>(1,b<<8),_=>(13,a+100)};
    state.pending_caller=caller;
    // The enqueue's second register is deliberately unrelated to the reply.
    state.queued.push_back(PdxMessage{type_id:1,caller_pd:caller,arg0:value,arg1:0,arg2:0});
    (0,0xBAD)
}) }
pub fn pdx_listen_raw(_:u64)->PdxMessage { TRANSPORT.with(|state| {
    let mut state=state.borrow_mut();
    state.blocking_reads+=1;
    let msg=state.queued.pop_front().expect("request must have a pending response");
    if msg.type_id==1&&msg.caller_pd==state.pending_caller {state.pending=false;state.completed+=1;}
    msg
}) }
pub mod app_lifecycle {pub fn close_requested(op:u64,caller:u32,args:[u64;3],target:u32,task:u32)->bool {op==0x108&&caller==3&&args==[target as u64,task as u64,0]}}
pub fn task_finish(_:u64)->! {panic!("authenticated close")}
fn reset(){TRANSPORT.with(|s|*s.borrow_mut()=Transport::default());}
#[test] fn entire_net_sequence_consumes_each_reply_before_next_send(){
    reset();
    let requests=[(0x200,0),(0x20B,1),(0x200,0x207),(0x200,0x208),(0x200,0x209),(0x200,0x20A)];
    for (opcode,arg) in requests {assert_eq!(service_call::request(18,13,opcode,[arg,7,0]),Ok(arg+100));}
    TRANSPORT.with(|s|{let s=s.borrow();assert_eq!(s.calls.len(),6);assert_eq!(s.completed,6);assert!(!s.pending);assert!(s.queued.is_empty());assert_eq!(s.calls[1],(18,0x20B,[1,7,0]));});
}
#[test] fn wrong_sender_and_nonreply_do_not_complete_request(){
    reset(); TRANSPORT.with(|s|{let mut s=s.borrow_mut();for (kind,caller) in [(1,11),(0x200,13),(0x108,12)] {s.queued.push_back(PdxMessage{type_id:kind,caller_pd:caller,arg0:15,arg1:15,arg2:0});}});
    assert_eq!(service_call::request(18,13,0x200,[42,0,0]),Ok(142));
    TRANSPORT.with(|s|assert_eq!(s.borrow().completed,1));
}
#[test] fn enqueue_failure_does_not_wait(){reset();TRANSPORT.with(|s|s.borrow_mut().send_error=3);assert_eq!(service_call::request(18,13,0x200,[0;3]),Err(3));}
#[test] #[should_panic(expected="authenticated close")] fn authorized_close_works_during_exchange(){
    reset();TRANSPORT.with(|s|s.borrow_mut().queued.push_back(PdxMessage{type_id:0x108,caller_pd:3,arg0:15,arg1:15,arg2:0}));
    let _=service_call::request(18,13,0x200,[0;3]);
}

#[test] fn shell_registration_and_display_feedback_finish_in_order(){
    reset();
    assert_eq!(service_call::request(6,3,0xFA,[300,300,15<<8]),Ok(300));
    TRANSPORT.with(|s|{let mut s=s.borrow_mut();assert_eq!(s.completed,1);assert!(!s.pending);
        // A stale shell reply cannot masquerade as display presentation.
        s.queued.push_back(PdxMessage{type_id:1,caller_pd:3,arg0:300,arg1:0,arg2:0});});
    assert_eq!(service_call::request(5,1,0x109,[300,1,0]),Ok(256));
    TRANSPORT.with(|s|{let s=s.borrow();assert_eq!(s.completed,2);assert!(!s.pending);assert!(s.queued.is_empty());});
}

pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
pub fn sys_yield(){TRANSPORT.with(|s|s.borrow_mut().yields+=1);}
pub fn pdx_try_listen_raw(_:u64)->Option<PdxMessage>{TRANSPORT.with(|s|s.borrow_mut().queued.pop_front())}
#[test] fn full_display_queue_retries_exact_payload_once_accepted(){
    reset();TRANSPORT.with(|s|s.borrow_mut().full_attempts=3);
    assert_eq!(service_call::enqueue(5,0xEC,[300,80,520]),Ok(()));
    TRANSPORT.with(|s|{let s=s.borrow();assert_eq!(s.calls,vec![(5,0xEC,[300,80,520]);4]);assert_eq!(s.yields,3);assert_eq!(s.accepted,1);assert_eq!(s.blocking_reads,0);});
}
#[test] fn full_request_queue_does_not_wait_before_acceptance(){
    reset();TRANSPORT.with(|s|s.borrow_mut().full_attempts=2);
    assert_eq!(service_call::request(6,3,0xFA,[300,300,15<<8]),Ok(300));
    TRANSPORT.with(|s|{let s=s.borrow();assert_eq!(s.calls,vec![(6,0xFA,[300,300,15<<8]);3]);assert_eq!(s.accepted,1);assert_eq!(s.yields,2);assert_eq!(s.blocking_reads,1);assert_eq!(s.completed,1);});
}
#[test] #[should_panic(expected="authenticated close")] fn authorized_close_during_full_queue_retry(){
    reset();TRANSPORT.with(|s|{let mut s=s.borrow_mut();s.full_attempts=1;s.queued.push_back(PdxMessage{type_id:0x108,caller_pd:3,arg0:15,arg1:15,arg2:0});});
    let _=service_call::enqueue(5,0xEC,[300,80,520]);
}
