"""Actual coordinator plus four actual providers: no early activation or raw data."""
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
class CoordinatorProtocol(unittest.TestCase):
 def test_actual_coordinator_and_providers(self):
  with tempfile.TemporaryDirectory() as d:
   root=Path(d);text='extern crate self as sex_pdx;\n'
   for name,path in [('restricted_helper','crates/sex-pdx/src/restricted_helper.rs'),('restricted_helper_journal','crates/sex-pdx/src/restricted_helper_journal.rs'),('restricted_helper_provider','crates/sex-pdx/src/restricted_helper_provider.rs'),('restricted_job','crates/sex-pdx/src/restricted_job.rs'),('coordinator','servers/silk-shell/src/restricted_coordinator.rs')]:
    text+='#[path="'+str(ROOT/path)+'"] mod '+name+';\n'
   text+=r'''
use coordinator::*;use restricted_helper::{Subject,Binding};use restricted_helper_provider::{Provider,Message};use restricted_helper_journal::Journal;use std::collections::VecDeque;
fn s(pd:u32,task:u32)->Subject{Subject{pd,task}}
fn index(service:Service)->usize{match service{Service::Files=>0,Service::Display=>1,Service::Net=>2,Service::Bell=>3}}
struct Context<'a>{epoch:u64,now:u64,child:Subject,replies:&'a mut VecDeque<(Subject,u64)>,service:Service,full:bool}
impl restricted_helper_provider::Platform for Context<'_>{
 fn epoch(&mut self)->Option<u64>{Some(self.epoch)}fn keep(&mut self,_:u64)->bool{true}fn challenge(&mut self)->Option<[u64;2]>{Some([0x1111+index(self.service)as u64,0x2222])}
 fn catalog(&mut self,pd:u32)->Option<Subject>{(pd==43).then_some(self.child)}fn now(&mut self)->u64{self.now}
 fn reply(&mut self,value:u64)->bool{if self.full{return false}self.replies.push_back((s(self.service.pd(),self.service.pd()),value));true}
}
struct Fake{now:u64,own:u64,epochs:[u64;4],providers:[Provider;4],journals:[Journal;4],replies:VecDeque<(Subject,u64)>,sent:Vec<(Service,u64,[u64;3])>,header:[u8;176],child:Subject,parent:Subject,copy:usize,bound:bool,deny_bound:bool,active:usize,refused:usize,kills:Vec<Subject>,retired:bool,started:usize,ready:usize,full:bool,reply_full:bool,ready_full:bool,start_full:bool,reserved:usize}
impl Fake{
 fn new()->Self{let mut f=Self{now:0,own:1,epochs:[1;4],providers:core::array::from_fn(|_|Provider::new()),journals:core::array::from_fn(|_|Journal::new()),replies:VecDeque::new(),sent:Vec::new(),header:[0;176],child:s(43,43),parent:s(42,99),copy:0,bound:false,deny_bound:false,active:0,refused:0,kills:Vec::new(),retired:false,started:0,ready:0,full:false,reply_full:false,ready_full:false,start_full:false,reserved:0};
  f.header[..8].copy_from_slice(b"MOANINF2");f.header[8..12].copy_from_slice(&7u32.to_le_bytes());f.header[12..16].copy_from_slice(&1u32.to_le_bytes());f.header[16..24].copy_from_slice(&1u64.to_le_bytes());f.header[24..40].fill(7);f.header[40..72].fill(1);f.header[72..104].fill(2);f.header[136..144].copy_from_slice(&f.parent.word().to_le_bytes());f.header[164..168].copy_from_slice(&9u32.to_le_bytes());
  for i in 0..4{let mut ctx=Context{epoch:1,now:0,child:f.child,replies:&mut f.replies,service:[Service::Files,Service::Display,Service::Net,Service::Bell][i],full:false};f.providers[i].initialize(&f.journals[i],&mut ctx);}f
 }
 fn flush(&mut self){for i in 0..4{let mut ctx=Context{epoch:self.epochs[i],now:self.now,child:self.child,replies:&mut self.replies,service:[Service::Files,Service::Display,Service::Net,Service::Bell][i],full:self.reply_full};self.providers[i].poll(&mut ctx,|_|true);}}
 fn step(&mut self,c:&mut Coordinator){self.now+=1;self.flush();c.poll(self);if let Some((who,value))=self.replies.pop_front(){assert!(!c.reply(who,0xf5<<56|1<<32,self));assert!(!c.reply(who,0x5343_0000_0000_0001,self));assert!(!c.reply(who,0,self));assert!(c.reply(who,value,self));}}
 fn run(&mut self,c:&mut Coordinator){for _ in 0..1500{self.step(c);if !c.busy(){return}}panic!("bounded coordinator did not finish")}
}
impl Platform for Fake{
 fn now(&mut self)->u64{self.now}fn self_epoch(&mut self)->Option<u64>{Some(self.own)}fn peer_epoch(&mut self,service:Service)->Option<u64>{Some(self.epochs[index(service)])}
 fn live(&mut self,who:Subject)->bool{who==self.parent||(self.active>0&&who==self.child&&!self.retired)}
 fn reserve(&mut self)->Option<Reservation>{self.reserved+=1;Some(Reservation{previous:self.child,slot:250,fresh:true,generation:1})}
 fn bound(&mut self,r:Reservation,child:Subject)->bool{assert_eq!(r.previous,s(43,43));self.bound=child==self.child&&!self.deny_bound;self.bound}
 fn send(&mut self,service:Service,opcode:u64,words:[u64;3])->bool{
  if self.full{return false}self.sent.push((service,opcode,words));let i=index(service);
  if matches!(opcode,restricted_helper::OP_POLICY_TAGGED|restricted_helper::OP_FETCH_TAGGED){let mut ctx=Context{epoch:self.epochs[i],now:self.now,child:self.child,replies:&mut self.replies,service,full:self.reply_full};assert!(self.providers[i].message(Message{opcode,caller:s(3,3),words},&self.journals[i],&mut ctx));self.providers[i].poll(&mut ctx,|_|true);return true}
  assert!(service==Service::Files);let value=match opcode{
   restricted_job::OP_CLAIM=>{assert_eq!(words,[7,self.parent.word(),9]);0xd1<<56|7<<32},
   restricted_job::OP_DESCRIBE_TAGGED=>{let selector=(words[1]&255)as usize;let part=((words[1]>>8)&1)as u8;assert_eq!(words[0],7);assert_eq!(words[2],1);let value=if selector==22{11}else{u64::from_le_bytes(self.header[selector*8..selector*8+8].try_into().unwrap())};restricted_helper::chunk(7,0,selector as u8,part,(value>>(part as u32*32))as u32).unwrap()},
   restricted_job::OP_LOAD=>{assert_eq!(words,[7,1,s(43,43).word()]);assert!(self.providers.iter().all(|p|p.staging(self.now).is_some()));0xd2<<56|7<<32|100},
   restricted_job::OP_COPIED=>{assert_eq!(words,[7,1,11]);assert_eq!(self.copy,1);assert!(self.bound);0xd3<<56|7<<32},
   _=>panic!("raw/unknown control endpoint"),
  };self.replies.push_back((s(11,11),value));true
 }
 fn copy(&mut self,_:Reservation,length:u32)->Option<Subject>{assert_eq!(length,100);assert!(self.providers.iter().all(|p|p.staging(self.now).is_some()));self.copy+=1;self.child=s(43,0x40000001);Some(self.child)}
 fn activate(&mut self,_:Reservation,child:Subject)->Activation{assert_eq!(child,self.child);assert!(self.bound);assert_eq!(self.copy,1);for p in &self.providers{let b=p.binding().expect("all providers must be sealed before activation");assert_eq!(b.child,child);assert!(p.job_live(b,self.now));}self.active+=1;Activation::Active}
 fn started(&mut self,parent:Subject,cookie:u32,child:Subject)->bool{assert_eq!((parent,cookie,child),(self.parent,9,self.child));if self.start_full{return false}self.started+=1;true}
 fn refused(&mut self,_:Subject,_:u32){self.refused+=1;}
 fn ready(&mut self,child:Subject,value:u64)->bool{assert_eq!(child,self.child);assert_eq!(value,0xd4<<56|7<<32|1);if self.ready_full{return false}self.ready+=1;true}
 fn kill(&mut self,child:Subject){self.kills.push(child);}fn retired(&mut self,child:Subject)->bool{self.retired&&child==self.child}
}
#[test]fn full_four_provider_exchange_copy_and_ready_are_causally_ordered(){
 let mut f=Fake::new();let mut c=Coordinator::new();let parent=f.parent;assert!(c.begin(parent,7,9,1,&mut f));f.run(&mut c);assert_eq!(f.active,1);assert_eq!(f.copy,1);assert_eq!(f.reserved,1);assert_eq!(f.refused,0);c.poll(&mut f);assert_eq!(f.started,1);
 assert!(c.begin(parent,7,9,1,&mut f));assert!(!c.begin(parent,8,9,1,&mut f));assert_eq!(f.reserved,1);
 assert!(!c.ready(s(43,55),[1,0x0707070707070707;2],&mut f));
 assert!(!c.ready(f.child,[2,0x0707070707070707,0x0707070707070707],&mut f));assert!(!c.ready(f.child,[1,0,0],&mut f));
 assert!(c.ready(f.child,[1,0x0707070707070707,0x0707070707070707],&mut f));f.ready_full=true;c.poll(&mut f);assert_eq!(f.ready,0);f.ready_full=false;c.poll(&mut f);assert_eq!(f.ready,1);
 f.now=120_001;c.poll(&mut f);assert!(!f.kills.is_empty());assert!(f.kills.iter().all(|who|*who==f.child));
}
#[test]fn copied_child_is_exactly_cleaned_if_post_copy_binding_fails(){
 let mut f=Fake::new();f.deny_bound=true;let mut c=Coordinator::new();let parent=f.parent;assert!(c.begin(parent,7,9,1,&mut f));f.run(&mut c);assert_eq!(f.copy,1);assert_eq!(f.active,0);assert_eq!(f.refused,1);assert_eq!(f.kills,vec![f.child]);
}
#[test]fn parent_start_codec_preserves_full_cookie_and_both_epochs(){
 let words=restricted_job::start_words(0xffffff,u32::MAX,u64::MAX,u64::MAX).unwrap();assert_eq!(restricted_job::start_decode(words),Some((0xffffff,u32::MAX,u64::MAX,u64::MAX)));assert!(restricted_job::start_decode([words[0]|1<<63,1,1]).is_none());assert!(restricted_job::start_words(0,1,1,1).is_none());assert!(restricted_job::start_words(1,0,1,1).is_none());assert!(restricted_job::start_words(1,1,0,1).is_none());assert!(restricted_job::start_words(1,1,1,0).is_none());
}
#[test]fn wrong_owner_and_stale_epoch_never_reserve_or_activate(){
 let mut f=Fake::new();let mut c=Coordinator::new();let parent=f.parent;assert!(!c.begin(parent,7,9,2,&mut f));f.header[136..144].copy_from_slice(&s(42,100).word().to_le_bytes());assert!(c.begin(parent,7,9,1,&mut f));f.run(&mut c);assert_eq!(f.reserved,0);assert_eq!(f.active,0);assert_eq!(f.refused,1);
 let mut f=Fake::new();let mut c=Coordinator::new();let parent=f.parent;assert!(c.begin(parent,7,9,1,&mut f));f.step(&mut c);f.epochs[2]=2;c.poll(&mut f);assert!(!c.busy());assert_eq!(f.active,0);assert_eq!(f.refused,1);
}
#[test]fn admission_full_and_foreign_replies_do_not_advance_then_deadline_refuses(){
 let mut f=Fake::new();let mut c=Coordinator::new();let parent=f.parent;assert!(c.begin(parent,7,9,1,&mut f));f.full=true;c.poll(&mut f);assert!(f.sent.is_empty());assert!(!c.reply(s(11,11),0xd1<<56|7<<32,&mut f));f.full=false;c.poll(&mut f);let (who,value)=f.replies.pop_front().unwrap();assert!(!c.reply(s(11,12),value,&mut f));assert!(c.busy());assert!(c.reply(who,value,&mut f));c.poll(&mut f);let (who,value)=f.replies.pop_front().unwrap();let stale=restricted_helper::chunk(8,0,0,0,0).unwrap();assert!(c.reply(who,stale,&mut f));assert_eq!(f.reserved,0);assert!(c.reply(who,value,&mut f));f.now=5000;c.poll(&mut f);assert!(!c.busy());assert_eq!(f.active,0);assert_eq!(f.refused,1);
}
#[test]fn started_backpressure_survives_early_retirement_and_current_rpc_retries(){
 let mut f=Fake::new();let mut c=Coordinator::new();let parent=f.parent;assert!(c.begin(parent,7,9,1,&mut f));c.poll(&mut f);f.replies.clear();f.now+=50;c.poll(&mut f);assert_eq!(f.sent.len(),2);let(who,value)=f.replies.pop_front().unwrap();assert!(c.reply(who,value,&mut f));f.run(&mut c);f.start_full=true;f.retired=true;c.poll(&mut f);assert!(c.holds_done(f.child));assert_eq!(f.started,0);f.start_full=false;c.poll(&mut f);assert!(!c.holds_done(f.child));assert_eq!(f.started,1);
}
#[test]fn cancel_and_restart_are_exact_subject_and_kill_only_owned_child(){
 let mut f=Fake::new();let mut c=Coordinator::new();let parent=f.parent;assert!(c.begin(parent,7,9,1,&mut f));assert!(!c.cancel(s(42,100),7,9,1,&mut f));assert!(!c.cancel(parent,7,10,1,&mut f));f.run(&mut c);assert!(!c.cancel(s(41,99),7,9,1,&mut f));assert!(f.kills.is_empty());assert!(c.cancel(parent,7,9,1,&mut f));assert_eq!(f.kills.last(),Some(&f.child));
 let mut f=Fake::new();let mut c=Coordinator::new();let parent=f.parent;assert!(c.begin(parent,7,9,1,&mut f));f.run(&mut c);f.own=2;c.poll(&mut f);assert_eq!(f.kills.last(),Some(&f.child));
}
'''
   text=text.replace('[1,0x0707070707070707;2]','[1,0x0707070707070707,0x0707070707070707]')
   (root/'test.rs').write_text(text)
   built=subprocess.run(['rustc','--edition=2021','--test',str(root/'test.rs'),'-o',str(root/'test')],capture_output=True,text=True)
   self.assertEqual(built.returncode,0,built.stdout+built.stderr)
   ran=subprocess.run([str(root/'test')],capture_output=True,text=True)
   self.assertEqual(ran.returncode,0,ran.stdout+ran.stderr)
if __name__=='__main__':unittest.main()
