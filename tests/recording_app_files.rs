//! Actual asynchronous recorder Files client with the actual pure FS/Region wire.
extern crate self as sex_pdx;
extern crate self as sex_media;
use std::{alloc::{alloc_zeroed,dealloc,Layout},collections::VecDeque,sync::Mutex};
#[path="../crates/sex-pdx/src/desktop_recording.rs"]pub mod desktop_recording;
#[path="../crates/sex-pdx/src/fs.rs"]pub mod fs;
#[path="../crates/sex-pdx/src/memory_region.rs"]pub mod memory_region;
pub use memory_region::MemoryRegionInfo;
pub mod recording {pub const TURN_BYTES:usize=32768;}
pub mod app_catalog {
    pub const SPINDLE_TARGETS:[u32;2]=[12,35];
    pub struct Row{pub ordinal:u8}
    pub fn by_target(target:u32)->Option<Row>{(target==42).then_some(Row{ordinal:1})}
}
pub const SLOT_STORAGE:u64=1;
pub const ERR_SERVICE_NOT_READY:u64=0x17;
#[derive(Clone,Copy,Debug)]pub struct Peer{pub epoch:u64,pub life:crash_log::PeerLife}
pub mod crash_log {
    #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum PeerLife{Live,Ending}
    pub fn peer_state(slot:u64)->Result<super::Peer,u64>{assert_eq!(slot,super::SLOT_STORAGE);let mut h=super::HOST.lock().unwrap();h.sequence.pop_front().unwrap_or(h.peer).ok_or(1)}
}
#[derive(Clone,Copy)]pub struct PdxMessage{pub type_id:u64,pub arg0:u64,pub arg1:u64,pub arg2:u64,pub caller_pd:u32,pub caller_task:u32}
struct Host{base:usize,peer:Option<Peer>,sequence:VecDeque<Option<Peer>>,calls:Vec<[u64;5]>,results:VecDeque<Result<u64,u64>>,wrong_peer:bool}
static HOST:Mutex<Host>=Mutex::new(Host{base:0,peer:None,sequence:VecDeque::new(),calls:Vec::new(),results:VecDeque::new(),wrong_peer:false});
static SERIAL:Mutex<()>=Mutex::new(());
struct Buffer(usize);
impl Drop for Buffer{fn drop(&mut self){unsafe{dealloc(self.0 as *mut u8,Layout::from_size_align(fs::REGION_BYTES,4096).unwrap())}}}
fn reset()->Buffer{
    let base=unsafe{alloc_zeroed(Layout::from_size_align(fs::REGION_BYTES,4096).unwrap())}as usize;assert_ne!(base,0);
    *HOST.lock().unwrap()=Host{base,peer:Some(Peer{epoch:7,life:crash_log::PeerLife::Live}),sequence:VecDeque::new(),calls:Vec::new(),results:VecDeque::new(),wrong_peer:false};Buffer(base)
}
pub fn memory_region_info(slot:u64)->Result<MemoryRegionInfo,u64>{assert_eq!(slot,fs::CLIENT_SLOT);Ok(MemoryRegionInfo{region_id:fs::REGION_ID_BASE+(fs::PROGRAM_INDEX_BASE+1)as u32,length:fs::REGION_BYTES as u64,rights:7,peer_pd:Some(if HOST.lock().unwrap().wrong_peer{13}else{11})})}
pub fn memory_region_full_view(slot:u64,rights:u8)->Result<memory_region::RegionView,u64>{assert_eq!((slot,rights),(fs::CLIENT_SLOT,7));Ok(memory_region::RegionView{region_id:fs::REGION_ID_BASE+(fs::PROGRAM_INDEX_BASE+1)as u32,virtual_address:HOST.lock().unwrap().base as u64,length:fs::REGION_BYTES as u64,dma_address:None})}
pub fn pdx_call_checked(slot:u64,op:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{let mut h=HOST.lock().unwrap();let result=h.results.pop_front().unwrap_or(Ok(0));if result.is_ok(){h.calls.push([slot,op,a,b,c]);}result}
#[path="../apps/record/src/files.rs"]mod client;
fn answer(sequence:u32,status:u8,value:u32)->PdxMessage{PdxMessage{type_id:1,arg0:fs::reply(sequence,status,value),arg1:0,arg2:0,caller_pd:11,caller_task:11}}
#[test]
fn exclusive_request_retains_bytes_across_queue_full_and_unknown_ack_epoch(){
    let _serial=SERIAL.lock().unwrap();let buffer=reset();let mut f=client::Files::open(7).unwrap();
    f.start(fs::WRITE,b"one.mkv",99,b"encoded").unwrap();assert_eq!(f.start(fs::SYNC,b"",0,b""),Err(client::Error::Busy));
    HOST.lock().unwrap().results.push_back(Err(ERR_SERVICE_NOT_READY));f.poll();assert!(HOST.lock().unwrap().calls.is_empty());f.poll();
    let call=HOST.lock().unwrap().calls[0];let request=fs::Request::decode(call[2],call[3],call[4]).unwrap();assert_eq!((request.kind,request.offset,request.length),(fs::WRITE,99,7));
    assert_eq!(unsafe{std::slice::from_raw_parts((buffer.0+fs::PATH_BYTES)as *const u8,7)},b"encoded");
    assert!(f.receive(&answer(request.seq,fs::OK,7)));HOST.lock().unwrap().peer=None;
    assert!(f.take_reply().is_none());assert!(f.pending());assert!(!f.fenced());
    HOST.lock().unwrap().peer=Some(Peer{epoch:7,life:crash_log::PeerLife::Live});assert_eq!(f.take_reply(),Some(client::Reply{kind:fs::WRITE,status:fs::OK,value:7}));assert!(!f.pending());
}
#[test]
fn old_task_reply_and_files_rearm_cannot_release_current_buffer(){
    let _serial=SERIAL.lock().unwrap();let _buffer=reset();let mut f=client::Files::open(7).unwrap();f.start(fs::CREATE,b"one.mkv",0,b"").unwrap();f.poll();
    let mut forged=answer(1,fs::OK,0);forged.caller_task=42;assert!(!f.receive(&forged));assert!(f.take_reply().is_none());
    f.receive(&answer(1,fs::OK,0));HOST.lock().unwrap().sequence=VecDeque::from([Some(Peer{epoch:7,life:crash_log::PeerLife::Live}),Some(Peer{epoch:8,life:crash_log::PeerLife::Live})]);
    assert!(f.take_reply().is_none());assert!(f.fenced());assert!(f.pending());assert_eq!(f.start(fs::WRITE,b"one.mkv",0,b"x"),Err(client::Error::Fenced));
}
#[test]
fn wrong_binding_or_mutating_file_modes_are_refused(){
    let _serial=SERIAL.lock().unwrap();let _buffer=reset();HOST.lock().unwrap().wrong_peer=true;assert!(matches!(client::Files::open(7),Err(client::Error::Invalid)));HOST.lock().unwrap().wrong_peer=false;let mut f=client::Files::open(7).unwrap();
    for (kind,path,data) in [(fs::REMOVE,&b"one.mkv"[..],&b""[..]),(fs::RENAME,&b"one.mkv"[..],&b""[..]),(fs::CREATE,&b"../one.mkv"[..],&b""[..]),(fs::CREATE,&b"@private.mkv"[..],&b""[..]),(fs::SYNC,&b"one.mkv"[..],&b""[..])] {assert_eq!(f.start(kind,path,0,data),Err(client::Error::Invalid));}
    assert!(HOST.lock().unwrap().calls.is_empty());
}
