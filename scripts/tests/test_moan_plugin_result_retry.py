"""Compile actual SDK supervisor+Moan guest adapter with deterministic file/launch
fixtures; fail real allocations at result-read or quantization and recover once.
Only target cfg and external transports/nonce are replaced, not method bodies.
"""
import re,shutil,subprocess,tempfile,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class CompletedRetry(unittest.TestCase):
 def test_actual_completed_job_survives_read_and_quantization_oom(self):
  source=(ROOT/'apps/moan/src/plugins.rs').read_text()
  cfg='#[cfg(all(feature = "sexos", target_arch = "x86_64", target_os = "none"))]'
  self.assertEqual(source.count(cfg),2)
  source=source.replace(cfg,'#[cfg(feature="sexos")]')
  code='''
extern crate alloc;extern crate self as moan_core;extern crate self as moan_plugin;extern crate self as sex_app;
pub type Frame=[i16;2];
#[path="JOB"]pub mod job;#[path="SUPERVISOR"]pub mod supervisor;#[path="ADAPTER"]mod plugins;
pub mod nonce{pub fn fresh()->Option<[u8;16]>{Some([8;16])}}
use std::sync::{Mutex,atomic::{AtomicBool,AtomicUsize,AtomicU32,Ordering}};
static OUTPUT:Mutex<Vec<u8>>=Mutex::new(Vec::new());static READY:AtomicBool=AtomicBool::new(false);static READS:AtomicUsize=AtomicUsize::new(0);static LAST_COOKIE:AtomicU32=AtomicU32::new(0);
pub mod files{pub mod direct{use super::super::*;pub struct Stat{pub dir:bool,pub size:u64}pub fn stat(path:&[u8])->Result<Stat,u8>{if path.ends_with(b".out")&&READY.load(Ordering::Relaxed){Ok(Stat{dir:false,size:OUTPUT.lock().unwrap().len()as u64})}else{Err(1)}}pub fn create(_:&[u8])->Result<(),u8>{Ok(())}pub fn sync()->Result<(),u8>{Ok(())}pub fn write_at(_:&[u8],_:u64,bytes:&[u8])->Result<usize,u8>{Ok(bytes.len())}pub fn read_at(_:&[u8],offset:u64,out:&mut[u8])->Result<usize,u8>{READS.fetch_add(1,Ordering::Relaxed);let data=OUTPUT.lock().unwrap();let start=offset as usize;let n=out.len().min(data.len().saturating_sub(start));if n>0{out[..n].copy_from_slice(&data[start..start+n]);}Ok(n)}}}
pub mod launch{use super::*;#[derive(Clone,Copy)]pub enum Event{Started{cookie:u32,row:u32,task:u32},Done{cookie:u32,row:u32,task:u32,status:Option<u32>},Refused{cookie:u32,code:u32}}static QUEUE:Mutex<Vec<Event>>=Mutex::new(Vec::new());pub fn run_hidden(_:&[u8],_:&[u8],cookie:u32)->Result<(),()>{assert_ne!(LAST_COOKIE.swap(cookie,Ordering::Relaxed),cookie,"different Render instances must not reuse cookies");READY.store(true,Ordering::Relaxed);let mut q=QUEUE.lock().unwrap();q.push(Event::Done{cookie,row:43,task:111,status:Some(0)});q.push(Event::Started{cookie,row:43,task:111});Ok(())}pub fn poll(_:u32)->Option<Event>{QUEUE.lock().unwrap().pop()}pub fn kill(_:u32){}}
use std::alloc::{GlobalAlloc,Layout,System};static COUNT:AtomicUsize=AtomicUsize::new(0);static FAIL_AT:AtomicUsize=AtomicUsize::new(usize::MAX);struct Heap;
fn refuse(size:usize)->bool{size>8*1024*1024&&COUNT.fetch_add(1,Ordering::Relaxed)+1==FAIL_AT.load(Ordering::Relaxed)}
unsafe impl GlobalAlloc for Heap{unsafe fn alloc(&self,l:Layout)->*mut u8{if refuse(l.size()){std::ptr::null_mut()}else{System.alloc(l)}}unsafe fn dealloc(&self,p:*mut u8,l:Layout){System.dealloc(p,l)}unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{if refuse(n){std::ptr::null_mut()}else{System.realloc(p,l,n)}}}
#[global_allocator]static HEAP:Heap=Heap;
fn main(){let input=vec![[8192,-16384];job::MAX_FRAMES as usize];let floats=plugins::float_input(&input).unwrap();*OUTPUT.lock().unwrap()=job::encode([8;16],&floats).unwrap();drop(floats);
 for fail in [1,3]{READY.store(false,Ordering::Relaxed);let mut render=plugins::guest::Render::default();render.start(b"@/p",b"@/r",&input,job::Format::Clap,&[],0).unwrap();COUNT.store(0,Ordering::Relaxed);FAIL_AT.store(fail,Ordering::Relaxed);assert!(render.poll(1));assert_eq!(render.status(),plugins::Status::WaitingForMemory);assert!(render.retired());assert!(render.busy());assert!(render.take_result().is_none());assert!(input.iter().all(|f|*f==[8192,-16384]));
 FAIL_AT.store(usize::MAX,Ordering::Relaxed);let reads=READS.load(Ordering::Relaxed);assert!(!render.poll(2));assert_eq!(READS.load(Ordering::Relaxed),reads,"memory-blocked result must not poll-spin");render.retry_result();assert!(render.poll(3));assert_eq!(render.status(),plugins::Status::Ready);assert!(render.retired());assert!(!render.busy());let result=render.take_result().unwrap();assert_eq!(result.frames,input);assert_eq!(result.clipped_samples,0);assert!(render.take_result().is_none());assert_eq!(render.status(),plugins::Status::Idle);}
}
'''
  with tempfile.TemporaryDirectory()as t:
   p=Path(t);(p/'adapter.rs').write_text(source)
   code=code.replace('JOB',str(ROOT/'crates/moan-plugin/src/job.rs')).replace('SUPERVISOR',str(ROOT/'crates/moan-plugin/src/supervisor.rs')).replace('ADAPTER',str(p/'adapter.rs'))
   (p/'retry.rs').write_text(code)
   built=subprocess.run([shutil.which('rustc'),'--edition=2021','--cfg','feature="sexos"','-O',str(p/'retry.rs'),'-o',str(p/'retry')],capture_output=True,text=True)
   self.assertEqual(built.returncode,0,built.stderr)
   result=subprocess.run([str(p/'retry')],capture_output=True,text=True,timeout=30)
   self.assertEqual(result.returncode,0,result.stderr)
if __name__=='__main__':unittest.main()
