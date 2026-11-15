"""Compile the actual SDK private-status mailbox consumer and wire decoder."""
from pathlib import Path
import subprocess,tempfile,unittest
ROOT=Path(__file__).resolve().parents[2]
class StatusMailbox(unittest.TestCase):
 def test_actual_consumer_ignores_stale_and_preserves_other_job(self):
  with tempfile.TemporaryDirectory()as d:
   root=Path(d)
   rust='extern crate self as sex_pdx;\n'
   for name,path in [('restricted_helper','crates/sex-pdx/src/restricted_helper.rs'),('restricted_job','crates/sex-pdx/src/restricted_job.rs'),('restricted_status','crates/sex-app/src/restricted_status.rs')]:
    rust+='#[path="'+str(ROOT/path)+'"]mod '+name+';\n'
   rust+=r'''
const SHELL:u32=3;const SLOT_SHELL:u64=6;
#[derive(Clone,Copy)]struct PdxMessage{caller_pd:u32,caller_task:u32,type_id:u64,arg0:u64,arg1:u64,arg2:u64}
static RAW:std::sync::Mutex<Vec<PdxMessage>>=std::sync::Mutex::new(Vec::new());
fn pdx_try_listen_raw(_:u64)->Option<PdxMessage>{let mut q=RAW.lock().unwrap();if q.is_empty(){None}else{Some(q.remove(0))}}
fn pdx_call(_:u64,_:u64,_:u64,_:u64,_:u64)->(u64,u64){(0,0)}
mod mailbox{use super::*;pub static Q:std::sync::Mutex<Vec<PdxMessage>>=std::sync::Mutex::new(Vec::new());pub fn put(m:PdxMessage){Q.lock().unwrap().push(m)}pub fn take(mut p:impl FnMut(&PdxMessage)->bool)->Option<PdxMessage>{let mut q=Q.lock().unwrap();let i=q.iter().position(|m|p(m))?;Some(q.remove(i))}}
fn msg(cookie:u32,epoch:u64)->PdxMessage{let w=restricted_job::Observation{cookie,state:restricted_job::STATUS_UNKNOWN,recovery_complete:true,current:None,shell_epoch:epoch}.words().unwrap();PdxMessage{caller_pd:3,caller_task:3,type_id:restricted_job::OP_STATUS_EVENT,arg0:w[0],arg1:w[1],arg2:w[2]}}
#[test]fn exact_sender_epoch_cookie_and_single_drain(){
 let old=msg(9,1);mailbox::put(old);mailbox::put(msg(10,2));mailbox::put(msg(9,2));
 assert!(restricted_status::poll(9,2).is_some());assert_eq!(mailbox::Q.lock().unwrap().len(),1);assert!(restricted_status::poll(9,2).is_none());assert_eq!(mailbox::Q.lock().unwrap().len(),1);assert!(restricted_status::poll(10,2).is_some());
 for m in [PdxMessage{caller_pd:4,..msg(9,2)},PdxMessage{caller_task:4,..msg(9,2)},PdxMessage{arg0:0,..msg(9,2)},msg(9,1),msg(10,2),msg(9,2)]{RAW.lock().unwrap().push(m);}
 assert!(restricted_status::poll(9,2).is_some());assert!(RAW.lock().unwrap().is_empty());assert!(restricted_status::poll(9,2).is_none());assert!(restricted_status::poll(10,2).is_some());
 assert!(!restricted_status::query(0,None,1));assert!(!restricted_status::query(9,None,0));assert!(restricted_status::query(9,None,2));
}
'''
   (root/'actual.rs').write_text(rust)
   built=subprocess.run(['rustc','--edition=2021','--test',str(root/'actual.rs'),'-o',str(root/'test')],capture_output=True,text=True)
   self.assertEqual(built.returncode,0,built.stdout+built.stderr)
   ran=subprocess.run([str(root/'test')],capture_output=True,text=True)
   self.assertEqual(ran.returncode,0,ran.stdout+ran.stderr)
if __name__=='__main__':unittest.main()
