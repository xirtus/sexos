#![allow(dead_code, static_mut_refs)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY: u64 = 2;
#[path="../crates/sex-pdx/src/app_catalog.rs"] pub mod app_catalog;
#[path="../crates/sex-pdx/src/spindle_status.rs"] pub mod spindle_status;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"] mod lifecycle_wire;
#[path="../servers/silk-shell/src/app_custody.rs"] mod app_custody;

// Actual custody/status/catalog types; only kernel query transport and UI effects
// are doubles. No host test executes the SDK syscall instruction.
pub mod app_lifecycle {
    pub use crate::lifecycle_wire::{CatalogInfo, LifecycleInfo, LIVE};
    pub fn catalog(slot: usize) -> Result<CatalogInfo, u64> {
        unsafe { crate::HELD.iter().flatten().find(|g| g.slot == slot).map_or(Err(1), |g| g.catalog) }
    }
    pub fn info(slot: usize) -> Result<LifecycleInfo, u64> {
        unsafe { crate::HELD.iter().flatten().find(|g| g.slot == slot).map_or(Err(1), |g| g.info) }
    }
}
#[derive(Clone, Copy)]
pub struct PdxMessage { pub type_id:u64, pub arg0:u64, pub arg1:u64, pub arg2:u64, pub caller_pd:u32, pub caller_task:u32 }
#[derive(Clone, Copy)]
struct Held { slot:usize, catalog:Result<app_lifecycle::CatalogInfo,u64>, info:Result<app_lifecycle::LifecycleInfo,u64> }
static mut HELD:[Option<Held>;app_catalog::COUNT]=[None;app_catalog::COUNT];
static mut APP_CUSTODY:[Option<app_custody::Entry>;app_catalog::COUNT]=[None;app_catalog::COUNT];
static mut SPINDLE_HISTORY_STATUS:[Option<spindle_status::Receiver>;app_catalog::SPINDLE_COUNT]=[const{None};app_catalog::SPINDLE_COUNT];
static mut COMMAND_PALETTE_OPEN:bool=true;
use std::sync::{Mutex, atomic::{AtomicUsize,Ordering}};
static LOCK:Mutex<()>=Mutex::new(());
static LOGS:AtomicUsize=AtomicUsize::new(0);
static PAINTS:AtomicUsize=AtomicUsize::new(0);
#[macro_export]
macro_rules! serial_record { ($($arg:tt)*) => {{ let _=format_args!($($arg)*); $crate::LOGS.fetch_add(1,std::sync::atomic::Ordering::SeqCst); }}; }
fn palette_render_list(){ PAINTS.fetch_add(1,Ordering::SeqCst); }
include!(env!("SHELL_SPINDLE_STATUS_POLICY"));

fn fixture(target:u32,task:u32) {
    let row=app_catalog::by_target(target).unwrap();
    unsafe {
        HELD=[None;app_catalog::COUNT];APP_CUSTODY=[None;app_catalog::COUNT];
        SPINDLE_HISTORY_STATUS=[const{None};app_catalog::SPINDLE_COUNT];COMMAND_PALETTE_OPEN=true;
        APP_CUSTODY[0]=Some(app_custody::Entry::new(row.lifecycle_slot,target,task,row.surface));
        HELD[0]=Some(Held {slot:row.lifecycle_slot,catalog:Ok(app_lifecycle::CatalogInfo {
            target_pd:target,task_id:task,surface_id:row.surface,coordinator_pd:3 }),info:Err(app_lifecycle::LIVE)});
    }
    LOGS.store(0,Ordering::SeqCst);PAINTS.store(0,Ordering::SeqCst);
}
fn message(target:u32,task:u32,seq:u64,state:spindle_status::State)->PdxMessage {
    let args=spindle_status::Notice{target,task,sequence:seq,state}.args();
    PdxMessage {type_id:spindle_status::OP,caller_pd:target,caller_task:task,arg0:args[0],arg1:args[1],arg2:args[2]}
}
fn send(msg:PdxMessage)->bool {unsafe{spindle_history_status_receive(&msg)}}
fn state(target:u32)->Option<spindle_status::State>{unsafe{SPINDLE_HISTORY_STATUS[app_catalog::SPINDLE_TARGETS.iter().position(|t|*t==target).unwrap()].as_ref().and_then(|s|s.state())}}
fn unbound(){assert!(unsafe{SPINDLE_HISTORY_STATUS.iter().all(Option::is_none)});assert_eq!(LOGS.load(Ordering::SeqCst),0);assert_eq!(PAINTS.load(Ordering::SeqCst),0);}
#[test]
fn exact_held_unequal_tasks_bind_both_actual_incarnations(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for(target,task)in[(12,120),(17,170)]{
        fixture(target,task);assert!(send(message(target,task,1,spindle_status::State::Ready)));
        assert_eq!(state(target),Some(spindle_status::State::Ready));assert_eq!(LOGS.load(Ordering::SeqCst),1);assert_eq!(PAINTS.load(Ordering::SeqCst),1);
    }
}
#[test]
fn wrong_actual_sender_task_cannot_forge_the_expected_packed_tuple(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for(target,task)in[(12,120),(17,170)]{
        for forged in [0,target,task+1] {
            fixture(target,task);let mut msg=message(target,task,100,spindle_status::State::Saving);msg.caller_task=forged;
            assert!(send(msg));unbound();assert!(send(message(target,task,1,spindle_status::State::Ready)));assert_eq!(state(target),Some(spindle_status::State::Ready));
        }
    }
}
#[test]
fn held_catalog_tuple_mismatch_or_absence_cannot_bind(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for field in 0..6 {
        fixture(12,120);
        unsafe { let g=HELD[0].as_mut().unwrap();let c=g.catalog.as_mut().unwrap();
            match field {0=>c.target_pd=17,1=>c.task_id=12,2=>c.surface_id=302,3=>c.coordinator_pd=11,4=>g.catalog=Err(1),_=>HELD[0]=None}
        }
        assert!(send(message(12,120,1,spindle_status::State::Ready)));unbound();
    }
}
#[test]
fn nonlive_contended_and_terminal_held_status_cannot_bind(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for status in [Err(1),Err(2),Err(3),Ok(app_lifecycle::LifecycleInfo{target_pd:12,task_id:120,required_mask:31,acked_mask:0})] {
        fixture(12,120);unsafe{HELD[0].as_mut().unwrap().info=status;}
        assert!(send(message(12,120,1,spindle_status::State::Ready)));unbound();
    }
}
#[test]
fn missing_custody_and_wrong_pd_do_not_bind(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    fixture(12,120);unsafe{APP_CUSTODY[0]=None;}assert!(send(message(12,120,1,spindle_status::State::Ready)));unbound();
    for pd in [0,9,17,u32::MAX] {
        fixture(12,120);let mut msg=message(12,120,1,spindle_status::State::Ready);msg.caller_pd=pd;
        assert!(send(msg));unbound();
    }
}
#[test]
fn terminal_or_close_pending_custody_retires_and_cannot_reaccept(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for field in 0..3 {
        fixture(12,120);assert!(send(message(12,120,1,spindle_status::State::Ready)));
        unsafe{let e=APP_CUSTODY[0].as_mut().unwrap();match field {0=>e.dead=true,1=>e.retired=true,_=>e.close_pending=true}}
        assert!(send(message(12,120,2,spindle_status::State::Saving)));assert_eq!(state(12),None);
        assert_eq!(LOGS.load(Ordering::SeqCst),1);assert_eq!(PAINTS.load(Ordering::SeqCst),1);
        // Clearing a local close flag cannot revive the already-retired receiver.
        unsafe{let e=APP_CUSTODY[0].as_mut().unwrap();e.dead=false;e.retired=false;e.close_pending=false;}
        assert!(send(message(12,120,3,spindle_status::State::Loading)));assert_eq!(state(12),None);assert_eq!(LOGS.load(Ordering::SeqCst),1);
    }
}
#[test]
fn stale_and_malformed_notices_do_not_advance_state_or_sequence(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    fixture(17,170);assert!(send(message(17,170,1,spindle_status::State::Ready)));
    for field in 0..5 {
        let mut msg=message(17,170,100,spindle_status::State::Saving);
        match field {0=>msg.arg1=0,1=>msg.arg1=1,2=>msg.arg0=17|17<<32,3=>msg.arg0=12|120<<32,_=>msg.arg2=99}
        assert!(send(msg));assert_eq!(state(17),Some(spindle_status::State::Ready));assert_eq!(LOGS.load(Ordering::SeqCst),1);assert_eq!(PAINTS.load(Ordering::SeqCst),1);
    }
    assert!(send(message(17,170,2,spindle_status::State::Loading)));assert_eq!(state(17),Some(spindle_status::State::Loading));
}
#[test]
fn unrelated_opcode_does_not_consume_message_or_bind(){
    let _lock=LOCK.lock().unwrap_or_else(|e|e.into_inner());
    fixture(12,120);let mut msg=message(12,120,1,spindle_status::State::Ready);msg.type_id=1;
    assert!(!send(msg));unbound();
}
