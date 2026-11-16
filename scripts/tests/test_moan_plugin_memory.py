"""Fault-inject real large allocations in production codecs/region adapter.

The standalone Rust process avoids changing the Python/test-runner allocator.
Existing project and completed-result bytes are allocated before refusal begins.
"""
import shutil,subprocess,tempfile,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class OfflineMemory(unittest.TestCase):
 def test_long_render_refuses_cleanly_and_completed_result_retries(self):
  job=ROOT/'crates/moan-plugin/src/job.rs';adapter=ROOT/'apps/moan/src/plugins.rs'
  code='''
extern crate alloc;
extern crate self as moan_core;
extern crate self as moan_plugin;
pub type Frame=[i16;2];
#[path="JOB"]pub mod job;
#[path="ADAPTER"]mod plugins;
use std::alloc::{GlobalAlloc,Layout,System};
use std::sync::atomic::{AtomicUsize,Ordering};
static LIMIT:AtomicUsize=AtomicUsize::new(usize::MAX);
static BIG:AtomicUsize=AtomicUsize::new(0);
struct Heap;
unsafe impl GlobalAlloc for Heap{
 unsafe fn alloc(&self,l:Layout)->*mut u8{if l.size()>LIMIT.load(Ordering::Relaxed){return std::ptr::null_mut()}if l.size()>8*1024*1024{BIG.fetch_add(1,Ordering::Relaxed);}System.alloc(l)}
 unsafe fn dealloc(&self,p:*mut u8,l:Layout){System.dealloc(p,l)}
 unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{if n>LIMIT.load(Ordering::Relaxed){return std::ptr::null_mut()}if n>8*1024*1024{BIG.fetch_add(1,Ordering::Relaxed);}System.realloc(p,l,n)}
}
#[global_allocator]static HEAP:Heap=Heap;
fn main(){
 let frames=job::MAX_FRAMES as usize;let original=vec![[8192,-16384];frames];let floats=plugins::float_input(&original).unwrap();let nonce=[8;16];let encoded=job::encode(nonce,&floats).unwrap();let request=job::encode_request(nonce,&floats,&[job::Parameter{id:0,value:0.25}]).unwrap();
 let mut complete=job::Job::new(nonce,7,frames as u32).unwrap();let who=job::Identity{row:43,task:111};complete.started(7,who).unwrap();complete.retired(7,who,Some(0)).unwrap();
 LIMIT.store(16*1024*1024,Ordering::Relaxed);
 assert!(unsafe{HEAP.alloc(Layout::from_size_align(32*1024*1024,8).unwrap())}.is_null());
 assert_eq!(plugins::float_input(&original).err(),Some(plugins::Error::Memory));
 assert_eq!(job::encode(nonce,&floats),Err(job::Error::Memory));
 assert_eq!(job::encode_request(nonce,&floats,&[]),Err(job::Error::Memory));
 assert_eq!(job::decode_request(&request,nonce).err(),Some(job::Error::Memory));
 assert_eq!(complete.result(&encoded),Err(job::Error::Memory));assert_eq!(complete.status(),job::Status::Complete);
 LIMIT.store(8*1024*1024,Ordering::Relaxed);assert_eq!(plugins::quantize(&floats,job::Format::Clap).err(),Some(plugins::Error::Memory));
 assert!(original.iter().all(|f|*f==[8192,-16384]));assert!(floats.iter().all(|f|*f==[0.25,-0.5]));
 LIMIT.store(usize::MAX,Ordering::Relaxed);let recovered=complete.result(&encoded).unwrap();assert_eq!(recovered.len(),frames);assert!(recovered.iter().all(|f|*f==[0.25,-0.5]));drop(recovered);
 let audio=plugins::quantize(&floats,job::Format::Clap).unwrap();assert_eq!(audio.frames,original);drop(audio);
 BIG.store(0,Ordering::Relaxed);let final_request=job::encode_request(nonce,&floats,&[job::Parameter{id:0,value:0.25}]).unwrap();assert_eq!(BIG.load(Ordering::Relaxed),1,"request must reserve exactly once without23MBreallocation");assert_eq!(final_request,request);
}
'''.replace('JOB',str(job)).replace('ADAPTER',str(adapter))
  with tempfile.TemporaryDirectory()as t:
   p=Path(t);(p/'memory.rs').write_text(code)
   compiled=subprocess.run([shutil.which('rustc'),'--edition=2021','-O',str(p/'memory.rs'),'-o',str(p/'memory')],text=True,capture_output=True)
   self.assertEqual(compiled.returncode,0,compiled.stderr)
   result=subprocess.run([str(p/'memory')],text=True,capture_output=True,timeout=30)
   self.assertEqual(result.returncode,0,result.stderr)
if __name__=='__main__':unittest.main()
