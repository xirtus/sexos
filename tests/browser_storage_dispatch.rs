//! Actual legacy dispatch boundary, extracted verbatim by the host gate.
//! Closure observations represent every downstream VFS effect, not an alternate
//! implementation of the policy. Actual typed document tests run separately.
#![allow(dead_code)]
extern crate alloc;extern crate self as spin;extern crate self as sex_pdx;
include!(concat!(env!("APP_RESOURCE_TEST_DIR"),"/diskfs_constant.rs"));
pub struct RwLock<T>(std::sync::RwLock<T>);
impl<T> RwLock<T>{pub const fn new(v:T)->Self{Self(std::sync::RwLock::new(v))}pub fn read(&self)->std::sync::RwLockReadGuard<'_,T>{self.0.read().unwrap()}pub fn write(&self)->std::sync::RwLockWriteGuard<'_,T>{self.0.write().unwrap()}}
mod backends{include!(concat!(env!("APP_RESOURCE_TEST_DIR"),"/backend_trait.rs"));}
#[macro_export]macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
#[path="../servers/sexfiles/src/messages.rs"]mod messages;
#[path="../servers/sexfiles/src/backends/ramfs.rs"]mod ramfs;
use backends::FsBackend;
include!(env!("BROWSER_STORAGE_DISPATCH_SOURCE"));
const OWNED_RAMFS:[u64;8]=[messages::OP_RAMFS_OPEN,messages::OP_RAMFS_READ,messages::OP_RAMFS_WRITE,messages::OP_RAMFS_CLOSE,messages::OP_RAMFS_LIST,messages::OP_RAMFS_STAT,messages::OP_RAMFS_OBJECT_ID,messages::OP_RAMFS_READNAME];
#[derive(Default,Debug,PartialEq)]struct Effects{calls:usize,selection:Option<u64>,creates:usize,writes:usize,deletes:usize,reads:usize,counter:usize}
impl Effects{
 fn dispatch(&mut self,op:u64,a:u64,b:u64,c:u64,caller:u32)->u64{
  self.calls+=1;self.counter+=1;
  match op{messages::OP_DISKFS_SELECT=>self.selection=Some(a),messages::OP_DISKFS_CREATE|messages::OP_RAMFS_OPEN|messages::OP_RAMFS_CREATE_OWNER=>self.creates+=1,messages::OP_DISKFS_WRITE|messages::OP_DISKFS_TRUNCATE|messages::OP_DISKFS_RENAME|messages::OP_RAMFS_WRITE=>self.writes+=1,messages::OP_DISKFS_DELETE=>self.deletes+=1,_=>self.reads+=1}
  op.wrapping_add(a).wrapping_add(b).wrapping_add(c).wrapping_add(caller as u64)
 }
}
#[test]fn browser_legacy_opcodes_full_width_aliases_and_unknown_operations_have_no_effect(){
 let mut state=Effects::default();for op in (0x30u64..=0x4D).chain([0,u64::MAX,0x100]){
  for full in [op,op|(1<<32),op|(1<<63)]{
   if OWNED_RAMFS.contains(&full){continue}
   let result=legacy_dispatch(full,2,0x12345678,0xE8FFFF,15,|op,a,b,c,pd|state.dispatch(op,a,b,c,pd));
   assert_eq!(result,messages::ERR_PERM_DENIED as u64,"op={full:#x}");assert_eq!(state,Effects::default());
  }
 }
}
#[test]fn browser_keeps_only_exact_finite_owned_ramfs_dispatch(){
 let mut state=Effects::default();for op in OWNED_RAMFS{
  let got=legacy_dispatch(op,2,3,5,15,|op,a,b,c,pd|state.dispatch(op,a,b,c,pd));assert_eq!(got,op+2+3+5+15);
 }assert_eq!(state.calls,8);assert_eq!(state.counter,8);
}
#[test]fn allowed_ramfs_operations_preserve_actual_owner_checks_and_retirement(){
 let fs=ramfs::RamFs::new();
 let own=legacy_dispatch(messages::OP_RAMFS_OPEN,0,0,0,15,|_,_,_,_,pd|fs.open(b"browser-private",messages::RAMFS_O_CREATE,0,pd).unwrap());
 let foreign=fs.open(b"quil-private",messages::RAMFS_O_CREATE,0,9).unwrap();fs.write(foreign,0,b"preserve",9).unwrap();
 assert_eq!(legacy_dispatch(messages::OP_RAMFS_WRITE,own,0,0,15,|_,h,off,_,pd|fs.write(h,off,b"owned data",pd).unwrap()),10);
 assert_eq!(legacy_dispatch(messages::OP_RAMFS_OBJECT_ID,own,0,0,15,|_,h,_,_,pd|fs.object_id_for_handle(h,pd).unwrap()),fs.object_id_for_handle(own,15).unwrap());
 assert_eq!(fs.list_at(0,15).unwrap().0,own);assert_eq!(fs.list_at(1,15),None);
 let mut output=[0u8;16];let count=legacy_dispatch(messages::OP_RAMFS_READ,own,0,0,15,|_,h,off,_,pd|fs.read(h,off,&mut output,pd).unwrap());assert_eq!(&output[..count as usize],b"owned data");
 assert_eq!(legacy_dispatch(messages::OP_RAMFS_OPEN,0,0,0,15,|_,_,_,_,pd|fs.open(b"quil-private",messages::RAMFS_O_CREATE,0,pd).unwrap_err() as u64),messages::ERR_PERM_DENIED as u64);
 for op in [messages::OP_RAMFS_READ,messages::OP_RAMFS_READNAME,messages::OP_RAMFS_WRITE,messages::OP_RAMFS_CLOSE,messages::OP_RAMFS_STAT,messages::OP_RAMFS_OBJECT_ID]{
  let error=legacy_dispatch(op,foreign,0,0,15,|op,h,off,_,pd|match op{
   messages::OP_RAMFS_READ|messages::OP_RAMFS_READNAME=>fs.read(h,off,&mut[0;8],pd).unwrap_err(),
   messages::OP_RAMFS_WRITE=>fs.write(h,off,b"changed!",pd).unwrap_err(),
   messages::OP_RAMFS_CLOSE=>fs.close(h,pd).unwrap_err(),messages::OP_RAMFS_STAT=>fs.stat(h,pd).unwrap_err(),
   messages::OP_RAMFS_OBJECT_ID=>fs.object_id_for_handle(h,pd).unwrap_err(),_=>unreachable!()
  } as u64);assert_eq!(error,messages::ERR_PERM_DENIED as u64);
 }
 let mut retained=[0;8];assert_eq!(fs.read(foreign,0,&mut retained,9),Ok(8));assert_eq!(&retained,b"preserve");assert_eq!(fs.stat(foreign,9).unwrap().0,8);
 assert_eq!(fs.retire_subject(15),(1,0));assert_eq!(fs.stat(own,15),Err(messages::ERR_INVALID_HANDLE));assert_eq!(fs.read(foreign,0,&mut retained,9),Ok(8));
}
#[test]fn arbitrary_selected_object_cannot_be_read_written_renamed_or_deleted_by_browser(){
 let mut state=Effects{selection:Some(7),..Effects::default()};let before=Effects{selection:Some(7),..Effects::default()};
 for id in 0..32{for op in [messages::OP_DISKFS_SELECT,messages::OP_DISKFS_WRITE,messages::OP_DISKFS_READ,messages::OP_DISKFS_READ_V2,messages::OP_DISKFS_TRUNCATE,messages::OP_DISKFS_DELETE,messages::OP_DISKFS_RENAME,messages::OP_DISKFS_CREATE]{assert_eq!(legacy_dispatch(op,id,u64::MAX,u64::MAX,15,|op,a,b,c,pd|state.dispatch(op,a,b,c,pd)),messages::ERR_PERM_DENIED as u64);assert_eq!(state,before);}}
}
#[test]fn caller_zero_quil_linen_spindle_and_other_identities_keep_exact_existing_dispatch(){
 for caller in [0,3,7,9,12,16,17,18,31,u32::MAX]{let mut state=Effects::default();
  for op in [0x30,0x38,0x3E,0x42,0x47,0x49,u64::MAX]{let got=legacy_dispatch(op,3,5,7,caller,|op,a,b,c,pd|state.dispatch(op,a,b,c,pd));assert_eq!(got,op.wrapping_add(3+5+7).wrapping_add(caller as u64));}
  assert_eq!(state.calls,7);assert_eq!(state.counter,7);assert_eq!(state.selection,Some(3));
 }
}
#[test]fn production_entry_wraps_all_legacy_work_and_typed_route_precedes_fallback(){
 let vfs=include_str!("../servers/sexfiles/src/vfs.rs");let entry=&vfs[vfs.find("pub fn handle_vfs_message(").unwrap()..];
 let first=entry.find('{').unwrap();assert!(entry[first+1..].trim_start().starts_with("legacy_dispatch(type_id,arg0,arg1,arg2,caller_pd,"));assert!(entry.find("legacy_dispatch(").unwrap()<entry.find("IPC_OPS_TOTAL.fetch_add").unwrap());
 let trampoline=include_str!("../servers/sexfiles/src/trampoline.rs");let typed=trampoline.find("if msg.type_id==sex_pdx::document::OP{").unwrap();let fallback=trampoline.find("let reply = vfs::handle_vfs_message(").unwrap();assert!(typed<fallback);assert!(trampoline[typed..fallback].contains("let live=document_caller_live(caller);"));assert!(trampoline[typed..fallback].contains("}continue;"));
}
