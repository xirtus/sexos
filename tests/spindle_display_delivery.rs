#![allow(dead_code)]
use std::{collections::VecDeque,sync::Mutex};
static LOCK:Mutex<()>=Mutex::new(());
static RESULTS:Mutex<VecDeque<Result<u64,u64>>>=Mutex::new(VecDeque::new());
static CALLS:Mutex<Vec<(u64,u64,u64,u64,u64)>>=Mutex::new(Vec::new());
static YIELDS:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
const SLOT_DISPLAY:u64=5;
fn instance_pd()->u32{17}
fn pdx_call(_:u64,_:u64,_:u64,_:u64,_:u64)->(u64,u64){panic!("successor must retain checked delivery")}
mod sex_pdx{
 pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
 #[macro_export]macro_rules! rec{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
 pub use crate::rec as serial_record;
 pub fn pdx_call_checked(s:u64,o:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{crate::CALLS.lock().unwrap().push((s,o,a,b,c));crate::RESULTS.lock().unwrap().pop_front().expect("fixture response")}
 pub fn sys_yield(){crate::YIELDS.fetch_add(1,std::sync::atomic::Ordering::Relaxed);}
 pub fn task_finish(code:u64)->!{panic!("terminal display denial {}",code)}
}
include!(env!("SPINDLE_DISPLAY_PRODUCTION"));
#[test]fn full_retains_exact_create_and_every_text_argument(){let _l=LOCK.lock().unwrap();for op in [0xEC,0xFA,0xFB,0xEE]{
 CALLS.lock().unwrap().clear();RESULTS.lock().unwrap().extend([Err(sex_pdx::ERR_SERVICE_NOT_READY),Err(sex_pdx::ERR_SERVICE_NOT_READY),Ok(7)]);YIELDS.store(0,std::sync::atomic::Ordering::Relaxed);
 assert_eq!(spindle_display_call(op,302,0x1234,0x5678),(0,7));assert_eq!(*CALLS.lock().unwrap(),vec![(5,op,302,0x1234,0x5678);3]);assert_eq!(YIELDS.load(std::sync::atomic::Ordering::Relaxed),2);
 }}
#[test]fn permanent_denial_is_terminal_not_success_or_retry(){let _l=LOCK.lock().unwrap();CALLS.lock().unwrap().clear();RESULTS.lock().unwrap().push_back(Err(1));
 assert!(std::panic::catch_unwind(||spindle_display_call(0xEC,302,1,2)).is_err());assert_eq!(CALLS.lock().unwrap().len(),1);
}
