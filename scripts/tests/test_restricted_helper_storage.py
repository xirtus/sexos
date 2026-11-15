"""Real sexfs snapshot/new-object publication, including adversarial replacement."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest
ROOT = Path(__file__).resolve().parents[2]


class SystemJobStorage(unittest.TestCase):
    def test_actual_filesystem_identity_and_snapshot(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'src').mkdir()
            (root / 'Cargo.toml').write_text('''[package]
name="restricted-storage-test"
version="0.0.0"
edition="2021"
[dependencies]
sha2={version="=0.10.9",default-features=false,features=["force-soft"]}
zeroize={version="=1.9.1",default-features=false,features=["alloc"]}
sexfs={path="''' + str(ROOT / 'crates/sexfs') + '''"}
[workspace]
''')
            code = 'extern crate self as sex_pdx;\n'
            for module, path in [('restricted_helper','crates/sex-pdx/src/restricted_helper.rs'), ('restricted_job','servers/sexfiles/src/restricted_job.rs'), ('storage','servers/sexfiles/src/restricted_storage.rs'), ('restricted_jobs','servers/sexfiles/src/restricted_jobs.rs'), ('wire','crates/sex-pdx/src/restricted_job.rs'), ('native_io','servers/sexfiles/src/restricted_io.rs'), ('restricted_control','servers/sexfiles/src/restricted_control.rs'), ('restricted_load','servers/sexfiles/src/restricted_load.rs'), ('restricted_data','servers/sexfiles/src/restricted_data.rs'), ('bootstrap','apps/moanplug/src/bootstrap.rs')]:
                if module in ('native_io', 'restricted_load', 'restricted_data', 'bootstrap'):
                    io = (ROOT / path).read_text().replace('restricted_job as wire', 'restricted_job_wire as wire').replace('program_store as store', 'program_store_wire as store').replace('sex_pdx::restricted_job::','sex_pdx::restricted_job_wire::').replace('restricted_job::Info','restricted_job_wire::Info')
                    code += 'mod ' + module + ' {\n' + io.replace('//!', '//') + '\n}\n'
                else:
                    code += '#[path="' + str(ROOT / path) + '"] mod ' + module + ';\n'
            code += 'use storage as restricted_storage; use wire as restricted_job_wire;\n'
            code += r'''

mod app_catalog{
 #[derive(PartialEq)]pub enum App{Program}pub struct Row{pub app:App,pub target:u32}
 pub const SPINDLE_TARGETS:[u32;0]=[];pub const INCARNATIONS:[Row;2]=[Row{app:App::Program,target:42},Row{app:App::Program,target:43}];
}
#[path="SDK_FS_PATH"]mod fs;
#[path="MEMORY_REGION_PATH"]mod memory_region_actual;
mod memory_region{pub use super::memory_region_actual::*;pub type Info=MemoryRegionInfo;pub type View=RegionView;}
use core::sync::atomic::{AtomicBool,AtomicU32,AtomicU64,Ordering};
static TASK:AtomicU32=AtomicU32::new(99);static EPOCH:AtomicU64=AtomicU64::new(1);static NOW:AtomicU64=AtomicU64::new(100);static FULL:AtomicBool=AtomicBool::new(true);static LAST:AtomicU64=AtomicU64::new(0);static SENDS:AtomicU32=AtomicU32::new(0);
#[repr(align(4096))]struct Region([u8;256*1024]);static mut REGION:Region=Region([0;256*1024]);static mut REGION_CHILD:Region=Region([0;256*1024]);static CHILD_TASK:AtomicU32=AtomicU32::new(0);static JOB_LIVE:AtomicBool=AtomicBool::new(false);static SCRUBS:AtomicU32=AtomicU32::new(0);
mod crash_log{pub fn self_epoch()->Result<u64,()>{Ok(super::EPOCH.load(super::Ordering::SeqCst))}}
fn civil_time_ms()->Option<u64>{Some(1000)}
#[macro_export]macro_rules! serial_record{($($t:tt)*)=>{{let _=format_args!($($t)*);}}}

fn get_ticks()->u64{NOW.load(Ordering::SeqCst)}
fn pdx_reply(_:u32,word:u64)->u64{if FULL.load(Ordering::SeqCst){return 1}LAST.store(word,Ordering::SeqCst);SENDS.fetch_add(1,Ordering::SeqCst);0}
fn memory_region_info(slot:u64)->Result<memory_region::Info,()>{let image=slot==100;let child=slot==fs::PROVIDER_SLOT_BASE+1;Ok(memory_region::Info{peer_pd:if image{None}else{Some(if child{43}else{42})},region_id:if image{0x4000}else{fs::REGION_ID_BASE+child as u32},length:if image{32*1024*1024}else{fs::REGION_BYTES as u64},rights:7})}
fn memory_region_view(slot:u64,_:u64,_:u64,_:u8)->Result<memory_region::View,()>{let info=memory_region_info(slot)?;Ok(memory_region::View{region_id:info.region_id,length:info.length,virtual_address:if info.peer_pd==Some(43){core::ptr::addr_of_mut!(REGION_CHILD) as u64}else{core::ptr::addr_of_mut!(REGION) as u64},dma_address:None})}
#[path="PROGRAM_STORE_PATH"]mod program_store_wire;
mod program_store{pub fn load_unbound(_:[u64;2])->u64{super::program_store_wire::loaded(4)}}
mod restricted_image{
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Lease{pub serial:u64,pub shell_epoch:u64,pub files_epoch:u64,pub parent:u32,pub parent_task:u32,pub cookie:u32,pub nonce:[u8;16]}
 pub fn attest_and_lease(mut lease:Lease,_:&[u8])->Option<Lease>{lease.serial=11;Some(lease)}pub fn copied(_:Lease,_:u64,_:u64)->bool{true}
}
mod restricted_runtime{
 pub fn staging()->Option<super::restricted_helper::Binding>{
 use sha2::Digest;let words=|h:[u8;32]|core::array::from_fn(|i|u64::from_le_bytes(h[i*8..i*8+8].try_into().unwrap()));
 Some(super::restricted_helper::Binding{generation:7,coordinator_epoch:2,provider_epoch:1,previous:super::restricted_helper::Subject{pd:43,task:43},child:super::restricted_helper::Subject{pd:0,task:0},parent:super::restricted_helper::Subject{pd:42,task:99},cookie:9,nonce:[0x0101010101010101;2],plugin_hash:words(sha2::Sha256::digest(b"original-plugin").into()),input_hash:words(sha2::Sha256::digest(super::tests::request()).into()),ticket:0})
 }
 pub fn denied(pd:u32)->bool{pd==43}pub fn job_live(_:super::restricted_helper::Binding,_:u64)->bool{super::JOB_LIVE.load(super::Ordering::SeqCst)}
 pub fn scrub_retired(_:super::restricted_helper::Subject)->bool{super::SCRUBS.fetch_add(1,super::Ordering::SeqCst);true}
}
mod fs_service{
 use super::*;static mut FS:Option<sexfs::Fs<'static,tests::Disk>>=None;
 pub fn setup(){unsafe{FS=Some(tests::setup())}}
 pub fn live_run(pd:u32,task:u32)->bool{pd==42&&task==TASK.load(Ordering::SeqCst)||pd==43&&task!=0&&task==CHILD_TASK.load(Ordering::SeqCst)}
 pub fn restricted_path(_:restricted_helper::Subject,token:&[u8],out:&mut[u8;fs::PATH_BYTES])->Option<usize>{if !wire::path(token){return None}out[..token.len()-1].copy_from_slice(&token[1..]);Some(token.len()-1)}
 pub fn restricted_storage<T>(action:impl FnOnce(&mut sexfs::Fs<'static,tests::Disk>)->Result<T,restricted_job::Error>)->Result<T,restricted_job::Error>{unsafe{action((&mut *core::ptr::addr_of_mut!(FS)).as_mut().unwrap())}}
}
#[cfg(test)]mod tests{
use super::*;use sexfs::*;use restricted_job::{Job,Blob,Error as JobError};use restricted_helper::{Subject,Binding};
pub struct Disk(Vec<[u8;BLOCK]>);impl Device for Disk{
fn blocks(&self)->u64{self.0.len() as u64}fn read(&mut self,b:u64,out:&mut[u8;BLOCK])->Result<(),()>{*out=self.0[b as usize];Ok(())}
fn write(&mut self,b:u64,data:&[u8;BLOCK])->Result<(),()>{self.0[b as usize]=*data;Ok(())}fn flush(&mut self)->Result<(),()>{Ok(())}}
fn fs()->Fs<'static,Disk>{let n=2048;Fs::format(Disk(vec![[0;BLOCK];n]),Memory{cache:Box::leak(vec![[0;BLOCK];64].into_boxed_slice()),slots:Box::leak(vec![Slot::EMPTY;64].into_boxed_slice()),used:Box::leak(vec![0;n/64].into_boxed_slice()),pending:Box::leak(vec![0;n/64].into_boxed_slice()),scratch:Box::leak(Box::new([0;BLOCK]))}).unwrap()}
pub fn request()->Vec<u8>{let mut b=Vec::new();b.extend_from_slice(b"MOANREQ1");b.extend_from_slice(&[1;16]);for n in [48000u32,1,2,0]{b.extend_from_slice(&n.to_le_bytes());}for n in [0.25f32,-0.5]{b.extend_from_slice(&n.to_le_bytes());}b}
pub fn setup()->Fs<'static,Disk>{let mut fs=fs();fs.mkdir(b"/renders",0).unwrap();let p=fs.create(b"/plugin",0).unwrap();fs.write(p,0,b"original-plugin",0).unwrap();let i=fs.create(b"/input",0).unwrap();fs.write(i,0,&request(),0).unwrap();fs.commit().unwrap();fs}
fn capture(fs:&mut Fs<'static,Disk>)->(storage::Storage,Job){storage::Storage::capture(fs,[b"@/plugin",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],7,Subject{pd:42,task:99},9,[1;16]).unwrap()}
fn ready(job:&mut Job)->Vec<u8>{let words=|h:[u8;32]|core::array::from_fn(|i|u64::from_le_bytes(h[i*8..i*8+8].try_into().unwrap()));let child=Subject{pd:43,task:100};let b=Binding{generation:1,coordinator_epoch:1,provider_epoch:1,previous:child,child,parent:Subject{pd:42,task:99},cookie:9,nonce:[0x0101010101010101;2],plugin_hash:words(job.plugin_hash()),input_hash:words(job.input_hash()),ticket:7};job.bind(b).unwrap();assert_eq!(job.read(child,b,true,Blob::Plugin,0,99).unwrap(),b"original-plugin");job.create(child,b,true,Blob::Output).unwrap();let mut output=request();output[..8].copy_from_slice(b"MOANPCM1");job.write(child,b,true,Blob::Output,0,output.len(),|i|output[i]).unwrap();job.seal(child,b,true).unwrap();output}
#[test]fn trusted_bootstrap_clears_whole_owned_pages_and_rejects_epoch_or_shape_substitution(){
 #[repr(align(4096))]struct Pages([u64;2048]);let mut pages=Box::new(Pages([0xfeed;2048]));let base=pages.0.as_mut_ptr()as u64+4096;
 let meta=memory_region::MemoryRegionInfo{region_id:0x3001,length:8192,rights:7,peer_pd:Some(1)};let view=memory_region::RegionView{region_id:0x3001,length:8192,virtual_address:base,dma_address:None};
 assert!(bootstrap::shape(meta,view,0x3001,true));assert!(!bootstrap::shape(memory_region::MemoryRegionInfo{peer_pd:Some(42),..meta},view,0x3001,true));assert!(!bootstrap::shape(meta,memory_region::RegionView{length:4096,..view},0x3001,true));assert!(!bootstrap::shape(meta,memory_region::RegionView{virtual_address:base+1,..view},0x3001,true));
 unsafe{bootstrap::erase(view)};assert!(pages.0[..512].iter().all(|v|*v==0xfeed));assert!(pages.0[512..1536].iter().all(|v|*v==0));assert!(pages.0[1536..].iter().all(|v|*v==0xfeed));
 let pixel=memory_region::MemoryRegionInfo{region_id:0x4201,length:8*1024*1024,..meta};let pv=memory_region::RegionView{region_id:0x4201,length:pixel.length,..view};assert!(bootstrap::shape(pixel,pv,0x4201,false));assert!(!bootstrap::shape(pixel,memory_region::RegionView{length:4*1024*1024,..pv},0x4201,false));
 let mut fs=setup();let mut jobs=restricted_jobs::Jobs::new();let owner=Subject{pd:42,task:99};let id=jobs.prepare(1,0,0,|id|storage::Storage::capture(&mut fs,[b"@/plugin",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],id,owner,9,[1;16])).unwrap();let e=jobs.get(id).unwrap();assert!(wire::Info::decode(&e.info(false).unwrap()).unwrap().child.is_none());
 let scope=restricted_runtime::staging().unwrap();let child=Subject{pd:43,task:100};e.claim(owner,9,0,1).unwrap();e.bind(Binding{ticket:id as u64,child,..scope},0,1).unwrap();let data=e.info(false).unwrap();let info=wire::Info::decode(&data).unwrap();assert!(bootstrap::scope(info,child,1,2));assert!(!bootstrap::scope(info,Subject{task:101,..child},1,2));assert!(!bootstrap::scope(info,child,2,2));assert!(!bootstrap::scope(info,child,1,3));
 let mut wrong=data;wrong[168..176].fill(0);assert!(wire::Info::decode(&wrong).is_none());let ack=0xd4<<56|(id as u64)<<32|7;assert!(bootstrap::receipt(1,3,3,ack,info));assert!(!bootstrap::receipt(1,3,4,ack,info));assert!(!bootstrap::receipt(1,42,3,ack,info));assert!(!bootstrap::receipt(1,3,3,ack+1,info));
}
#[test]fn copy_fence_requires_exact_scope_and_successful_receipt_admission(){
 let scope=Binding{generation:7,coordinator_epoch:2,provider_epoch:1,previous:Subject{pd:43,task:43},child:Subject{pd:0,task:0},parent:Subject{pd:42,task:99},cookie:9,nonce:[0x0101010101010101;2],plugin_hash:[1;4],input_hash:[2;4],ticket:0};
 let lease=restricted_image::Lease{serial:11,shell_epoch:2,files_epoch:1,parent:42,parent_task:99,cookie:9,nonce:[1;16]};let mut l=restricted_load::Load::new(7,scope,lease,100,0).unwrap();
 assert!(!l.serial([7,22,1]));assert!(!l.copied([7,7,11],|_|panic!("early copy")));let first=l.reply();assert_eq!(first,Some(0xd2<<56|7<<32|100));assert_eq!(l.reply(),first);l.delivered();
 assert!(!l.serial([7,22,2]));assert!(l.serial([7,22,1]));assert_eq!(l.reply(),Some(11));assert!(!l.copied([7,7,11],|_|panic!("serial not admitted")));l.delivered();
 assert!(!l.copied([7,8,11],|_|panic!("wrong generation")));assert!(!l.copied([7,7,12],|_|panic!("wrong lease")));assert!(!l.copied([7,7,11],|_|false));let mut releases=0;assert!(l.copied([7,7,11],|_|{releases+=1;true}));assert!(l.copied([7,7,11],|_|panic!("duplicate release")));assert_eq!(releases,1);
 let b=Binding{ticket:7,child:Subject{pd:43,task:100},..scope};assert!(!l.install(b));l.delivered();assert!(l.install(b));assert!(!l.install(Binding{generation:8,..b}));assert!(!l.install(Binding{input_hash:[3;4],..b}));assert!(!l.valid(2,1));assert!(!l.valid(1,5000));
}
#[test]fn control_credits_are_complete_and_epoch_scoped(){
 let request=[7,Subject{pd:42,task:99}.word(),9];let data=core::array::from_fn(|i|i as u8);let mut c=restricted_control::Control::new(request,1,data,100).unwrap();
 assert!(!c.fetch([7,0,1],100));assert!(!c.complete());let ack=c.reply();assert_eq!(ack,Some(0xd1<<56|7<<32));assert_eq!(c.reply(),ack);c.delivered();
 assert!(!c.fetch([7,1,1],101));assert!(!c.fetch([7,0,2],101));assert!(!c.fetch([8,0,1],101));assert!(c.fetch([7,0,1],101));let first=c.reply();assert_eq!(first,Some(u64::from_le_bytes(data[..8].try_into().unwrap())));assert_eq!(c.reply(),first);assert!(!c.fetch([7,1,1],101));c.delivered();assert!(c.fetch([7,0,1],101));c.delivered();
 for i in 1..22{assert!(c.fetch([7,i,1],102));c.delivered();}assert!(c.complete());assert!(!c.fetch([7,0,1],102));assert!(!c.fetch([7,22,1],102));assert!(!c.repeat_claim(request,1,102));assert!(!c.fetch([7,21,1],5100));
}
#[test]fn tagged_descriptor_and_serial_reject_parts_and_raw_downgrade(){
 let request=[7,Subject{pd:42,task:99}.word(),9];let data=core::array::from_fn(|i|i as u8);let mut c=restricted_control::Control::new(request,1,data,100).unwrap();
 assert!(!c.fetch_tagged([7,0,1],100));c.delivered();assert!(!c.fetch_tagged([7,1<<8,1],101));assert!(c.fetch_tagged([7,0,1],101));let low=c.reply().unwrap();assert_eq!(restricted_helper::unchunk(low,7,0,0,0),Some(u32::from_le_bytes(data[..4].try_into().unwrap())));assert!(!c.fetch_tagged([7,1<<8,1],101));c.delivered();assert!(!c.fetch([7,0,1],101));assert!(!c.repeat_claim(request,1,101));assert!(c.fetch_tagged([7,0,1],101));c.delivered();assert!(c.fetch_tagged([7,1<<8,1],101));c.delivered();assert!(!c.fetch_tagged([7,0,1],101));
 for i in 1..22{for part in 0..2{assert!(c.fetch_tagged([7,i|part<<8,1],102));let chunk=c.reply().unwrap();assert_eq!(restricted_helper::unchunk(chunk,7,0,i as u8,part as u8),Some(u32::from_le_bytes(data[i as usize*8+part as usize*4..i as usize*8+part as usize*4+4].try_into().unwrap())));c.delivered();}}assert!(c.complete());assert!(!c.fetch_tagged([7,21|1<<8,2],102));assert!(!c.fetch_tagged([7,21|1<<8,1],5100));
 let scope=restricted_runtime::staging().unwrap();let lease=restricted_image::Lease{serial:u64::MAX,shell_epoch:2,files_epoch:1,parent:42,parent_task:99,cookie:9,nonce:[1;16]};let mut l=restricted_load::Load::new(7,scope,lease,100,0).unwrap();assert!(!l.serial_tagged([7,22,1]));l.delivered();assert!(!l.serial_tagged([7,22|1<<8,1]));assert!(l.serial_tagged([7,22,1]));l.delivered();assert!(!l.serial([7,22,1]));assert!(!l.repeat([7,7,scope.previous.word()]));assert!(!l.copied([7,7,u64::MAX],|_|panic!("partial serial")));assert!(l.serial_tagged([7,22|1<<8,1]));l.delivered();assert!(!l.serial_tagged([7,22,1]));assert!(l.copied([7,7,u64::MAX],|_|true));
}
#[test]fn actual_native_transport_full_duplicate_and_exact_task_delivery(){
 fs_service::setup();let actor=Subject{pd:42,task:99};let p=wire::Prepare{format:0,cookie:9,nonce:[1;16],paths:[b"@/plugin",b"@/input",b"@/renders/output"]};let mut input=[0;wire::MAX];let n=p.encode(&mut input).unwrap();
 for i in 0..n{unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION) as *mut u8).add(fs::PATH_BYTES+i),input[i])}}
 let req=|seq|fs::Request{seq,kind:wire::KIND,path_len:0,offset:wire::PREPARE,length:n as u64}.args();
 assert!(native_io::message(fs::OP,actor,req(1)));native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),0);
 // Same accepted request while SYS29 is Full does not create another job.
 assert!(native_io::message(fs::OP,actor,req(1)));FULL.store(false,Ordering::SeqCst);native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((1,fs::OK,1)));
 let initial=unsafe{core::slice::from_raw_parts((core::ptr::addr_of!(REGION)as *const u8).add(fs::PATH_BYTES),wire::INFO_BYTES)};let initial=wire::Info::decode(initial).unwrap();assert_eq!((initial.ticket,initial.parent,initial.cookie,initial.nonce,initial.files_epoch,initial.child),(1,actor,9,[1;16],1,None));assert_eq!(initial.output_hash,[0;32]);
 let shell=Subject{pd:3,task:3};let claim=[1,actor.word(),9];let count=SENDS.load(Ordering::SeqCst);assert!(native_io::message(wire::OP_CLAIM,actor,claim));native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),count);
 assert!(native_io::message(wire::OP_CLAIM,shell,claim));FULL.store(true,Ordering::SeqCst);native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),count);FULL.store(false,Ordering::SeqCst);native_io::poll();assert_eq!(LAST.load(Ordering::SeqCst),0xd1<<56|1<<32);
 let mut descriptor=[0;176];for i in 0..22{let mut word=0;for part in 0..2{assert!(native_io::message(wire::OP_DESCRIBE_TAGGED,shell,[1,i|part<<8,1]));native_io::poll();word|=(restricted_helper::unchunk(LAST.load(Ordering::SeqCst),1,0,i as u8,part as u8).unwrap()as u64)<<(part*32);}descriptor[i as usize*8..i as usize*8+8].copy_from_slice(&word.to_le_bytes());}assert!(wire::Info::decode(&descriptor).is_some());
 assert!(native_io::message(wire::OP_LOAD,shell,[1,7,Subject{pd:43,task:43}.word()]));native_io::poll();assert_eq!(LAST.load(Ordering::SeqCst),0xd2<<56|1<<32|4);
 let mut serial=0;for part in 0..2{assert!(native_io::message(wire::OP_DESCRIBE_TAGGED,shell,[1,22|part<<8,1]));native_io::poll();serial|=(restricted_helper::unchunk(LAST.load(Ordering::SeqCst),1,0,22,part as u8).unwrap()as u64)<<(part*32);}assert_eq!(serial,11);
 assert!(native_io::message(wire::OP_COPIED,shell,[1,7,11]));native_io::poll();assert_eq!(LAST.load(Ordering::SeqCst),0xd3<<56|1<<32);
 let child=Subject{pd:43,task:100};CHILD_TASK.store(100,Ordering::SeqCst);let binding=Binding{child,ticket:1,..restricted_runtime::staging().unwrap()};assert!(native_io::install(binding));JOB_LIVE.store(true,Ordering::SeqCst);
 let child_request=|seq,kind,path:&[u8],length|{for (i,b) in path.iter().enumerate(){unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION_CHILD)as *mut u8).add(i),*b)}}fs::Request{seq,kind,path_len:path.len()as u16,offset:0,length}.args()};
 assert!(native_io::message(fs::OP,child,child_request(1,fs::STAT,b"@/other",0)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((1,fs::DENIED,0)));
 assert!(native_io::message(fs::OP,child,child_request(2,fs::READ,b"@/plugin",15)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((2,fs::OK,15)));let read=unsafe{core::slice::from_raw_parts((core::ptr::addr_of!(REGION_CHILD)as *const u8).add(fs::PATH_BYTES),15)};assert_eq!(read,b"original-plugin");
 assert!(native_io::message(fs::OP,child,child_request(3,fs::CREATE,b"@/result",0)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((3,fs::OK,0)));
 assert!(native_io::message(fs::OP,child,child_request(4,fs::TRUNCATE,b"@/result",0)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((4,fs::OK,0)));
 let mut output=request();output[..8].copy_from_slice(b"MOANPCM1");for (i,b) in output.iter().enumerate(){unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION_CHILD) as *mut u8).add(fs::PATH_BYTES+i),*b)}}
 assert!(native_io::message(fs::OP,child,child_request(5,fs::WRITE,b"@/result",output.len() as u64)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((5,fs::OK,output.len() as u32)));
 assert!(native_io::message(fs::OP,child,child_request(6,fs::SYNC,b"",0)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((6,fs::OK,0)));
 fs_service::restricted_storage(|fs|{let st=fs.open(b"/renders/output").unwrap();let mut actual=vec![0;st.size as usize];fs.read(st.id,0,&mut actual).unwrap();assert_eq!(actual,output);Ok(())}).unwrap();
 for i in 0..16{unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION)as *mut u8).add(fs::PATH_BYTES+i),1)}}
 let result=fs::Request{seq:6,kind:wire::KIND,path_len:0,offset:1<<8|wire::RESULT,length:16}.args();assert!(native_io::message(fs::OP,actor,result));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((6,fs::OK,176)));let info=unsafe{core::slice::from_raw_parts((core::ptr::addr_of!(REGION)as *const u8).add(fs::PATH_BYTES),176)};assert_eq!(&info[..8],b"MOANINF2");{use sha2::Digest;assert_eq!(&info[104..136],sha2::Sha256::digest(&output).as_slice());}
 let p2=wire::Prepare{paths:[b"@/plugin",b"@/input",b"@/renders/second"],..p};assert_eq!(p2.encode(&mut input),Some(n));for i in 0..n{unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION)as *mut u8).add(fs::PATH_BYTES+i),input[i])}}
 FULL.store(true,Ordering::SeqCst);assert!(native_io::message(fs::OP,child,child_request(9,fs::READ,b"@/plugin",15)));native_io::poll();let sent=SENDS.load(Ordering::SeqCst);JOB_LIVE.store(false,Ordering::SeqCst);FULL.store(false,Ordering::SeqCst);native_io::poll();
 // The ended job's data reply is never delivered; the parked helper is woken with a bare denial.
 assert_eq!(SENDS.load(Ordering::SeqCst),sent+1);assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((9,fs::DENIED,0)));assert!(native_io::retire(child));assert_eq!(SCRUBS.load(Ordering::SeqCst),1);
 assert!(native_io::message(fs::OP,actor,req(2)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((2,fs::OK,2)));
 // A consumed proof from job1 must not block the next claimed job until timeout.
 assert!(native_io::message(wire::OP_CLAIM,shell,[2,actor.word(),9]));native_io::poll();assert_eq!(LAST.load(Ordering::SeqCst),0xd1<<56|2<<32);

 // Capacity must fail without overwriting either accepted immutable job.
 for i in 0..n{unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION)as *mut u8).add(fs::PATH_BYTES+i),input[i])}}
 assert!(native_io::message(fs::OP,actor,req(3)));FULL.store(true,Ordering::SeqCst);native_io::poll();let before=SENDS.load(Ordering::SeqCst);
 TASK.store(100,Ordering::SeqCst);FULL.store(false,Ordering::SeqCst);native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),before);
 assert!(native_io::message(fs::OP,actor,req(4)));native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),before);
 // New run can receive its own reply, never the former task's pending value.
 let fresh=Subject{pd:42,task:100};assert!(native_io::message(fs::OP,fresh,req(5)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((5,fs::OK,3)));
 // End is idempotent only for absent tickets, never a wrong extant nonce.
 let end=|seq,id:u64,action,nonce:u8|{for i in 0..16{unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION)as *mut u8).add(fs::PATH_BYTES+i),nonce)}}fs::Request{seq,kind:wire::KIND,path_len:0,offset:id<<8|action,length:16}.args()};
 assert!(native_io::message(fs::OP,fresh,end(6,3,wire::CANCEL,2)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((6,fs::DENIED,0)));
 assert!(native_io::message(fs::OP,fresh,end(7,3,wire::CANCEL,1)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((7,fs::OK,0)));
 assert!(native_io::message(fs::OP,fresh,end(8,3,wire::RELEASE,1)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((8,fs::OK,0)));
 for i in 0..n{unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION)as *mut u8).add(fs::PATH_BYTES+i),input[i])}}
 assert!(native_io::message(fs::OP,fresh,req(9)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((9,fs::OK,4)));
 NOW.store(120100,Ordering::SeqCst);native_io::poll();assert!(!native_io::pending());
 assert!(native_io::message(fs::OP,fresh,end(10,4,wire::RELEASE,1)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((10,fs::OK,0)));
 assert!(native_io::message(fs::OP,fresh,end(11,7,wire::CANCEL,1)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((11,fs::OK,0)));assert!(!native_io::pending());
 for (seq,id,nonce)in[(12,0,1),(13,0x1000000,1),(14,7,0)]{assert!(native_io::message(fs::OP,fresh,end(seq,id,wire::CANCEL,nonce)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((seq,fs::INVALID,0)));}
 EPOCH.store(2,Ordering::SeqCst);native_io::poll();assert!(!native_io::pending());
}
#[test]fn jobs_are_exact_owner_one_use_bounded_and_expire(){
 let mut fs=setup();let mut jobs=restricted_jobs::Jobs::new();let owner=Subject{pd:42,task:99};
 let first=jobs.prepare(1,100,0,|id|storage::Storage::capture(&mut fs,[b"@/plugin",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],id,owner,9,[1;16])).unwrap();
 let e=jobs.get(first).unwrap();assert_eq!(e.claim(Subject{pd:42,task:100},9,100,1),Err(JobError::Identity));assert_eq!(e.claim(owner,8,100,1),Err(JobError::Identity));e.claim(owner,9,100,1).unwrap();assert_eq!(e.claim(owner,9,100,1),Err(JobError::Identity));
 assert!(!jobs.release(first,owner,[2;16],101,1));assert!(!jobs.release(first,Subject{pd:42,task:100},[1;16],101,1));
 jobs.prepare(1,100,1,|id|storage::Storage::capture(&mut fs,[b"@/plugin",b"@/input",b"@/renders/other"],[b"/plugin",b"/input",b"/renders/other"],id,owner,9,[1;16])).unwrap();
 let mut called=false;assert_eq!(jobs.prepare(1,100,0,|_|{called=true;Err(JobError::State)}),Err(JobError::Budget));assert!(!called);
 jobs.sweep(120100,1,|_|true,|_|true);assert!(!jobs.pending());
 let next=jobs.prepare(1,120100,0,|id|storage::Storage::capture(&mut fs,[b"@/plugin",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],id,owner,9,[1;16])).unwrap();assert!(next>first);
 jobs.sweep(120101,2,|_|true,|_|true);assert!(!jobs.pending());
}
#[test]fn exact_bounded_wire_rejects_ambiguous_paths_and_reserved_bytes(){
 let mut bytes=[0;wire::MAX];let p=wire::Prepare{format:0,cookie:9,nonce:[1;16],paths:[b"@/plugin",b"@/input",b"@/renders/output"]};let n=p.encode(&mut bytes).unwrap();assert_eq!(wire::Prepare::decode(&bytes[..n]).unwrap().paths,p.paths);
 for index in [9,38,47]{let mut wrong=bytes;wrong[index]=1;assert!(wire::Prepare::decode(&wrong[..n]).is_none());}
 for path in [b"@/".as_slice(),b"@/a//b",b"@/a/../b",b"@/a b",b"relative",b"@/a/"]{assert!(!wire::path(path));}
 assert!(wire::Prepare::decode(&bytes[..n-1]).is_none());bytes[n]=0;assert!(wire::Prepare::decode(&bytes[..n+1]).is_none());
}
#[test]fn input_mutation_cannot_change_captured_bytes_new_output_matches_private_seal(){
 let mut fs=setup();let (disk,mut job)=capture(&mut fs);let plugin=fs.open(b"/plugin").unwrap().id;fs.truncate(plugin,0,1).unwrap();fs.write(plugin,0,b"replacement",1).unwrap();fs.commit().unwrap();let output=ready(&mut job);
 disk.publish(&mut fs,job.publication_bytes().unwrap(),2,||true).unwrap();job.published();let stat=fs.open(b"/renders/output").unwrap();let mut data=vec![0;stat.size as usize];fs.read(stat.id,0,&mut data).unwrap();assert_eq!(data,output);assert_eq!(job.result(Subject{pd:42,task:99},7,[1;16]).unwrap().0,output);
}
#[test]fn occupied_or_replaced_directory_never_redirects_writes(){
 let mut fs=setup();let (disk,mut job)=capture(&mut fs);ready(&mut job);let id=fs.create(b"/renders/output",1).unwrap();fs.write(id,0,b"foreign",1).unwrap();fs.commit().unwrap();assert_eq!(disk.publish(&mut fs,job.publication_bytes().unwrap(),2,||true),Err(JobError::Alias));let mut data=[0;7];fs.read(id,0,&mut data).unwrap();assert_eq!(&data,b"foreign");
 fs.remove_path(b"/renders/output").unwrap();fs.remove_path(b"/renders").unwrap();fs.mkdir(b"/renders",2).unwrap();fs.commit().unwrap();assert_eq!(disk.publish(&mut fs,job.publication_bytes().unwrap(),3,||true),Err(JobError::Identity));assert!(fs.open(b"/renders/output").is_err());
}
#[test]fn revoked_lease_never_successfully_publishes_and_no_alias_normalization(){
 let mut fs=setup();let (disk,mut job)=capture(&mut fs);ready(&mut job);let mut checks=0;
 assert_eq!(disk.publish(&mut fs,job.publication_bytes().unwrap(),2,||{checks+=1;checks<3}),Err(JobError::Identity));assert!(fs.open(b"/renders/output").is_err());assert!(job.result(Subject{pd:42,task:99},7,[1;16]).is_err());
 for path in [b"relative".as_slice(),b"/a/../b",b"/a//b",b"/a/./b",b"/a/",b"/space here"]{assert!(!storage::canonical(path));}
 let mismatch=storage::Storage::capture(&mut fs,[b"@/other",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],8,Subject{pd:42,task:99},9,[1;16]);assert_eq!(mismatch.err(),Some(JobError::Bounds));
 let result=storage::Storage::capture(&mut fs,[b"@/media/plug",b"@/input",b"@/renders/output"],[b"/media/plug",b"/input",b"/renders/output"],8,Subject{pd:42,task:99},9,[1;16]);assert_eq!(result.err(),Some(JobError::Unsupported));
}
}
'''
            code = code.replace('MEMORY_REGION_PATH', str(ROOT / 'crates/sex-pdx/src/memory_region.rs'))
            code = code.replace('PROGRAM_STORE_PATH', str(ROOT / 'crates/sex-pdx/src/program_store.rs'))
            code = code.replace('SDK_FS_PATH', str(ROOT / 'crates/sex-pdx/src/fs.rs'))
            code = code.replace('->Result<(),()>', '->core::result::Result<(),()>')
            (root / 'src/lib.rs').write_text(code)
            env = os.environ.copy()
            env['CARGO_TARGET_DIR'] = str(ROOT / 'target/restricted-storage-tests')
            result = subprocess.run(['cargo', 'test', '--offline', '--manifest-path', str(root / 'Cargo.toml'), '--lib'], cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
