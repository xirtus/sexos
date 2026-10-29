//! Production dispatch and initial fill; model only IPC transport and device effects.
#![allow(dead_code,unused_variables,static_mut_refs)]
extern crate self as sex_pdx;
use std::sync::{Mutex,mpsc};
#[macro_export]macro_rules! serial_record{($($t:tt)*)=>{}}
macro_rules! serial_println{($($t:tt)*)=>{}}
include!(env!("SEXDRIVE_SERVICE_ABI_SOURCE"));
const SLOT_NVME_HOST:u64=0;const NVME_LBA_BYTES:u64=BLOCK_SECTOR_SIZE;const WRITE_PROOF_LBA:u64=2047;
#[derive(Clone,Copy)]struct RegionView{virtual_address:u64}
const READ:u8=1;const WRITE:u8=2;
#[derive(Clone,Copy)]struct Message{type_id:u64,caller_pd:u32,arg0:u64,arg1:u64,arg2:u64}
struct NvmeState{ready:bool,next_cid:u16,sq_tail:u32}
static mut NVME_IO_STATE:NvmeState=NvmeState{ready:true,next_cid:1,sq_tail:0};
#[derive(Default)]struct Effects{replies:Vec<(u32,u64)>,device:Vec<(u64,u64,u64)>,fail_next_read:bool}
static EFFECTS:Mutex<Effects>=Mutex::new(Effects{replies:Vec::new(),device:Vec::new(),fail_next_read:false});
static INBOX:Mutex<Option<mpsc::Receiver<Message>>>=Mutex::new(None);
static TEST_LOCK:Mutex<()>=Mutex::new(());
fn hot_log()->bool{false}
fn pdx_listen_raw(slot:u64)->Message{assert_eq!(slot,0);INBOX.lock().unwrap().as_ref().unwrap().recv().unwrap()}
fn pdx_reply(caller:u32,value:u64){EFFECTS.lock().unwrap().replies.push((caller,value));}
fn block_buffer_view(caller:u32,slot:u64,size:u64,_rights:u8)->Option<RegionView>{
 if caller==27&&slot==17&&size>0&&size<=4096{Some(RegionView{virtual_address:0x400000})}else{None}
}
fn typed_read_handoff(offset:u64,size:u64,_slot:u64,_view:Option<RegionView>)->u64{
 if !unsafe{NVME_IO_STATE.ready}{return BLOCK_ERR_NO_DEVICE}
 let mut e=EFFECTS.lock().unwrap();e.device.push((BLOCK_READ,offset,size));
 if e.fail_next_read{e.fail_next_read=false;unsafe{NVME_IO_STATE.ready=false;}return BLOCK_ERR_NO_DEVICE}0
}
fn write_guard_allows(offset:u64,size:u64,slot:u64)->bool{slot==17&&size==512&&offset==400*512}
fn nvme_write_one_block(offset:u64,size:u64,_src:u64)->u64{EFFECTS.lock().unwrap().device.push((BLOCK_WRITE,offset,size));0}
fn nvme_write_readback_proof(_:u64,_:u64,_:u64)->u64{panic!("not a proof request")}
fn nvme_flush()->u64{EFFECTS.lock().unwrap().device.push((BLOCK_SYNC,0,0));0}
include!(env!("SEXDRIVE_SERVICE_SOURCE"));
fn setup()->mpsc::Sender<Message>{
 *EFFECTS.lock().unwrap()=Effects::default();unsafe{NVME_IO_STATE.ready=true;}
 let (tx,rx)=mpsc::channel();*INBOX.lock().unwrap()=Some(rx);tx
}
fn msg(op:u64)->Message{Message{type_id:op,caller_pd:27,arg0:400*512,arg1:512,arg2:17}}
#[test]fn idle_service_waits_then_handles_a_real_queued_request(){
 let _guard=TEST_LOCK.lock().unwrap();let tx=setup();let(start_tx,start_rx)=mpsc::channel();
 let worker=std::thread::spawn(move||{start_tx.send(()).unwrap();service_next_request();});start_rx.recv().unwrap();
 assert!(EFFECTS.lock().unwrap().device.is_empty());assert!(EFFECTS.lock().unwrap().replies.is_empty());
 tx.send(msg(BLOCK_READ)).unwrap();worker.join().unwrap();
 assert_eq!(EFFECTS.lock().unwrap().replies,[(27,0)]);
}
#[test]fn queued_read_write_flush_and_unknown_request_complete_without_demo_work(){
 let _guard=TEST_LOCK.lock().unwrap();let tx=setup();
 for op in [BLOCK_READ,BLOCK_WRITE,BLOCK_SYNC,99]{tx.send(msg(op)).unwrap();}
 for _ in 0..4{service_next_request();}
 let e=EFFECTS.lock().unwrap();assert_eq!(e.device,[(BLOCK_READ,400*512,512),(BLOCK_WRITE,400*512,512),(BLOCK_SYNC,0,0)]);
 assert_eq!(e.replies,[(27,0),(27,0),(27,0),(27,BLOCK_ERR_BAD_CMD)]);
}
#[test]fn real_read_opcode_is_not_mistaken_for_a_scalar_receipt_kind(){
 let _guard=TEST_LOCK.lock().unwrap();let tx=setup();assert_eq!(BLOCK_READ,1);
 tx.send(Message{caller_pd:3,arg1:0,arg2:0,..msg(1)}).unwrap();tx.send(msg(BLOCK_READ)).unwrap();
 service_next_request();service_next_request();assert_eq!(EFFECTS.lock().unwrap().replies,[(3,BLOCK_ERR_BAD_LEN),(27,0)]);
}
#[test]fn failed_read_keeps_service_alive_and_next_request_sees_stopped_device(){
 let _guard=TEST_LOCK.lock().unwrap();let tx=setup();EFFECTS.lock().unwrap().fail_next_read=true;
 tx.send(msg(BLOCK_READ)).unwrap();tx.send(msg(BLOCK_READ)).unwrap();service_next_request();service_next_request();
 let e=EFFECTS.lock().unwrap();assert_eq!(e.device.len(),1);assert_eq!(e.replies,[(27,BLOCK_ERR_NO_DEVICE),(27,BLOCK_ERR_NO_DEVICE)]);
}
#[test]fn peer_slot_and_length_rejections_never_reach_device(){
 let _guard=TEST_LOCK.lock().unwrap();let tx=setup();
 for m in [Message{caller_pd:9,..msg(BLOCK_READ)},Message{arg2:99,..msg(BLOCK_WRITE)},Message{arg1:4097,..msg(BLOCK_READ)},Message{arg0:1,..msg(BLOCK_READ)}]{tx.send(m).unwrap();service_next_request();}
 let e=EFFECTS.lock().unwrap();assert!(e.device.is_empty());assert_eq!(e.replies,[(9,BLOCK_ERR_NO_DEVICE),(27,BLOCK_ERR_NO_DEVICE),(27,BLOCK_ERR_BAD_LEN),(27,BLOCK_ERR_BAD_LEN)]);
}
#[test]fn initial_frame_fills_only_the_boot_buffer_before_publication(){
 let _guard=TEST_LOCK.lock().unwrap();let n=1024*768;let mut storage=vec![0xdeadbeefu32;n+2];fill_initial_frame(unsafe{storage.as_mut_ptr().add(1)}as u64);
 assert_eq!(storage[0],0xdeadbeef);assert_eq!(storage[n+1],0xdeadbeef);assert_eq!(storage[1],1);assert_eq!(storage[n],(1023^767)+1);
 assert!(!storage[1..=n].contains(&0xdeadbeef));
}
