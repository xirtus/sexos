// SPDX-License-Identifier: GPL-3.0-only
//! Actual FIFO, key protocol and extracted Shell dispatch; transport/liveness
//! callbacks are host models, not capability or native delivery evidence.
#![allow(dead_code, static_mut_refs)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] #[allow(dead_code)] pub mod app_catalog;
#[path = "../crates/sex-pdx/src/text_encoding.rs"] pub mod text_encoding;
#[path = "../crates/sex-pdx/src/app_key.rs"] pub mod app_key;
#[path = "../crates/sex-pdx/src/app_scroll.rs"] pub mod app_scroll;
pub const EV_WHEEL:u64=app_scroll::CLASS;
#[path = "../crates/sex-pdx/src/app_lifecycle.rs"] pub mod app_lifecycle;
#[path = "../crates/sex-pdx/src/text_selection.rs"] pub mod text_selection;
#[path = "../crates/sex-pdx/src/clipboard.rs"] pub mod clipboard;
#[path = "../servers/silk-shell/src/app_input.rs"] mod app_input;
#[path = "../servers/silk-shell/src/clipboard.rs"] mod broker;
#[path = "../servers/silk-shell/src/desktop_keys.rs"] mod desktop_keys;
#[path = "../servers/silk-shell/src/browser_input.rs"] mod browser_input;
use app_input::{Input, Intake, Item, Queue, CAPACITY, INTAKE_DEFER_TURNS};
use app_key::{Key, Modifiers};
use clipboard::{Action, Participant, Subject};
use std::collections::VecDeque;
fn participant(pd: u32, task: u32) -> Participant {
    Participant::from_catalog(app_lifecycle::CatalogInfo { target_pd: pd, task_id: task,
        surface_id: match pd { 9 => 156, 15 => 300, 16 => 301, 18 => 307, _ => panic!() }, coordinator_pd: 3 }).unwrap()
}
fn item(n: usize, destination: Participant) -> Item {
    Item { destination, input: if n%5==0 { Input::Clipboard(Action::Copy) } else {
        if n%3==0 { Input::Scroll(app_scroll::Scroll::new(if n%2==0{-1}else{1}).unwrap()) } else { Input::Key(Key { code: (n%255+1)as u8, shift: n%2!=0, control:n%3==0 }) } } }
}
fn drain(queue: &mut Queue) -> Vec<Item> {
    let mut result=Vec::new();while let Some(front)=queue.front() {result.push(front);queue.pop();}result
}
#[test] fn full_fifo_never_overwrites_accepted_destinations_or_modifier_snapshots() {
    let mut q=Queue::new();let expected:Vec<_>=(0..CAPACITY).map(|n|item(n,participant(if n%2==0{9}else{15},70+n as u32))).collect();
    for &event in &expected {assert!(q.push(event));}let front=q.front();
    for n in 0..10 {assert!(!q.push(item(100+n,participant(18,18))));assert_eq!(q.front(),front);}
    assert_eq!(drain(&mut q),expected);assert!(!q.pending());q.pop();assert!(q.front().is_none());
}
#[test] fn many_wraps_match_ordered_oracle_and_exact_retirement_preserves_survivors() {
    let mut q=Queue::new();let mut oracle=VecDeque::new();let subjects=[participant(9,9),participant(9,99),participant(15,15),participant(18,18)];
    for n in 0..4096 {
        if n%7==0 {let subject=subjects[(n/7)%subjects.len()].subject;let before=oracle.len();oracle.retain(|x:&Item|x.destination.subject!=subject);assert_eq!(q.retire(subject),before-oracle.len());}
        else if n%3==0 {assert_eq!(q.front(),oracle.front().copied());q.pop();oracle.pop_front();}
        else {let event=item(n,subjects[n%subjects.len()]);let accepted=q.push(event);assert_eq!(accepted,oracle.len()<CAPACITY);if accepted{oracle.push_back(event);}}
        assert_eq!(q.front(),oracle.front().copied());assert_eq!(q.pending(),!oracle.is_empty());
    }
    assert_eq!(drain(&mut q),oracle.into_iter().collect::<Vec<_>>());
}
#[test] fn retirement_is_exact_pd_and_task_after_wrapped_full_queue() {
    let mut q=Queue::new();let target=participant(9,90);let survivors=[participant(9,91),participant(15,90),participant(16,160)];
    for n in 0..CAPACITY {assert!(q.push(item(n,target)));}for _ in 0..19{q.pop();}
    for n in 0..19 {assert!(q.push(item(100+n,survivors[n%3])));}
    assert_eq!(q.retire(Subject{pd:9,task:999}),0);assert_eq!(q.retire(target.subject),CAPACITY-19);
    let expected:Vec<_>=(0..19).map(|n|item(100+n,survivors[n%3])).collect();assert_eq!(drain(&mut q),expected);assert_eq!(q.retire(target.subject),0);
}
#[test] fn invalid_queue_destinations_fail_without_changing_existing_items() {
    let mut q=Queue::new();let original=item(1,participant(9,9));assert!(q.push(original));
    for invalid in [Participant{subject:Subject{pd:0,task:9},..original.destination},Participant{subject:Subject{pd:9,task:0},..original.destination},Participant{route:0,..original.destination}] {
        assert!(!q.push(item(2,invalid)));assert_eq!(q.front(),Some(original));
    }assert_eq!(drain(&mut q),[original]);
}
#[test] fn both_modifiers_remain_owned_by_shell_through_focus_changes() {
    let mut m=Modifiers::new();assert!(m.observe(0x2a,1));assert!(m.observe(0x36,1));assert!(m.observe(0x1d,1));assert!(m.observe(0x61,1));let mut q=Queue::new();
    let old=Item{destination:participant(9,9),input:Input::Key(m.key(0x2e))};assert!(q.push(old));
    // Focus moves while both physical shifts/controls are down.
    assert!(m.observe(0x2a,0));assert!(m.observe(0x1d,0));let new=Item{destination:participant(15,15),input:Input::Key(m.key(0x2f))};assert!(q.push(new));
    assert!(m.observe(0x36,0));assert!(m.observe(0x61,0));assert_eq!(m.key(0x1e),Key{code:0x1e,shift:false,control:false});
    assert_eq!(drain(&mut q),[old,new]);assert!(matches!(new.input,Input::Key(Key{shift:true,control:true,..})));
    for code in [0x2a,0x36,0x1d,0x61] {assert!(!m.observe(code,2));}assert!(!m.observe(0x20,1));assert_eq!(m.key(0x20),Key{code:0x20,shift:false,control:false});
}
#[test] fn key_wire_is_canonical_for_all_codes_modifiers_and_reserved_bits() {
    for code in 1..=255 {for shift in [false,true] {for control in [false,true] {
        let key=Key{code,shift,control};assert_eq!(Key::decode(key.wire(),1,1),Some(key));
        for bit in (10..16).chain(17..64) {assert!(Key::decode(key.wire()|1u64<<bit,1,1).is_none());}
        for (down,class) in [(0,1),(2,1),(1,0),(1,2),(u64::MAX,1),(1,u64::MAX)] {assert!(Key::decode(key.wire(),down,class).is_none());}
    }}}
    for word in [0,1,255,1<<16,(1<<16)|256,u64::MAX] {assert!(Key::decode(word,1,1).is_none());}
}

fn payload(ticket:u32,byte:u8)->clipboard::Packet {clipboard::Request::Write{ticket,offset:0,count:16,bytes:[byte;16]}.wire()}
#[test] fn intake_full_and_transient_live_retry_retain_exact_fifo_and_authenticated_subject_data(){
    let mut q=Intake::new();let owner=participant(9,9).subject;let expected:Vec<_>=(0..CAPACITY).map(|n|(owner,payload(n as u32+1,b'A'+n as u8%26))).collect();
    for &(who,packet)in &expected{assert!(q.push(who,packet));}let front=q.front();assert!(!q.push(owner,payload(999,b'z')));
    for _ in 0..20{assert!(q.defer());assert_eq!(q.front(),front);}for event in expected{assert_eq!(q.front(),Some(event));q.pop();}assert!(!q.pending());assert!(q.storage_erased());assert!(!q.defer());
}
#[test] fn intake_rejects_noncanonical_request_and_zero_subject_without_overwriting(){
    let mut q=Intake::new();let owner=participant(9,9).subject;let packet=payload(1,b'x');assert!(q.push(owner,packet));let before=q.front();
    for subject in [Subject{pd:0,task:9},Subject{pd:9,task:0},Subject{pd:0,task:0}]{assert!(!q.push(subject,packet));assert_eq!(q.front(),before);}
    for bad in [clipboard::Packet{opcode:clipboard::DATA,args:packet.args},clipboard::Packet{opcode:clipboard::WRITE,args:[0,0,0]},clipboard::Packet{opcode:clipboard::WRITE,args:[1|1u64<<48,1,1]},clipboard::Packet{opcode:clipboard::QUERY,args:[1,0,1]}]{assert!(!q.push(owner,bad));assert_eq!(q.front(),before);}
    q.pop();assert!(q.storage_erased());
}
#[test] fn intake_wrapped_retirement_erases_whole_array_and_retains_exact_survivor_order_and_age(){
    let mut q=Intake::new();let target=participant(9,9).subject;let survivor=participant(9,99).subject;
    for n in 0..CAPACITY{assert!(q.push(target,payload(n as u32+1,b'x')));}for _ in 0..19{q.pop();}
    for n in 0..19{assert!(q.push(survivor,payload(100+n,b'y')));}assert_eq!(q.retire(Subject{pd:9,task:999}),0);assert_eq!(q.retire(target),CAPACITY-19);
    for n in 0..19{assert_eq!(q.front(),Some((survivor,payload(100+n,b'y'))));q.pop();}assert!(q.storage_erased());
    for n in 0..CAPACITY{assert!(q.push(target,payload(n as u32+1,b'p')));}assert_eq!(q.retire(target),CAPACITY);assert!(q.storage_erased());
    assert!(q.push(survivor,payload(1,b's')));for _ in 0..INTAKE_DEFER_TURNS-1{assert!(q.defer());}assert_eq!(q.retire(target),0);assert!(!q.defer());assert!(q.storage_erased());
}
#[test] fn every_intake_entry_expires_within_finite_turns_even_with_frozen_clock(){
    let mut q=Intake::new();let owner=participant(15,15).subject;for n in 0..CAPACITY{assert!(q.push(owner,payload(n as u32+1,b'x')));}
    let mut expired=0;for _ in 0..CAPACITY*INTAKE_DEFER_TURNS as usize{if !q.defer(){expired+=1;}}
    assert_eq!(expired,CAPACITY);assert!(!q.pending());assert!(q.storage_erased());
}

pub const ERR_SERVICE_NOT_READY:u64=9;
pub const ERR_CAP_INVALID:u64=0xffff_ffff_ffff_fffc;
pub fn get_ticks()->u64{1}
#[macro_export] macro_rules! serial_record {($($arg:tt)*)=>{{let _=format_args!($($arg)*);}}}
mod shell {
    use super::*;
    mod clipboard {pub use crate::clipboard::*;pub use crate::broker::Admission;}
    const OP_HID_EVENT:u64=0x100;
    const EV_KEY:u64=1;
    static LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
    static mut APP_INPUT:Queue=Queue::new();
    static mut CLIPBOARD_INTAKE:Intake=Intake::new();
    static mut CLIPBOARD:broker::Broker=broker::Broker::new();
    static mut APP_MODIFIERS:Modifiers=Modifiers::new();
    static mut DESKTOP_KEYS:desktop_keys::Keys=desktop_keys::Keys::new();
    static mut FOCUSED_SURFACE_ID:u64=156;
    static mut COMMAND_PALETTE_OPEN:bool=false;
    static mut FOCUS_CHANGES:usize=0;
    static mut PALETTE_TOGGLES:usize=0;
    static mut LIVE:Option<bool>=Some(true);
    static mut SOURCE_LIVE:Option<(Subject,Option<bool>)>=None;
    static mut LIVE_ONCE:Option<(Subject,Option<bool>)>=None;
    static mut LIVE_QUERIES:Vec<Subject>=Vec::new();
    static mut STATUS:u64=0;
    static mut SENT:Vec<(u64,u64,[u64;3])>=Vec::new();
    unsafe fn clipboard_live(subject:Subject)->Option<bool>{LIVE_QUERIES.push(subject);if let Some((source,value))=LIVE_ONCE{if source==subject{LIVE_ONCE=None;return value}}match SOURCE_LIVE{Some((source,value))if source==subject=>value,_=>LIVE}}
    unsafe fn pdx_call(route:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){SENT.push((route,op,[a,b,c]));(STATUS,0)}
    unsafe fn focused_clipboard_participant()->Option<Participant>{match FOCUSED_SURFACE_ID{156=>Some(participant(9,9)),300=>Some(participant(15,15)),301=>Some(participant(16,16)),307=>Some(participant(18,18)),_=>None}}
    unsafe fn surface_is_alive(sid:u64)->bool{matches!(sid,156|300|301|307)}
    unsafe fn toggle_command_palette(){COMMAND_PALETTE_OPEN=!COMMAND_PALETTE_OPEN;PALETTE_TOGGLES+=1;}
    unsafe fn focus_next_frame(){FOCUSED_SURFACE_ID=if FOCUSED_SURFACE_ID==156{300}else{156};FOCUS_CHANGES+=1;}
    include!(env!("APP_INPUT_PRODUCTION"));
    unsafe fn reset(){APP_INPUT=Queue::new();CLIPBOARD_INTAKE=Intake::new();CLIPBOARD=broker::Broker::new();APP_MODIFIERS=Modifiers::new();DESKTOP_KEYS=desktop_keys::Keys::new();FOCUSED_SURFACE_ID=156;COMMAND_PALETTE_OPEN=false;FOCUS_CHANGES=0;PALETTE_TOGGLES=0;for pd in [9,15,16,18]{CLIPBOARD.register(app_lifecycle::CatalogInfo{target_pd:pd,task_id:pd,surface_id:participant(pd,pd).surface,coordinator_pd:3}).unwrap();}LIVE=Some(true);SOURCE_LIVE=None;LIVE_ONCE=None;STATUS=0;SENT.clear();LIVE_QUERIES.clear();}
    #[test] fn actual_shell_full_keeps_exact_front_then_sends_original_route_in_fifo_order(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let first=item(1,participant(9,9));let second=item(2,participant(15,15));assert!(APP_INPUT.push(first));assert!(APP_INPUT.push(second));STATUS=9;
        for _ in 0..4{app_input_poll();assert_eq!(APP_INPUT.front(),Some(first));}assert_eq!(SENT.len(),4);assert!(SENT.iter().all(|&(route,_,_)|route==first.destination.route));
        STATUS=0;app_input_poll();assert_eq!(APP_INPUT.front(),Some(second));app_input_poll();assert!(!APP_INPUT.pending());assert_eq!(SENT.last().unwrap().0,second.destination.route);
        let key=match first.input{Input::Key(k)=>k,_=>panic!()};assert_eq!(SENT[0].2,[key.wire(),1,1]);}
    }
    #[test] fn actual_shell_transient_live_query_retains_and_terminal_destination_drops_without_send(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let first=item(1,participant(9,9));let second=item(2,participant(15,15));APP_INPUT.push(first);APP_INPUT.push(second);LIVE=None;app_input_poll();assert_eq!(APP_INPUT.front(),Some(first));assert!(SENT.is_empty());LIVE=Some(false);app_input_poll();assert_eq!(APP_INPUT.front(),Some(second));assert!(SENT.is_empty());LIVE=Some(true);app_input_poll();assert_eq!(SENT.len(),1);assert_eq!(SENT[0].0,second.destination.route);}
    }
    #[test] fn actual_shell_waits_for_intent_admission_but_not_entire_clipboard_transaction(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();CLIPBOARD.issue(participant(9,9).subject,Action::Copy,true,1).unwrap();let event=item(1,participant(9,9));APP_INPUT.push(event);app_input_poll();assert_eq!(APP_INPUT.front(),Some(event));assert!(SENT.is_empty());clipboard_poll();assert!(CLIPBOARD.busy());assert!(!CLIPBOARD.outbox_pending());app_input_poll();assert!(!APP_INPUT.pending());assert_eq!(SENT.len(),2);assert_eq!(SENT[0].1,clipboard::INTENT);assert_eq!(SENT[1].1,OP_HID_EVENT);}
    }
    #[test] fn actual_shell_busy_clipboard_action_retains_fifo_until_prior_transaction_quiesces(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let who=participant(9,9);let old=CLIPBOARD.issue(who.subject,Action::Copy,true,1).unwrap();clipboard_poll();let paste=Item{destination:who,input:Input::Clipboard(Action::Paste)};let after=item(1,who);APP_INPUT.push(paste);APP_INPUT.push(after);let before=SENT.len();
        for _ in 0..5{app_input_poll();assert_eq!(APP_INPUT.front(),Some(paste));assert_eq!(SENT.len(),before);}CLIPBOARD.receive(who.subject,clipboard::Request::Cancel{ticket:old},true,1).unwrap();clipboard_poll();app_input_poll();assert_eq!(APP_INPUT.front(),Some(after));assert_eq!(SENT.last().unwrap().1,clipboard::INTENT);app_input_poll();assert!(!APP_INPUT.pending());assert_eq!(SENT.last().unwrap().1,OP_HID_EVENT);}
    }
    #[test] fn actual_shell_intake_live_retry_retains_then_processes_exact_begin_without_replay(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let who=participant(9,9).subject;let t=CLIPBOARD.issue(who,Action::Copy,true,1).unwrap();clipboard_poll();let request=clipboard::Request::Begin{ticket:t,len:16,source:text_selection::Source{generation:1,revision:0}}.wire();CLIPBOARD_INTAKE.push(who,request);LIVE=None;let before=SENT.len();
        for _ in 0..20{clipboard_intake_poll();assert_eq!(CLIPBOARD_INTAKE.front(),Some((who,request)));}assert_eq!(SENT.len(),before);LIVE=Some(true);clipboard_intake_poll();assert!(CLIPBOARD_INTAKE.storage_erased());assert!(CLIPBOARD.outbox_pending());clipboard_poll();assert_eq!(SENT.len(),before+1);let (_,opcode,args)=SENT[before];assert!(matches!(clipboard::Response::decode(opcode,args),Some(clipboard::Response::Meta{phase:clipboard::Phase::Begin,status:clipboard::Status::Ok,len:16,..})));clipboard_intake_poll();assert_eq!(SENT.len(),before+1);}
    }
    #[test] fn actual_shell_intake_frozen_retry_expires_and_erases_without_fabricated_ack(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let owner=participant(9,9).subject;CLIPBOARD_INTAKE.push(owner,payload(1,b'x'));LIVE=None;for _ in 0..INTAKE_DEFER_TURNS{clipboard_intake_poll();}assert!(CLIPBOARD_INTAKE.storage_erased());assert!(SENT.is_empty());assert!(!CLIPBOARD.pending());assert!(CLIPBOARD.publication().is_none());}
    }
    unsafe fn unfinished_read()->(Subject,Subject,u32,u32,u64){
        let producer=participant(15,15).subject;let consumer=participant(9,9).subject;let body=b"private";let ticket=CLIPBOARD.issue(producer,Action::Copy,true,1).unwrap();clipboard_poll();
        CLIPBOARD.receive(producer,clipboard::Request::Begin{ticket,len:body.len(),source:text_selection::Source{generation:7,revision:11}},true,1).unwrap();clipboard_poll();let mut bytes=[0;16];bytes[..body.len()].copy_from_slice(body);
        CLIPBOARD.receive(producer,clipboard::Request::Write{ticket,offset:0,count:body.len(),bytes},true,1).unwrap();CLIPBOARD.receive(producer,clipboard::Request::Publish{ticket,len:body.len(),hash:clipboard::hash(body)},true,1).unwrap();clipboard_poll();let publication=CLIPBOARD.publication().unwrap();
        let ticket=CLIPBOARD.issue(consumer,Action::Paste,true,1).unwrap();clipboard_poll();CLIPBOARD.receive(consumer,clipboard::Request::Read{ticket},true,1).unwrap();clipboard_poll();CLIPBOARD.receive(consumer,clipboard::Request::Get{ticket,offset:0},true,1).unwrap();clipboard_poll();
        (producer,consumer,ticket,publication.generation,publication.hash)
    }
    #[test] fn actual_shell_live_consumer_cannot_receive_queued_positive_finish_after_observed_producer_retirement(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let(producer,consumer,ticket,generation,hash)=unfinished_read();CLIPBOARD.receive(consumer,clipboard::Request::Finish{ticket,generation,hash},true,1).unwrap();
        APP_INPUT.push(item(1,participant(15,15)));let survivor=item(2,participant(9,9));APP_INPUT.push(survivor);CLIPBOARD_INTAKE.push(producer,payload(1,b'x'));CLIPBOARD_INTAKE.push(consumer,clipboard::Request::Query{ticket}.wire());
        SOURCE_LIVE=Some((producer,Some(false)));let before=SENT.len();LIVE_QUERIES.clear();clipboard_poll();assert_eq!(LIVE_QUERIES[0],producer);assert!(LIVE_QUERIES.contains(&consumer));assert_eq!(SENT.len(),before+1);let(route,op,args)=SENT[before];assert_eq!(route,participant(9,9).route);assert!(matches!(clipboard::Response::decode(op,args),Some(clipboard::Response::Meta{ticket:t,phase:clipboard::Phase::Finish,status:clipboard::Status::Invalidated,len:0,generation:0,hash:0})if t==ticket));
        assert!(CLIPBOARD.publication().is_none());assert!(!CLIPBOARD.busy());assert_eq!(APP_INPUT.front(),Some(survivor));assert_eq!(CLIPBOARD_INTAKE.front().unwrap().0,consumer);assert_eq!(CLIPBOARD.issue(producer,Action::Copy,true,1),Err(clipboard::Status::Denied));}
    }
    #[test] fn actual_shell_source_retry_retains_finish_then_positive_observation_allows_completed_handoff(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let(producer,consumer,ticket,generation,hash)=unfinished_read();CLIPBOARD.receive(consumer,clipboard::Request::Finish{ticket,generation,hash},true,1).unwrap();SOURCE_LIVE=Some((producer,None));let before=SENT.len();
        for _ in 0..4{clipboard_poll();assert_eq!(SENT.len(),before);assert!(CLIPBOARD.busy());assert!(CLIPBOARD.outbox_pending());}
        SOURCE_LIVE=Some((producer,Some(true)));clipboard_poll();assert_eq!(SENT.len(),before+1);let(_,op,args)=SENT[before];assert!(matches!(clipboard::Response::decode(op,args),Some(clipboard::Response::Meta{phase:clipboard::Phase::Finish,status:clipboard::Status::Ok,..})));assert!(!CLIPBOARD.busy());
        // This proves service-observed retirement ordering, not an atomic
        // global task-exit versus IPC-admission policy in the kernel.
        SOURCE_LIVE=Some((producer,Some(false)));clipboard_poll();assert_eq!(SENT.len(),before+1);CLIPBOARD.receive(consumer,clipboard::Request::Query{ticket},true,1).unwrap();clipboard_poll();let(_,op,args)=*SENT.last().unwrap();assert!(matches!(clipboard::Response::decode(op,args),Some(clipboard::Response::Meta{phase:clipboard::Phase::Finish,status:clipboard::Status::Ok,..})));}
    }
    #[test] fn actual_shell_intake_checks_producer_before_processing_live_consumers_finish(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let(producer,consumer,ticket,generation,hash)=unfinished_read();let finish=clipboard::Request::Finish{ticket,generation,hash}.wire();CLIPBOARD_INTAKE.push(consumer,finish);SOURCE_LIVE=Some((producer,None));let before=SENT.len();clipboard_intake_poll();assert_eq!(CLIPBOARD_INTAKE.front(),Some((consumer,finish)));assert_eq!(SENT.len(),before);
        SOURCE_LIVE=Some((producer,Some(false)));clipboard_intake_poll();assert!(CLIPBOARD_INTAKE.storage_erased());assert!(CLIPBOARD.publication().is_none());clipboard_poll();assert_eq!(SENT.len(),before+1);let(_,op,args)=SENT[before];assert!(matches!(clipboard::Response::decode(op,args),Some(clipboard::Response::Meta{phase:clipboard::Phase::Finish,status:clipboard::Status::Invalidated,..})));}
    }
    #[test] fn source_retirement_during_fresh_query_never_pops_another_subjects_surviving_intake(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();let(producer,consumer,ticket,_,_)=unfinished_read();CLIPBOARD_INTAKE.push(producer,clipboard::Request::Query{ticket:1}.wire());let survivor=clipboard::Request::Query{ticket}.wire();CLIPBOARD_INTAKE.push(consumer,survivor);
        SOURCE_LIVE=Some((producer,Some(false)));LIVE_ONCE=Some((producer,Some(true)));clipboard_intake_poll();assert_eq!(CLIPBOARD_INTAKE.front(),Some((consumer,survivor)));clipboard_poll();assert!(CLIPBOARD.publication().is_none());assert_eq!(CLIPBOARD_INTAKE.front(),Some((consumer,survivor)));}
    }
    #[test] fn actual_shortcut_delivers_ps2_usb_browser_navigation_with_exact_modifier_snapshot(){
        let _lock=LOCK.lock().unwrap();unsafe{for (raw,canonical)in [(0x47,0x66),(0x48,0x67),(0x49,0x68),(0x4b,0x69),(0x4d,0x6a),(0x4f,0x6b),(0x50,0x6c),(0x51,0x6d)]{for code in [raw,canonical]{for shift in [false,true]{for control in [false,true]{
            reset();FOCUSED_SURFACE_ID=300;if shift{assert!(desktop_shortcut(0x36,1));}if control{assert!(desktop_shortcut(0x61,1));}assert!(SENT.is_empty());assert!(desktop_shortcut(code,1));assert_eq!(SENT.len(),1);let(route,opcode,args)=SENT[0];assert_eq!((route,opcode),(43,OP_HID_EVENT));assert_eq!(Key::decode(args[0],args[1],args[2]),Some(Key{code:canonical,shift,control}));assert!(desktop_shortcut(code,0));assert_eq!(SENT.len(),1);
        }}}}reset();for code in [0x53,0x6f]{assert!(desktop_shortcut(code,1));let(route,_,args)=*SENT.last().unwrap();assert_eq!(route,41);assert_eq!(Key::decode(args[0],args[1],args[2]).unwrap().code,code);}}
    }
    #[test] fn actual_shortcut_preserves_shell_grave_function_and_explicit_global_reserved_keys(){
        let _lock=LOCK.lock().unwrap();unsafe{for focus in [156,300]{reset();FOCUSED_SURFACE_ID=focus;for code in [0x0f,0x29,0x3b,0x3c,0x3d,0x3e,0x3f,0x40,0x41,0x42,0x43,0x44,0x46,0x52,0x57,0x58,0x59]{assert!(!desktop_shortcut(code,1),"reserved code={code:#x}");assert!(!APP_INPUT.pending());assert!(SENT.is_empty());}}
        COMMAND_PALETTE_OPEN=true;assert!(!desktop_shortcut(0x1e,1));assert!(SENT.is_empty());FOCUSED_SURFACE_ID=999;COMMAND_PALETTE_OPEN=false;assert!(!desktop_shortcut(0x1e,1));assert!(SENT.is_empty());}
    }
    #[test] fn actual_alt_tab_consumes_one_switch_and_modifiers_release_correctly_after_focus_change(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();for code in [0x2a,0x36,0x1d,0x61]{assert!(desktop_shortcut(code,1));}COMMAND_PALETTE_OPEN=true;assert!(desktop_shortcut(0x38,1));assert!(desktop_shortcut(0x0f,1));assert_eq!(FOCUSED_SURFACE_ID,300);assert_eq!(FOCUS_CHANGES,1);assert_eq!(PALETTE_TOGGLES,1);assert!(!COMMAND_PALETTE_OPEN);assert!(desktop_shortcut(0x0f,1));assert_eq!(FOCUS_CHANGES,1);assert!(desktop_shortcut(0x0f,0));assert!(desktop_shortcut(0x38,0));assert!(SENT.is_empty());
        assert!(desktop_shortcut(0x2a,0));assert!(desktop_shortcut(0x1d,0));assert!(desktop_shortcut(0x4b,1));let(route,_,args)=*SENT.last().unwrap();assert_eq!(route,43);assert_eq!(Key::decode(args[0],args[1],args[2]),Some(Key{code:0x69,shift:true,control:true}));assert!(desktop_shortcut(0x36,0));assert!(desktop_shortcut(0x61,0));assert!(desktop_shortcut(0x30,1));let(_,_,args)=*SENT.last().unwrap();assert_eq!(Key::decode(args[0],args[1],args[2]),Some(Key{code:0x30,shift:false,control:false}));assert_eq!(FOCUS_CHANGES,1);}
    }
    #[test] fn plain_tab_reaches_shell_focus_cycle_for_quil_and_browser_but_alt_tab_keeps_capture(){
        let _lock=LOCK.lock().unwrap();unsafe{for focus in [156,300]{reset();FOCUSED_SURFACE_ID=focus;assert!(!desktop_shortcut(0x0f,1));assert!(!APP_INPUT.pending());assert!(SENT.is_empty());assert_eq!(FOCUS_CHANGES,0);assert!(desktop_shortcut(0x0f,0));
        assert!(desktop_shortcut(0x38,1));assert!(desktop_shortcut(0x0f,1));assert_eq!(FOCUS_CHANGES,1);assert!(desktop_shortcut(0x0f,1));assert_eq!(FOCUS_CHANGES,1);assert!(desktop_shortcut(0x0f,0));assert!(desktop_shortcut(0x38,0));assert!(!APP_INPUT.pending());assert!(SENT.is_empty());
        assert!(!desktop_shortcut(0x0f,1));assert_eq!(FOCUS_CHANGES,1);assert!(!APP_INPUT.pending());assert!(SENT.is_empty());}}
    }
    #[test] fn actual_ctrl_c_intent_precedes_following_key_and_keeps_original_focus_destination(){
        let _lock=LOCK.lock().unwrap();unsafe{reset();assert!(desktop_shortcut(0x36,1));assert!(desktop_shortcut(0x1e,1));assert_eq!(SENT.len(),1);assert!(desktop_shortcut(0x1d,1));STATUS=ERR_SERVICE_NOT_READY;assert!(desktop_shortcut(0x2e,1));assert!(CLIPBOARD.outbox_pending());assert!(desktop_shortcut(0x4d,1));let expected=Item{destination:participant(9,9),input:Input::Key(Key{code:0x4d,shift:true,control:true})};assert_eq!(APP_INPUT.front(),Some(expected));assert!(desktop_shortcut(0x2e,0));
        FOCUSED_SURFACE_ID=300;STATUS=0;clipboard_poll();let intent_index=SENT.len()-1;assert_eq!((SENT[intent_index].0,SENT[intent_index].1),(41,clipboard::INTENT));app_input_poll();assert!(!APP_INPUT.pending());let(route,opcode,args)=*SENT.last().unwrap();assert_eq!((route,opcode),(41,OP_HID_EVENT));assert_eq!(Key::decode(args[0],args[1],args[2]),Some(Key{code:0x4d,shift:true,control:true}));assert!(SENT.len()-1>intent_index);assert!(desktop_shortcut(0x1d,0));assert!(desktop_shortcut(0x36,0));assert!(desktop_shortcut(0x30,1));let(route,_,args)=*SENT.last().unwrap();assert_eq!(route,43);assert_eq!(Key::decode(args[0],args[1],args[2]),Some(Key{code:0x30,shift:false,control:false}));}
    }
}

#[test]fn wheel_fifo_keeps_exact_subject_and_signed_payload_through_wrap_full_and_retire(){
 let mut queue=Queue::new();let browser=participant(15,15);let other=participant(9,9);
 let mut expected=Vec::new();
 for n in 0..CAPACITY{let entry=Item{destination:if n%2==0{browser}else{other},input:Input::Scroll(app_scroll::Scroll::new((n as i8+1)*if n%2==0{-1}else{1}).unwrap())};assert!(queue.push(entry));expected.push(entry);}
 assert!(!queue.push(Item{destination:browser,input:Input::Scroll(app_scroll::Scroll::new(-1).unwrap())}));
 assert_eq!(queue.retire(Subject{pd:15,task:99}),0);
 assert_eq!(queue.retire(browser.subject),CAPACITY/2);expected.retain(|entry|entry.destination!=browser);
 assert_eq!(drain(&mut queue),expected);
}
