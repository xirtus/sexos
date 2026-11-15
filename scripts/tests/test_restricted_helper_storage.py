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
            for module, path in [('restricted_helper','crates/sex-pdx/src/restricted_helper.rs'), ('restricted_job','servers/sexfiles/src/restricted_job.rs'), ('storage','servers/sexfiles/src/restricted_storage.rs'), ('restricted_jobs','servers/sexfiles/src/restricted_jobs.rs'), ('wire','crates/sex-pdx/src/restricted_job.rs'), ('native_io','servers/sexfiles/src/restricted_io.rs')]:
                if module == 'native_io':
                    io = (ROOT / path).read_text().replace('restricted_job as wire', 'restricted_job_wire as wire')
                    code += 'mod native_io {\n' + io.replace('//!', '//') + '\n}\n'
                else:
                    code += '#[path="' + str(ROOT / path) + '"] mod ' + module + ';\n'
            code += 'use storage as restricted_storage; use wire as restricted_job_wire;\n'
            code += r'''

mod app_catalog{
 #[derive(PartialEq)]pub enum App{Program}pub struct Row{pub app:App,pub target:u32}
 pub const SPINDLE_TARGETS:[u32;0]=[];pub const INCARNATIONS:[Row;2]=[Row{app:App::Program,target:42},Row{app:App::Program,target:43}];
}
#[path="SDK_FS_PATH"]mod fs;
mod memory_region{
 pub const QUERY_VIEW:u64=1;pub const READ:u64=2;pub const WRITE:u64=4;
 pub struct Info{pub peer_pd:Option<u32>,pub region_id:u32,pub length:u64,pub rights:u64}
 pub struct View{pub region_id:u32,pub length:u64,pub virtual_address:u64}
}
use core::sync::atomic::{AtomicBool,AtomicU32,AtomicU64,Ordering};
static TASK:AtomicU32=AtomicU32::new(99);static EPOCH:AtomicU64=AtomicU64::new(1);static NOW:AtomicU64=AtomicU64::new(100);static FULL:AtomicBool=AtomicBool::new(true);static LAST:AtomicU64=AtomicU64::new(0);static SENDS:AtomicU32=AtomicU32::new(0);
#[repr(align(4096))]struct Region([u8;256*1024]);static mut REGION:Region=Region([0;256*1024]);
mod crash_log{pub fn self_epoch()->Result<u64,()>{Ok(super::EPOCH.load(super::Ordering::SeqCst))}}
fn get_ticks()->u64{NOW.load(Ordering::SeqCst)}
fn pdx_reply(_:u32,word:u64)->u64{if FULL.load(Ordering::SeqCst){return 1}LAST.store(word,Ordering::SeqCst);SENDS.fetch_add(1,Ordering::SeqCst);0}
fn memory_region_info(_:u64)->Result<memory_region::Info,()>{Ok(memory_region::Info{peer_pd:Some(42),region_id:fs::REGION_ID_BASE,length:fs::REGION_BYTES as u64,rights:7})}
fn memory_region_view(_:u64,_:u64,_:u64,_:u64)->Result<memory_region::View,()>{Ok(memory_region::View{region_id:fs::REGION_ID_BASE,length:fs::REGION_BYTES as u64,virtual_address:core::ptr::addr_of_mut!(REGION) as u64})}
mod restricted_runtime{
 pub fn denied(_:u32)->bool{false}pub fn job_live(_:super::restricted_helper::Binding,_:u64)->bool{false}
}
mod fs_service{
 use super::*;static mut FS:Option<sexfs::Fs<'static,tests::Disk>>=None;
 pub fn setup(){unsafe{FS=Some(tests::setup())}}
 pub fn live_run(pd:u32,task:u32)->bool{pd==42&&task==TASK.load(Ordering::SeqCst)}
 pub fn restricted_path(_:restricted_helper::Subject,token:&[u8],out:&mut[u8;fs::PATH_BYTES])->Option<usize>{if !wire::path(token){return None}out[..token.len()-1].copy_from_slice(&token[1..]);Some(token.len()-1)}
 pub fn restricted_storage<T>(action:impl FnOnce(&mut sexfs::Fs<'static,tests::Disk>)->Result<T,restricted_job::Error>)->Result<T,restricted_job::Error>{unsafe{action((&mut *core::ptr::addr_of_mut!(FS)).as_mut().unwrap())}}
}
#[cfg(test)]mod tests{
use super::*;use sexfs::*;use restricted_job::{Job,Blob,Error as JobError};use restricted_helper::{Subject,Binding};
pub struct Disk(Vec<[u8;BLOCK]>);impl Device for Disk{
fn blocks(&self)->u64{self.0.len() as u64}fn read(&mut self,b:u64,out:&mut[u8;BLOCK])->Result<(),()>{*out=self.0[b as usize];Ok(())}
fn write(&mut self,b:u64,data:&[u8;BLOCK])->Result<(),()>{self.0[b as usize]=*data;Ok(())}fn flush(&mut self)->Result<(),()>{Ok(())}}
fn fs()->Fs<'static,Disk>{let n=2048;Fs::format(Disk(vec![[0;BLOCK];n]),Memory{cache:Box::leak(vec![[0;BLOCK];64].into_boxed_slice()),slots:Box::leak(vec![Slot::EMPTY;64].into_boxed_slice()),used:Box::leak(vec![0;n/64].into_boxed_slice()),pending:Box::leak(vec![0;n/64].into_boxed_slice()),scratch:Box::leak(Box::new([0;BLOCK]))}).unwrap()}
fn request()->Vec<u8>{let mut b=Vec::new();b.extend_from_slice(b"MOANREQ1");b.extend_from_slice(&[1;16]);for n in [48000u32,1,2,0]{b.extend_from_slice(&n.to_le_bytes());}for n in [0.25f32,-0.5]{b.extend_from_slice(&n.to_le_bytes());}b}
pub fn setup()->Fs<'static,Disk>{let mut fs=fs();fs.mkdir(b"/renders",0).unwrap();let p=fs.create(b"/plugin",0).unwrap();fs.write(p,0,b"original-plugin",0).unwrap();let i=fs.create(b"/input",0).unwrap();fs.write(i,0,&request(),0).unwrap();fs.commit().unwrap();fs}
fn capture(fs:&mut Fs<'static,Disk>)->(storage::Storage,Job){storage::Storage::capture(fs,[b"@/plugin",b"@/input",b"@/renders/output"],[b"/plugin",b"/input",b"/renders/output"],7,Subject{pd:42,task:99},9,[1;16]).unwrap()}
fn ready(job:&mut Job)->Vec<u8>{let words=|h:[u8;32]|core::array::from_fn(|i|u64::from_le_bytes(h[i*8..i*8+8].try_into().unwrap()));let child=Subject{pd:43,task:100};let b=Binding{generation:1,coordinator_epoch:1,provider_epoch:1,previous:child,child,parent:Subject{pd:42,task:99},cookie:9,nonce:[0x0101010101010101;2],plugin_hash:words(job.plugin_hash()),input_hash:words(job.input_hash()),ticket:7};job.bind(b).unwrap();assert_eq!(job.read(child,b,true,Blob::Plugin,0,99).unwrap(),b"original-plugin");job.create(child,b,true,Blob::Output).unwrap();let mut output=request();output[..8].copy_from_slice(b"MOANPCM1");job.write(child,b,true,Blob::Output,0,output.len(),|i|output[i]).unwrap();job.seal(child,b,true).unwrap();output}
#[test]fn actual_native_transport_full_duplicate_and_exact_task_delivery(){
 fs_service::setup();let actor=Subject{pd:42,task:99};let p=wire::Prepare{format:0,cookie:9,nonce:[1;16],paths:[b"@/plugin",b"@/input",b"@/renders/output"]};let mut input=[0;wire::MAX];let n=p.encode(&mut input).unwrap();
 for i in 0..n{unsafe{core::ptr::write_volatile((core::ptr::addr_of_mut!(REGION) as *mut u8).add(fs::PATH_BYTES+i),input[i])}}
 let req=|seq|fs::Request{seq,kind:wire::KIND,path_len:0,offset:wire::PREPARE,length:n as u64}.args();
 assert!(native_io::message(fs::OP,actor,req(1)));native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),0);
 // Same accepted request while SYS29 is Full does not create another job.
 assert!(native_io::message(fs::OP,actor,req(1)));FULL.store(false,Ordering::SeqCst);native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((1,fs::OK,1)));
 assert!(native_io::message(fs::OP,actor,req(2)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((2,fs::OK,2)));
 // Capacity must fail without overwriting either accepted immutable job.
 assert!(native_io::message(fs::OP,actor,req(3)));FULL.store(true,Ordering::SeqCst);native_io::poll();let before=SENDS.load(Ordering::SeqCst);
 TASK.store(100,Ordering::SeqCst);FULL.store(false,Ordering::SeqCst);native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),before);
 assert!(native_io::message(fs::OP,actor,req(4)));native_io::poll();assert_eq!(SENDS.load(Ordering::SeqCst),before);
 // New run can receive its own reply, never the former task's pending value.
 let fresh=Subject{pd:42,task:100};assert!(native_io::message(fs::OP,fresh,req(5)));native_io::poll();assert_eq!(fs::decode_reply(LAST.load(Ordering::SeqCst)),Some((5,fs::OK,3)));
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
            code = code.replace('SDK_FS_PATH', str(ROOT / 'crates/sex-pdx/src/fs.rs'))
            code = code.replace('->Result<(),()>', '->core::result::Result<(),()>')
            (root / 'src/lib.rs').write_text(code)
            env = os.environ.copy()
            env['CARGO_TARGET_DIR'] = str(ROOT / 'target/restricted-storage-tests')
            result = subprocess.run(['cargo', 'test', '--offline', '--manifest-path', str(root / 'Cargo.toml'), '--lib'], cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
