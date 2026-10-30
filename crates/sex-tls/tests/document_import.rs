//! Actual document ABI and service policy with an owned, observable test body.
extern crate self as sex_pdx;
#[path="../../sex-pdx/src/document.rs"] pub mod document;
#[path="../../../servers/sexfiles/src/document.rs"] mod service;
#[path="../../../servers/sexfiles/src/document_journal.rs"] mod journal;
#[path="../../../apps/kaleidoscope/src/document_import.rs"] mod import;
pub const SLOT_STORAGE:u64=1;
pub const ERR_SERVICE_NOT_READY:u64=9;
pub const BLOCK_MAX_XFER:u64=4096;
#[macro_export] macro_rules! serial_record {($($arg:tt)*)=>{{}};}
pub fn pdx_call_checked(_:u64,_:u64,_:u64,_:u64,_:u64)->Result<u64,u64>{panic!("host uses explicit transport")}
use std::{cell::Cell,rc::Rc,collections::VecDeque};
use document as w;
use import::{Client,Payload,View};
struct Body { bytes:Vec<u8>, erased:Rc<Cell<usize>> }
impl Payload for Body {fn as_bytes(&self)->&[u8]{&self.bytes}}
impl Drop for Body {fn drop(&mut self){for b in &mut self.bytes {unsafe{core::ptr::write_volatile(b,0);}}assert!(self.bytes.iter().all(|b|*b==0));self.erased.set(self.erased.get()+1);}}
fn body(bytes:&[u8])->(Body,Rc<Cell<usize>>){let erased=Rc::new(Cell::new(0));(Body{bytes:bytes.to_vec(),erased:erased.clone()},erased)}
fn name()->[u8;16]{let mut n=[0;16];n[..8].copy_from_slice(b"page.txt");n}
#[derive(Clone,Copy,Debug,PartialEq,Eq)] enum JournalFault { PostCommitRead, FinalFlush }
struct JournalIo { live:Vec<u8>,durable:Vec<u8>,syncs:usize,fault:JournalFault,triggered:bool }
impl JournalIo {
    fn new(fault:JournalFault)->Self{Self{live:vec![0;journal::CAPACITY as usize],durable:vec![0;journal::CAPACITY as usize],syncs:0,fault,triggered:false}}
}
impl journal::Io for JournalIo {
    fn read(&mut self,offset:u64,out:&mut[u8])->Result<(),u32>{
        if self.fault==JournalFault::PostCommitRead&&!self.triggered&&self.syncs==3&&offset%journal::STRIDE==0&&out.len()==512{self.triggered=true;return Err(w::IO);}
        let at=offset as usize;out.copy_from_slice(&self.live[at..at+out.len()]);Ok(())
    }
    fn write(&mut self,offset:u64,bytes:&[u8])->Result<(),u32>{let at=offset as usize;self.live[at..at+bytes.len()].copy_from_slice(bytes);Ok(())}
    fn sync(&mut self)->Result<(),u32>{
        self.syncs+=1;self.durable.copy_from_slice(&self.live);
        // A completion error cannot prove that the device discarded a write.
        if self.fault==JournalFault::FinalFlush&&!self.triggered&&self.syncs==3{self.triggered=true;return Err(w::IO);}Ok(())
    }
}
struct JournalBackend { cursor:Option<journal::Save>,io:JournalIo }
struct Backend { exists:bool,created:usize,saves:usize,cancels:usize,bytes:Vec<u8>,journal:Option<JournalBackend> }
impl Backend {fn new()->Self{Self{exists:false,created:0,saves:0,cancels:0,bytes:vec![],journal:None}}}
impl service::Backend for Backend {
    fn identity(&mut self,id:u8)->Result<u32,u32>{if id==3&&self.created!=0{Ok(41)}else{Err(w::DENIED)}}
    fn load(&mut self,_:u8,_:&mut[u8;w::MAX])->Result<usize,u32>{Err(w::DENIED)}
    fn name(&mut self,_:u8)->Result<[u8;16],u32>{Ok(name())}
    fn create_new(&mut self,_:[u8;16])->Result<(u8,u32),u32>{if self.exists{return Err(w::EXISTS);}self.created+=1;Ok((3,41))}
    fn cancel_save(&mut self,_:u32){self.cancels+=1;if let Some(j)=self.journal.as_mut(){j.cursor=None;}}
    fn save_step(&mut self,caller:u32,id:u8,bytes:&[u8])->Result<bool,u32>{
        if let Some(j)=self.journal.as_mut(){
            if j.cursor.is_none(){self.saves+=1;j.cursor=Some(journal::Save::new(id,41));}
            j.cursor.as_mut().unwrap().step(&mut j.io,bytes).map(|receipt|receipt.is_some())
        }else{self.save(caller,id,bytes).map(|_|true)}
    }
    fn save(&mut self,_:u32,_:u8,bytes:&[u8])->Result<(),u32>{self.saves+=1;self.bytes=bytes.to_vec();Ok(())}
}
struct World { svc:service::Service,backend:Backend,calls:Vec<w::Request>,accepted:Vec<w::Request>,replies:VecDeque<u64>,full:usize,deny:Option<u8>,raw_deny:Option<u8> }
impl World {
    fn new()->Self{Self{svc:service::Service::new(),backend:Backend::new(),calls:vec![],accepted:vec![],replies:VecDeque::new(),full:0,deny:None,raw_deny:None}}
    fn send(&mut self,r:w::Request)->Result<(),u64>{
        self.calls.push(r);if self.full!=0{self.full-=1;return Err(ERR_SERVICE_NOT_READY);}
        if self.raw_deny==Some(r.kind){self.raw_deny=None;return Err(4);}
        assert!(self.accepted.last().is_none_or(|old|r.seq>old.seq),"admitted mutation replayed");self.accepted.push(r);
        if self.deny==Some(r.kind){self.deny=None;self.replies.push_back(w::reply(r.seq,Err(w::DENIED)));return Ok(());}
        if let Some(word)=self.svc.handle(15,true,r,&mut self.backend){self.replies.push_back(word);}Ok(())
    }
    fn progress(&mut self){self.svc.progress(&mut self.backend);let q=&mut self.replies;self.svc.poll(|caller,word|{assert_eq!(caller,15);q.push_back(word);0});}
    fn turn(&mut self,c:&mut Client<Body>,now:u64,deliver:bool){let before=self.calls.len();c.advance_with(now,|r|self.send(r));assert!(self.calls.len()-before<=4);self.progress();if deliver{while let Some(word)=self.replies.pop_front(){c.reply(word,now);}}}
    fn until(&mut self,c:&mut Client<Body>,kind:u8,start:u64)->u64{for now in start..start+2000{self.turn(c,now,false);if self.accepted.last().is_some_and(|r|r.kind==kind){return now;}while let Some(word)=self.replies.pop_front(){c.reply(word,now);}}panic!("missing kind {kind}")}
    fn finish(&mut self,c:&mut Client<Body>,start:u64){for now in start..start+2000{self.turn(c,now,true);if !c.busy(){return;}}panic!("busy {:?}",c.view());}
}
fn started(bytes:&[u8])->(Client<Body>,Rc<Cell<usize>>){let(mut c,(b,erased))=(Client::new(),body(bytes));assert!(c.start(b,name(),0).is_ok());(c,erased)}

#[test] fn actual_service_saves_all_bytes_once_and_only_after_end(){
    let bytes:Vec<u8>=(0..w::MAX).map(|i|(i%251)as u8).collect();let(mut c,erased)=started(&bytes);let mut world=World::new();
    let now=world.until(&mut c,w::COMMIT,0);assert_ne!(c.view(),View::Saved);assert_eq!(world.backend.bytes,bytes);assert_eq!(c.document().unwrap().as_bytes(),bytes);
    while let Some(word)=world.replies.pop_front(){c.reply(word,now);}assert_eq!(c.view(),View::Saving);assert!(c.busy());
    world.finish(&mut c,now+1);assert_eq!(c.view(),View::Saved);assert_eq!(world.backend.saves,1);assert_eq!(world.backend.created,1);assert_eq!(erased.get(),0);
    let event=c.take_event().unwrap();assert_eq!((event.object,event.generation,event.bytes,event.hash),(3,41,bytes.len(),w::hash(&bytes)));
    let kinds:Vec<_>=world.accepted.iter().map(|r|r.kind).collect();assert_eq!(&kinds[..3],&[w::CREATE,w::INFO,w::BEGIN_SAVE]);assert_eq!(&kinds[kinds.len()-2..],&[w::COMMIT,w::END]);
    assert!(world.calls.windows(4).any(|g|g.iter().all(|r|r.kind==w::WRITE)));c.clear(now+2);assert_eq!(erased.get(),1);assert_eq!(c.view(),View::Idle);
}
#[test] fn full_retries_exact_before_effect_and_has_finite_deadline(){
    let(mut c,erased)=started(b"secret");let mut world=World::new();world.full=2;for now in 0..3{world.turn(&mut c,now,false);}assert_eq!(world.calls.len(),3);assert!(world.calls.iter().all(|r|*r==world.calls[0]));assert_eq!(world.accepted.len(),1);
    c.close();assert_eq!(erased.get(),1);
    let(mut c,erased)=started(b"secret");let mut world=World::new();world.full=usize::MAX;world.turn(&mut c,0,false);world.turn(&mut c,1999,false);world.turn(&mut c,2000,false);assert!(!c.busy());assert_eq!(c.view(),View::Unavailable);assert_eq!(world.calls.len(),2);assert!(world.accepted.is_empty());assert_eq!(erased.get(),1);
}
#[test] fn write_window_full_front_retries_but_admitted_requests_never_repeat(){
    let(mut c,_)=started(&[7;100]);let mut world=World::new();let now=world.until(&mut c,w::BEGIN_SAVE,0);while let Some(word)=world.replies.pop_front(){c.reply(word,now);}
    world.full=1;world.turn(&mut c,now+1,false);let blocked=*world.calls.last().unwrap();assert_eq!(blocked.kind,w::WRITE);
    world.turn(&mut c,now+2,false);assert_eq!(world.calls[world.calls.len()-4],blocked);assert_eq!(c.accepted(),4);
    let count=world.calls.len();world.turn(&mut c,now+3,false);assert_eq!(world.calls.len(),count);while let Some(word)=world.replies.pop_front(){c.reply(word,now+3);}world.finish(&mut c,now+4);assert_eq!(c.view(),View::Saved);
}
#[test] fn wrong_sequence_tag_duplicate_and_before_admission_are_ignored(){
    let(mut c,_)=started(b"body");let mut world=World::new();c.reply(w::reply(1,Ok(3)),0);assert_eq!(c.accepted(),0);world.turn(&mut c,1,false);
    for word in [w::reply(2,Ok(3)),(0x77u64<<56)|(1<<32)|3]{c.reply(word,2);assert_eq!(c.accepted(),1);}
    let receipt=world.replies.pop_front().unwrap();c.reply(receipt,3);c.reply(receipt,3);world.finish(&mut c,4);assert_eq!(c.view(),View::Saved);
}
#[test] fn collision_never_binds_or_sends_end_and_denials_after_create_abort(){
    let(mut c,erased)=started(b"body");let mut world=World::new();world.backend.exists=true;world.finish(&mut c,0);assert_eq!(c.view(),View::Exists);assert_eq!(erased.get(),0);assert!(c.has_document());assert_eq!(world.accepted.len(),1);assert_eq!(c.take_event().unwrap().error,w::EXISTS);c.clear(1);assert_eq!(erased.get(),1);
    for kind in [w::CREATE,w::INFO,w::BEGIN_SAVE,w::WRITE,w::COMMIT,w::END]{let(mut c,erased)=started(b"body");let mut world=World::new();world.deny=Some(kind);world.finish(&mut c,0);assert_eq!(erased.get(),1);assert_ne!(c.view(),View::Saved);assert_eq!(c.view(),if matches!(kind,w::COMMIT|w::END){View::Uncertain}else{View::Unavailable});assert_eq!(world.accepted.last().unwrap().kind,if kind==w::CREATE{w::CREATE}else{w::ABORT});}
}
#[test] fn admitted_commit_timeout_is_sticky_after_late_receipt_and_cleanup(){
    let(mut c,erased)=started(b"persisted");let mut world=World::new();let now=world.until(&mut c,w::COMMIT,0);let count=world.calls.len();world.turn(&mut c,now+2000,false);assert_eq!(c.view(),View::Uncertain);assert!(c.busy());assert_eq!(erased.get(),1);assert!(!c.has_document());
    for at in now+2001..now+2010{world.turn(&mut c,at,false);}assert_eq!(world.calls.len(),count);while let Some(word)=world.replies.pop_front(){c.reply(word,now+2010);}world.finish(&mut c,now+2011);assert_eq!(c.view(),View::Uncertain);assert_eq!(world.backend.saves,1);assert_eq!(world.accepted.last().unwrap().kind,w::ABORT);c.clear(now+2012);assert_eq!(c.view(),View::Uncertain);let(b,_)=body(b"new");assert!(c.start(b,name(),now+2013).is_err());
}
#[test] fn cancelled_window_waits_all_receipts_then_one_fresh_abort(){
    let(mut c,erased)=started(&[42;100]);let mut world=World::new();let now=world.until(&mut c,w::WRITE,0);assert_eq!(c.accepted(),4);let last=world.accepted.last().unwrap().seq;
    c.cancel(now);c.cancel(now+1);assert_eq!(erased.get(),1);assert!(!c.has_document());let count=world.calls.len();world.turn(&mut c,now+2,false);assert_eq!(world.calls.len(),count);
    let first=world.replies.pop_front().unwrap();c.reply(first,now+3);c.reply(first,now+3);assert_eq!(c.accepted(),3);while let Some(word)=world.replies.pop_front(){c.reply(word,now+4);}
    world.turn(&mut c,now+5,false);let abort=world.accepted.last().unwrap();assert_eq!((abort.kind,abort.seq),(w::ABORT,last+1));c.cancel(now+6);world.finish(&mut c,now+7);assert_eq!(c.view(),View::Cancelled);assert_eq!(world.accepted.iter().filter(|r|r.kind==w::ABORT).count(),1);assert_eq!(world.backend.saves,0);
}
#[test] fn precommit_timeout_can_reconcile_and_cancel_before_create_needs_no_abort(){
    let(mut c,erased)=started(b"body");let mut world=World::new();world.turn(&mut c,0,false);world.turn(&mut c,2000,false);assert_eq!(c.view(),View::Uncertain);assert_eq!(erased.get(),1);world.finish(&mut c,2001);assert_eq!(c.view(),View::Unavailable);assert_eq!(world.accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![w::CREATE,w::ABORT]);
    let(mut c,erased)=started(b"body");c.cancel(0);assert!(!c.busy());assert_eq!(c.view(),View::Cancelled);assert_eq!(erased.get(),1);c.advance_with(1,|_|panic!("no route before create"));
}
#[test] fn invalid_write_commit_and_end_sizes_cannot_publish_saved(){
    for(kind,value)in[(w::WRITE,99),(w::COMMIT,99),(w::END,1),(w::INFO,0),(w::BEGIN_SAVE,1)]{let(mut c,erased)=started(b"body");let mut world=World::new();let now=world.until(&mut c,kind,0);let seq=world.accepted.last().unwrap().seq;world.replies.clear();c.reply(w::reply(seq,Ok(value)),now);world.finish(&mut c,now+1);assert_ne!(c.view(),View::Saved);assert_eq!(erased.get(),1);assert_eq!(world.accepted.last().unwrap().kind,w::ABORT);}
}
#[test] fn actual_service_rejects_hash_mismatch_and_existing_object_save(){
    let mut svc=service::Service::new();let mut backend=Backend::new();let mut send=|seq,kind,object,a,b|{let r=w::Request{seq,kind,object,offset:0,a,b};svc.handle(15,true,r,&mut backend);let mut value=None;svc.poll(|_,v|{value=Some(v);0});value.unwrap()};
    assert_eq!(w::decode_reply(send(1,w::BEGIN_SAVE,3,41,4),1),Some(Err(w::DENIED)));
    assert_eq!(w::decode_reply(send(2,w::CREATE,0,u64::from_le_bytes(name()[..8].try_into().unwrap()),0),2),Some(Ok(3)));
    assert_eq!(w::decode_reply(send(3,w::BEGIN_SAVE,3,41,4),3),Some(Ok(0)));
    assert_eq!(w::decode_reply(send(4,w::WRITE,3,u32::from_le_bytes(*b"body")as u64,0),4),Some(Ok(4)));
    assert_eq!(w::decode_reply(send(5,w::COMMIT,3,123,0),5),Some(Err(w::INVALID)));assert_eq!(backend.saves,0);
}
#[test] fn close_erases_without_route_and_late_receipts_cannot_restart(){
    let(mut c,erased)=started(&[8;100]);let mut world=World::new();let now=world.until(&mut c,w::WRITE,0);let calls=world.calls.len();c.close();assert_eq!(erased.get(),1);while let Some(word)=world.replies.pop_front(){c.reply(word,now+1);}c.advance_with(now+2,|_|panic!("closed route"));assert!(!c.busy());assert_eq!(world.calls.len(),calls);assert_eq!(world.backend.saves,0);
}
#[test] fn held_body_custody_new_start_and_sequence_progress_survive_clear(){
    let(mut c,erased)=started(b"first");let mut world=World::new();world.finish(&mut c,0);assert_eq!(c.view(),View::Saved);let old_seq=world.accepted.last().unwrap().seq;
    let(new,new_erased)=body(b"second");let returned=c.start(new,name(),100).err().unwrap();assert_eq!(returned.as_bytes(),b"second");assert_eq!(c.document().unwrap().as_bytes(),b"first");assert_eq!((erased.get(),new_erased.get()),(0,0));drop(returned);assert_eq!(new_erased.get(),1);
    let old=c.take_document().unwrap();assert!(!c.has_document());assert_eq!(erased.get(),0);drop(old);assert_eq!(erased.get(),1);c.clear(101);let(next,_)=body(b"next");assert!(c.start(next,name(),102).is_ok());world.finish(&mut c,103);assert!(world.accepted.iter().any(|r|r.kind==w::CREATE&&r.seq>old_seq));assert_eq!(c.view(),View::Saved);
}
#[test] fn controls_report_actual_admission_and_exact_receipt_only(){
    let(mut c,_)=started(b"body");let mut world=World::new();world.full=1;world.turn(&mut c,0,false);assert!(c.take_control().is_none());
    world.turn(&mut c,1,false);let event=c.take_control().unwrap();assert_eq!((event.phase,event.kind,event.sequence,event.object,event.value),(import::ControlPhase::Admitted,w::CREATE,1,0,None));
    c.reply(w::reply(2,Ok(3)),2);assert!(c.take_control().is_none());let word=world.replies.pop_front().unwrap();c.reply(word,2);let event=c.take_control().unwrap();assert_eq!((event.phase,event.value),(import::ControlPhase::Reply,Some(Ok(3))));c.reply(word,2);assert!(c.take_control().is_none());
    let now=world.until(&mut c,w::WRITE,3);c.take_control();while let Some(word)=world.replies.pop_front(){c.reply(word,now);}assert!(c.take_control().is_none());world.finish(&mut c,now+1);assert_eq!(c.take_control().unwrap().kind,w::END);c.close();assert!(c.take_control().is_none());
}
#[test] fn transport_denials_before_effect_use_cleanup_only_after_create(){
    for kind in [w::CREATE,w::INFO,w::BEGIN_SAVE,w::WRITE,w::COMMIT,w::END]{let(mut c,erased)=started(b"body");let mut world=World::new();world.raw_deny=Some(kind);world.finish(&mut c,0);assert_eq!(erased.get(),1);assert_eq!(c.view(),if kind==w::END{View::Uncertain}else{View::Unavailable});if kind==w::CREATE{assert!(world.accepted.is_empty());}else{assert_eq!(world.accepted.last().unwrap().kind,w::ABORT);}}
}
#[test] fn accepted_abort_timeout_retains_correlation_without_replay(){
    let(mut c,_)=started(b"body");let mut world=World::new();let now=world.until(&mut c,w::BEGIN_SAVE,0);while let Some(word)=world.replies.pop_front(){c.reply(word,now);}c.cancel(now+1);world.turn(&mut c,now+2,false);let abort=*world.accepted.last().unwrap();assert_eq!(abort.kind,w::ABORT);let count=world.calls.len();world.turn(&mut c,now+2002,false);assert_eq!(c.view(),View::Uncertain);assert!(c.busy());world.turn(&mut c,now+3000,false);assert_eq!(world.calls.len(),count);while let Some(word)=world.replies.pop_front(){c.reply(word,now+3001);}assert!(!c.busy());assert_eq!(c.view(),View::Cancelled);
}
#[test] fn malformed_create_still_uses_fresh_abort_without_saving(){
    let(mut c,erased)=started(b"body");let mut world=World::new();world.turn(&mut c,0,false);world.replies.clear();c.reply(w::reply(1,Ok(99)),1);assert_eq!(erased.get(),1);world.finish(&mut c,2);assert_eq!(c.view(),View::Uncertain);assert_eq!(world.accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![w::CREATE,w::ABORT]);assert_eq!(world.backend.saves,0);
}
#[test] fn known_collision_retries_an_explicit_new_name_with_same_owned_body(){
    let(mut c,erased)=started(b"one owned released document");let mut world=World::new();world.backend.exists=true;world.finish(&mut c,0);assert_eq!(c.view(),View::Exists);assert_eq!(world.accepted.len(),1);let first=world.accepted[0];
    let held=c.take_document().unwrap();assert_eq!(held.as_bytes(),b"one owned released document");assert_eq!(erased.get(),0);c.clear(1);let mut other=[0;16];other[..9].copy_from_slice(b"other.txt");world.backend.exists=false;
    assert!(c.start(held,other,2).is_ok());world.finish(&mut c,3);assert_eq!(c.view(),View::Saved);assert_eq!(world.backend.bytes,b"one owned released document");assert_eq!(world.backend.created,1);assert_eq!(world.backend.saves,1);assert_eq!(erased.get(),0);
    let creates:Vec<_>=world.accepted.iter().filter(|r|r.kind==w::CREATE).collect();assert_eq!(creates.len(),2);assert!(creates[1].seq>first.seq);assert_ne!((creates[1].a,creates[1].b),(first.a,first.b));assert!(!world.accepted.iter().any(|r|r.kind==w::ABORT));c.clear(100);assert_eq!(erased.get(),1);
}
fn negative_commit_after_durable_publication(fault:JournalFault){
    let bytes:Vec<_>=(0..5000).map(|n|(n%251)as u8).collect();let(mut c,erased)=started(&bytes);let mut world=World::new();
    world.backend.journal=Some(JournalBackend{cursor:None,io:JournalIo::new(fault)});world.finish(&mut c,0);
    assert_eq!(c.view(),View::Uncertain);assert!(!c.busy());assert!(!c.has_document());assert_eq!(erased.get(),1);assert_eq!(world.backend.saves,1);assert!(world.svc.browser_import_scrubbed());
    let event=c.take_event().unwrap();assert_eq!((event.view,event.error),(View::Uncertain,w::IO));
    assert_eq!(world.accepted.iter().filter(|r|r.kind==w::COMMIT).count(),1);assert_eq!(world.accepted.iter().filter(|r|r.kind==w::ABORT).count(),1);assert!(!world.accepted.iter().any(|r|r.kind==w::END));
    let j=world.backend.journal.as_mut().unwrap();assert!(j.io.triggered);assert_eq!(j.io.syncs,3);assert!(j.cursor.is_none());
    // Reboot observation uses only the bytes that crossed the flush boundary.
    // Actual recovery must see a valid new bank despite the negative receipt.
    j.io.live.copy_from_slice(&j.io.durable);let mut recovered=[0;w::MAX];let receipt=journal::recover(&mut j.io,3,41,&mut recovered).unwrap().unwrap();
    assert_eq!((receipt.generation,receipt.len,receipt.hash),(1,bytes.len(),w::hash(&bytes)));assert_eq!(&recovered[..receipt.len],bytes);
    c.clear(1000);assert_eq!(c.view(),View::Uncertain);let(next,_)=body(b"no silent retry");assert!(c.start(next,name(),1001).is_err());c.advance_with(1002,|_|panic!("accepted COMMIT replayed"));
}
#[test] fn negative_commit_after_successful_flush_and_failed_readback_stays_uncertain(){negative_commit_after_durable_publication(JournalFault::PostCommitRead);}
#[test] fn negative_commit_after_flush_persists_then_reports_error_stays_uncertain(){negative_commit_after_durable_publication(JournalFault::FinalFlush);}
