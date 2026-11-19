"""Exercise actual parent/adapter allocation failures with finite transports."""
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class CompletedRetry(unittest.TestCase):
    def test_actual_completed_job_survives_read_and_quantization_oom(self):
        source = (ROOT / 'apps/moan/src/plugins.rs').read_text()
        cfg = '#[cfg(all(feature = "sexos", target_arch = "x86_64", target_os = "none"))]'
        self.assertEqual(source.count(cfg), 2)
        source = source.replace(cfg, '#[cfg(feature="sexos")]')
        fixture = ROOT / 'crates/moan-plugin/tests/restricted_supervisor.rs'
        code = fixture.read_text().split('#[test]', 1)[0]
        code = re.sub(r'#\[path\s*=\s*"([^"]+)"\]',
                      lambda m: '#[path=' + json.dumps(str((fixture.parent / m[1]).resolve())) + ']', code)
        code = code.replace('pub use moan_plugin::job;',
                            'pub use plugin_core::job;\nextern crate self as moan_plugin;\n'
                            'extern crate self as moan_core;\npub type Frame=[i16;2];')
        self.assertEqual(code.count('super::read_at(path, at, out)'), 1)
        code = code.replace('super::read_at(path, at, out)',
                            '{ crate::READS.fetch_add(1, Ordering::Relaxed); super::read_at(path, at, out) }')
        code = code.replace('pub mod restricted {',
                            'pub mod restricted { use std::sync::atomic::Ordering;', 1)
        code += r'''
use std::sync::atomic::{AtomicUsize,Ordering};
static READS:AtomicUsize=AtomicUsize::new(0);
static COUNT:AtomicUsize=AtomicUsize::new(0);
static FAIL_AT:AtomicUsize=AtomicUsize::new(usize::MAX);
pub mod nonce{pub fn fresh()->Option<[u8;16]>{Some([7;16])}}
#[path="ADAPTER"]mod plugins;
use std::alloc::{GlobalAlloc,Layout,System};struct Heap;
fn refuse(n:usize)->bool{n>8*1024*1024&&COUNT.fetch_add(1,Ordering::Relaxed)+1==FAIL_AT.load(Ordering::Relaxed)}
unsafe impl GlobalAlloc for Heap{
 unsafe fn alloc(&self,l:Layout)->*mut u8{if refuse(l.size()){std::ptr::null_mut()}else{System.alloc(l)}}
 unsafe fn dealloc(&self,p:*mut u8,l:Layout){System.dealloc(p,l)}
 unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{if refuse(n){std::ptr::null_mut()}else{System.realloc(p,l,n)}}
}
#[global_allocator]static HEAP:Heap=Heap;
fn main(){
 let input=vec![[8192,-16384];job::MAX_FRAMES as usize];let mut last_cookie=0;
 for fail in [1,3]{
  reset();let mut render=plugins::guest::Render::default();
  render.start(b"@/p",b"@/r",&input,job::Format::Clap,&[],0).unwrap();
  let floats=plugins::float_input(&input).unwrap();let encoded=job::encode([7;16],&floats).unwrap();drop(floats);
  with(|w|{
   let info=w.info.as_mut().unwrap();let cookie=info.cookie;assert_ne!(cookie,last_cookie);last_cookie=cookie;
   info.child=Some(Subject{pd:43,task:101});info.generation=1;info.coordinator_epoch=1;
   info.output_hash=Sha256::digest(&encoded).into();
   w.disk.insert(b"@/r/moan-0707070707070707.out".to_vec(),encoded);
   w.events.push_back(launch::Event::Started{cookie,row:43,task:101});
   w.events.push_back(launch::Event::Done{cookie,row:43,task:101,status:Some(0)});
  });
  COUNT.store(0,Ordering::Relaxed);FAIL_AT.store(fail,Ordering::Relaxed);
  assert!(render.poll(1));assert_eq!(render.status(),plugins::Status::WaitingForMemory);
  assert!(render.retired());assert!(render.busy());assert!(render.take_result().is_none());
  assert!(input.iter().all(|f|*f==[8192,-16384]));
  FAIL_AT.store(usize::MAX,Ordering::Relaxed);let reads=READS.load(Ordering::Relaxed);
  assert!(!render.poll(2));assert_eq!(READS.load(Ordering::Relaxed),reads,"memory-blocked result must not poll-spin");
  render.retry_result();assert!(render.poll(3));assert_eq!(render.status(),plugins::Status::Ready);
  assert!(!render.retired());assert!(render.busy(),"accepted result retains cleanup owner");
  let result=render.take_result().unwrap();assert_eq!(result.frames,input);assert_eq!(result.clipped_samples,0);
  assert!(render.take_result().is_none());assert_eq!(render.status(),plugins::Status::Finishing);
  assert!(render.poll(4));assert!(!render.busy());assert!(render.retired());
  assert_eq!(render.status(),plugins::Status::Idle);assert_eq!(with(|w|w.released),1);
 }
}
'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / 'adapter.rs').write_text(source)
            (path / 'src').mkdir()
            (path / 'src/main.rs').write_text(code.replace('ADAPTER', str(path / 'adapter.rs')))
            (path / 'Cargo.toml').write_text(
                '[package]\nname="moan-result-retry-fixture"\nversion="0.1.0"\nedition="2021"\n'
                '[workspace]\n[features]\nsexos=[]\n[dependencies]\n'
                'plugin_core={package="moan-plugin",path=' + json.dumps(str(ROOT / 'crates/moan-plugin')) + '}\n'
                'sha2={version="=0.10.9",default-features=false,features=["force-soft"]}\n')
            command = ['cargo', '+stable', 'run', '--offline', '--release', '--features', 'sexos',
                       '--target-dir', str(ROOT / 'target/moan-result-retry-host'),
                       '--manifest-path', str(path / 'Cargo.toml')]
            result = subprocess.run(command, capture_output=True, text=True, timeout=90)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
