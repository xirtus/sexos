"""Actual SDK front dispatch preserves authenticated child receipts while UI waits."""
import re,shutil,subprocess,tempfile,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class WindowLaunch(unittest.TestCase):
 def test_actual_dispatch_and_classifier(self):
  source=(ROOT/'crates/sex-app/src/lib.rs').read_text()
  classifier=re.search(r'pub\(crate\) fn is_event\(m: &sex_pdx::PdxMessage\) -> bool \{[^}]+\}',source)[0]
  is_shell=re.search(r'fn is_shell\(m: &sex_pdx::PdxMessage\) -> bool \{[^}]+\}',source)[0]
  begin=source.index('    fn event(&mut self, m: &sex_pdx::PdxMessage) -> Option<Event> {')
  end=source.index("        // Another task's answer",begin)
  prefix=source[begin:end]
  self.assertIn('mailbox::take(|m|is_shell(m)&&!sex_pdx::autofill::response(m.type_id))',source)
  self.assertEqual(prefix.count('if launch::is_event(m)'),1)
  rust='''
const SHELL:u32=3;
mod sex_pdx{#[derive(Clone,Copy)]pub struct PdxMessage{pub caller_pd:u32,pub caller_task:u32,pub type_id:u64,pub arg0:u64,pub arg1:u64,pub arg2:u64}pub mod autofill{pub fn response(op:u64)->bool{op==0xfeed}}}
mod mailbox{use super::sex_pdx::PdxMessage;pub static QUEUE:std::sync::Mutex<Vec<PdxMessage>>=std::sync::Mutex::new(Vec::new());pub fn put(m:PdxMessage){QUEUE.lock().unwrap().push(m);}}
mod launch{use super::*;mod l{pub const OP_EVENT:u64=0x59;}
'''+classifier+'\n}\n'+is_shell+'''
#[derive(Debug,PartialEq)]enum Event{UiStub}struct Window;
impl Window{
'''+prefix+'''Some(Event::UiStub)}}
#[test]fn preserve_exact_shell_started_and_done(){let mut w=Window;for kind in [1,3]{let m=sex_pdx::PdxMessage{caller_pd:3,caller_task:3,type_id:0x59,arg0:(kind<<32)|9,arg1:(43<<32)|1073741824,arg2:0};assert_eq!(w.event(&m),None);}let mut queue=mailbox::QUEUE.lock().unwrap();assert_eq!(queue.len(),2);assert_eq!(queue[0].arg0>>32,1);assert_eq!(queue[1].arg0>>32,3);assert!(queue.iter().all(|m|!is_shell(m)));assert!(queue.iter().all(launch::is_event));queue.clear();for (pd,task,op)in [(3,4,0x59),(4,3,0x59),(3,3,0x58)]{let m=sex_pdx::PdxMessage{caller_pd:pd,caller_task:task,type_id:op,arg0:0,arg1:0,arg2:0};assert!(!launch::is_event(&m));drop(queue);let _=w.event(&m);queue=mailbox::QUEUE.lock().unwrap();assert!(queue.is_empty());}}
'''
  with tempfile.TemporaryDirectory()as t:
   p=Path(t);(p/'actual.rs').write_text(rust)
   subprocess.run([shutil.which('rustc'),'--edition=2021','--test',str(p/'actual.rs'),'-o',str(p/'test')],check=True,capture_output=True)
   subprocess.run([str(p/'test')],check=True,capture_output=True)
if __name__=='__main__':unittest.main()
