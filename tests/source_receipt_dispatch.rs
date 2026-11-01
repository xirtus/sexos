//! Actual extracted Net dispatch/helpers, actual SDK and source Machines.
//! Finite held-query/provider/IPC callbacks are host controls, not authority.
#![allow(dead_code)]
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod lifecycle_wire;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../crates/sex-pdx/src/entropy_seed.rs"]pub mod entropy_seed;
#[path="../crates/sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../crates/sex-pdx/src/certificate_time.rs"]pub mod certificate_time;
#[path="../crates/sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../crates/sex-pdx/src/trust_policy.rs"]pub mod trust_policy;
#[path="../crates/sex-pdx/src/trust_source.rs"]pub mod trust_source;
#[path="../servers/sexnet/src/entropy.rs"]mod entropy;
#[path="../servers/sexnet/src/certificate_time.rs"]mod clock_machine;
#[path="../servers/sexnet/src/trust_policy.rs"]mod trust_machine;
#[path="../servers/sexnet/src/source_denial.rs"]mod source_denial;
use std::cell::RefCell;
mod jobs{#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Identity{pub pd:u32,pub task:u32}}
#[derive(Clone,Copy)]struct PdxMessage{caller_pd:u32,caller_task:u32,type_id:u64,arg0:u64,arg1:u64,arg2:u64}
#[derive(Clone,Copy)]struct Held{slot:usize,catalog:lifecycle_wire::CatalogInfo,live:bool,catalog_error:Option<u64>,info_override:Option<Result<lifecycle_wire::LifecycleInfo,u64>>}
struct Host{held:[Option<Held>;app_catalog::COUNT],words:usize,times:usize,trusts:usize,seconds:u64,ticks:u64,life_tick_after:Option<u64>,life_calls:usize,trust_consumer:trust_source::Identity,reply_status:u64,replies:Vec<(u32,u64)>,logs:Vec<String>}
impl Host{fn new()->Self{Self{held:[None;app_catalog::COUNT],words:0,times:0,trusts:0,seconds:clock_source::MIN_SECONDS+100,ticks:100,life_tick_after:None,life_calls:0,trust_consumer:trust_source::Identity{pd:15,task:150},reply_status:0,replies:Vec::new(),logs:Vec::new()}}}
thread_local!{static HOST:RefCell<Host>=RefCell::new(Host::new());}
pub mod app_lifecycle{
 pub use crate::lifecycle_wire::{SLOTS,LIVE,ROLE_NET,CatalogInfo,LifecycleInfo};
 pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{crate::HOST.with(|h|h.borrow().held.iter().flatten().find(|e|e.slot==slot).map_or(Err(1),|e|e.catalog_error.map_or(Ok(e.catalog),Err)))}
 pub fn info(slot:usize)->Result<LifecycleInfo,u64>{crate::HOST.with(|h|{let mut h=h.borrow_mut();h.life_calls+=1;if let Some(ticks)=h.life_tick_after{h.ticks=ticks;}h.held.iter().flatten().find(|e|e.slot==slot).map_or(Err(1),|e|e.info_override.unwrap_or_else(||if e.live{Err(LIVE)}else{Ok(LifecycleInfo{target_pd:e.catalog.target_pd,task_id:e.catalog.task_id,required_mask:31,acked_mask:31})}))})}
}
pub fn get_ticks()->u64{HOST.with(|h|h.borrow().ticks)}
fn pdx_reply(pd:u32,word:u64)->u64{HOST.with(|h|{let mut h=h.borrow_mut();h.replies.push((pd,word));h.reply_status})}
pub fn trust_info(slot:u64)->Result<trust_source::Info,u64>{assert_eq!(slot,62);HOST.with(|h|{let mut h=h.borrow_mut();h.trusts+=1;Ok(trust_source::Info{consumer:h.trust_consumer,profile:trust_config::PROFILE,root_bytes:trust_config::ROOT_DER.len(),version:trust_config::VERSION})})}
pub fn clock_snapshot(slot:u64)->Result<clock_source::Snapshot,u64>{assert_eq!(slot,61);HOST.with(|h|{let mut h=h.borrow_mut();h.times+=1;Ok(clock_source::Snapshot{seconds:h.seconds,attempts:1})})}
pub fn entropy_word(slot:u64)->Result<entropy_source::Word,u64>{assert_eq!(slot,60);HOST.with(|h|{let mut h=h.borrow_mut();h.words+=1;Ok(entropy_source::Word{value:h.words as u64,attempts:1})})}
fn record(text:String){HOST.with(|h|h.borrow_mut().logs.push(text));}
#[macro_export]macro_rules! serial_record{($($t:tt)*)=>{$crate::record(format!($($t)*))}}
include!(env!("SOURCE_RECEIPT_PRODUCTION"));

struct Services{denials:source_denial::Outbox,seed:entropy::Machine,clock:clock_machine::Machine,trust:trust_machine::Machine,seed_info:Option<entropy_source::Info>,clock_info:Option<clock_source::Info>}
impl Services{
 fn new()->Self{Self{denials:source_denial::Outbox::new(),seed:entropy::Machine::new(),clock:clock_machine::Machine::new(),trust:trust_machine::Machine::new(),seed_info:Some(entropy_source::Info{consumer:entropy_source::Identity{pd:15,task:150},available:true}),clock_info:Some(clock_source::Info{consumer:clock_source::Identity{pd:15,task:150},source:clock_source::CMOS_UTC})}}
 fn dispatch(&mut self,pd:u32,task:u32,op:u64,args:[u64;3]){actual_dispatch(PdxMessage{caller_pd:pd,caller_task:task,type_id:op,arg0:args[0],arg1:args[1],arg2:args[2]},&mut self.denials,&mut self.seed,&mut self.clock,&mut self.trust,self.seed_info,self.clock_info)}
 fn no_source_receipts(&self){assert_eq!(self.seed.pending_reply(),None);assert_eq!(self.seed.owner(),None);assert_eq!(self.clock.pending_reply(),None);assert_eq!(self.clock.snapshot().high_water,None);assert_eq!(self.trust.pending_reply(),None)}
}
const SPINDLES:[u32;8]=[12,17,24,25,26,27,28,29];
fn install(){HOST.with(|h|{let mut state=Host::new();for(i,pd)in [15,12,17,24,25,26,27,28,29].into_iter().enumerate(){let row=app_catalog::by_target(pd).unwrap();state.held[i]=Some(Held{slot:row.lifecycle_slot,catalog:lifecycle_wire::CatalogInfo{target_pd:pd,task_id:pd*10,surface_id:row.surface,coordinator_pd:3},live:true,catalog_error:None,info_override:None});}*h.borrow_mut()=state;});}
fn source_calls()->(usize,usize,usize){HOST.with(|h|{let h=h.borrow();(h.words,h.times,h.trusts)})}
fn entropy_request(seq:u32,kind:u8,ticket:u32,offset:u16,length:u16)->[u64;3]{entropy_seed::Request{seq,kind,ticket,offset,length,a:0}.encode()}
fn requests(seq:u32)->[(u64,[u64;3]);3]{[(entropy_seed::OP,entropy_request(seq,entropy_seed::BEGIN,0,0,32)),(certificate_time::OP,certificate_time::Request{seq}.encode()),(trust_policy::OP,trust_policy::Request{seq}.encode())]}
fn browser_seed_reply(s:&mut Services,seq:u32)->Result<u32,u32>{let(owner,word)=s.seed.pending_reply().expect("Browser retained seed receipt");assert_eq!(owner,entropy_source::Identity{pd:15,task:150});s.seed.reply_accepted();entropy_seed::decode_reply(word,seq)}

#[test]fn dispatch_eight_spindles_all_protocols_cannot_occupy_source_fences_or_prevent_seed32_time_trust(){
 install();assert_eq!(app_catalog::SPINDLE_TARGETS,SPINDLES);let mut s=Services::new();
 for pd in SPINDLES{for(op,args)in requests(1){s.dispatch(pd,pd*10,op,args);}}
 s.no_source_receipts();assert_eq!(source_calls(),(0,0,0));
 let mut denied=Vec::new();while let Some((owner,word))=s.denials.pending_reply(){denied.push((owner,word));s.denials.reply_accepted();}
 assert_eq!(denied.len(),24);
 for pd in SPINDLES{for word in [entropy_seed::error(1,entropy_seed::DENIED),certificate_time::error(1,certificate_time::DENIED),trust_policy::error(1,trust_policy::DENIED)]{assert_eq!(denied.iter().filter(|(o,w)|o.pd==pd&&o.task==pd*10&&*w==word).count(),1);}}
 // All denial cache/floors remain retained while the authorized machine works.
 s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();assert_eq!(ticket,1);
 for _ in 0..4{maintain_seed_source(&mut s.seed);}
 assert_eq!(s.seed.snapshot().produced,32);let mut bytes=Vec::new();
 for n in 0..8{let seq=n+2;s.dispatch(15,150,entropy_seed::OP,entropy_request(seq,entropy_seed::READ,ticket,(n*4)as u16,4));bytes.extend_from_slice(&browser_seed_reply(&mut s,seq).unwrap().to_le_bytes());}
 let expected:Vec<u8>=(1u64..=4).flat_map(u64::to_le_bytes).collect();assert_eq!(bytes,expected);
 s.dispatch(15,150,entropy_seed::OP,entropy_request(10,entropy_seed::RELEASE,ticket,0,0));assert_eq!(browser_seed_reply(&mut s,10),Ok(0));assert_eq!(s.seed.owner(),None);
 s.dispatch(15,150,certificate_time::OP,[1,0,0]);let(o,w)=s.clock.pending_reply().unwrap();assert_eq!(o,clock_source::Identity{pd:15,task:150});assert_eq!(certificate_time::decode_reply(w,1),Ok((clock_source::MIN_SECONDS+100)as u32));
 s.dispatch(15,150,trust_policy::OP,[1,0,0]);let(o,w)=s.trust.pending_reply().unwrap();assert_eq!(o,trust_source::Identity{pd:15,task:150});assert_eq!(trust_policy::decode_reply(w,1),Ok(trust_config::PROFILE));assert_eq!(source_calls(),(4,1,1));
}
#[test]fn dispatch_family_budget_is_not_typed_source_authority(){
 install();let mut s=Services::new();assert!(source_receipt_member(jobs::Identity{pd:15,task:150}));
 s.seed_info.as_mut().unwrap().consumer.task=15;s.clock_info.as_mut().unwrap().consumer.task=15;
 HOST.with(|h|h.borrow_mut().trust_consumer.task=15);
 for(op,args)in requests(1){s.dispatch(15,150,op,args);}
 assert_eq!(browser_seed_reply(&mut s,1),Err(entropy_seed::DENIED));assert_eq!(s.seed.owner(),None);
 assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,1),Err(certificate_time::DENIED));
 assert_eq!(trust_policy::decode_reply(s.trust.pending_reply().unwrap().1,1),Err(trust_policy::DENIED));
 assert_eq!(source_calls(),(0,0,1));assert_eq!(s.denials.pending_reply(),None);
}
#[test]fn dispatch_wrong_stamped_task_dead_unknown_and_noncanonical_requests_never_consume_denial_budget(){
 install();let mut s=Services::new();
 for(pd,task)in [(12,12),(12,0),(12,170),(15,15),(30,300)]{for(op,args)in requests(1){s.dispatch(pd,task,op,args);}}
 HOST.with(|h|h.borrow_mut().held.iter_mut().flatten().find(|e|e.catalog.target_pd==17).unwrap().live=false);
 for(op,args)in requests(2){s.dispatch(17,170,op,args);}
 for(op,args)in requests(3){for at in 0..3{let mut bad=args;bad[at]|=1u64<<63;s.dispatch(12,120,op,bad);}}
 assert_eq!(s.denials.pending_reply(),None);s.no_source_receipts();assert_eq!(source_calls(),(0,0,0));
 HOST.with(|h|assert!(!h.borrow().logs.iter().any(|l|l.starts_with("[net.source.denied]"))));
}
#[test]fn dispatch_full_denials_keep_exact_words_rotate_and_never_retransmit_after_accept(){
 install();let mut s=Services::new();for pd in [12,17]{for(op,args)in requests(7){s.dispatch(pd,pd*10,op,args);}}
 HOST.with(|h|h.borrow_mut().reply_status=9);
 let mut expected=Vec::new();for _ in 0..6{let(owner,word)=s.denials.pending_reply().unwrap();expected.push((owner.pd,word));poll_source_denials(&mut s.denials);}
 HOST.with(|h|{let mut h=h.borrow_mut();assert_eq!(h.replies,expected);h.replies.clear();h.reply_status=0;});
 for _ in 0..6{poll_source_denials(&mut s.denials);}assert_eq!(s.denials.pending_reply(),None);
 HOST.with(|h|assert_eq!(h.borrow().replies,expected));poll_source_denials(&mut s.denials);HOST.with(|h|assert_eq!(h.borrow().replies,expected));s.no_source_receipts();
}
#[test]fn dispatch_uncertain_denial_liveness_retains_exact_word_and_floor_without_sending(){
 for change in 0..7{install();let mut s=Services::new();for(op,args)in requests(7){s.dispatch(12,120,op,args);}
 let original=HOST.with(|h|h.borrow().held[1].unwrap());
 HOST.with(|h|{let mut h=h.borrow_mut();let e=h.held[1].as_mut().unwrap();match change{
  0=>e.info_override=Some(Err(2)),1=>e.catalog_error=Some(2),2=>e.catalog.task_id=121,
  3=>e.catalog.coordinator_pd=4,
  4=>e.info_override=Some(Ok(lifecycle_wire::LifecycleInfo{target_pd:12,task_id:121,required_mask:31,acked_mask:31})),
  5=>e.info_override=Some(Ok(lifecycle_wire::LifecycleInfo{target_pd:12,task_id:120,required_mask:31&!app_lifecycle::ROLE_NET,acked_mask:31})),
  _=>h.held[1]=None,
 }});
 let mut retained=Vec::new();for _ in 0..3{retained.push(s.denials.pending_reply().unwrap());poll_source_denials(&mut s.denials);}
 for expected in &retained{assert_eq!(s.denials.pending_reply(),Some(*expected));poll_source_denials(&mut s.denials);}
 HOST.with(|h|{let mut h=h.borrow_mut();assert!(h.replies.is_empty());h.held[1]=Some(original);});
 for _ in 0..3{poll_source_denials(&mut s.denials);}assert_eq!(s.denials.pending_reply(),None);
 HOST.with(|h|assert_eq!(h.borrow().replies,retained.iter().map(|(o,w)|(o.pd,*w)).collect::<Vec<_>>()));
 // A lower sequence remains stale after transient loss and later admission.
 for(op,args)in requests(6){s.dispatch(12,120,op,args);}
 let expected=[entropy_seed::error(6,entropy_seed::STALE),certificate_time::error(6,certificate_time::STALE),trust_policy::error(6,trust_policy::STALE)];
 for word in expected{let(o,w)=s.denials.pending_reply().unwrap();assert_eq!((o.pd,o.task,w),(12,120,word));s.denials.reply_accepted();}
 s.no_source_receipts();assert_eq!(source_calls(),(0,0,0));
 }
}
#[test]fn dispatch_exact_terminal_denial_proof_retires_only_that_identity_all_kinds(){
 install();let mut s=Services::new();for pd in [12,17]{for(op,args)in requests(1){s.dispatch(pd,pd*10,op,args);}}
 HOST.with(|h|h.borrow_mut().held[1].as_mut().unwrap().live=false);
 poll_source_denials(&mut s.denials);HOST.with(|h|assert!(h.borrow().replies.is_empty()));
 let mut remaining=Vec::new();while let Some((o,w))=s.denials.pending_reply(){assert_eq!((o.pd,o.task),(17,170));remaining.push(w);poll_source_denials(&mut s.denials);}
 assert_eq!(remaining.len(),3);HOST.with(|h|assert_eq!(h.borrow().replies.len(),3));s.no_source_receipts();
}
#[test]fn dispatch_clock_high_water_and_poison_survive_exact_owner_retirement(){
 install();let mut s=Services::new();let a=clock_source::Identity{pd:15,task:150};
 s.dispatch(15,150,certificate_time::OP,[1,0,0]);s.clock.reply_accepted();assert!(s.clock.retire(a));
 // Host-only unequal task namespace change does not create a runtime task.
 HOST.with(|h|{let mut h=h.borrow_mut();h.held[0].as_mut().unwrap().catalog.task_id=151;h.seconds-=1;});s.clock_info.as_mut().unwrap().consumer.task=151;
 s.dispatch(15,151,certificate_time::OP,[1,0,0]);assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,1),Err(certificate_time::ROLLBACK));
 let high=s.clock.snapshot();assert_eq!(high.high_water,Some(clock_source::MIN_SECONDS+100));assert!(high.rollback);
 s.clock.retire(clock_source::Identity{pd:15,task:151});HOST.with(|h|h.borrow_mut().seconds+=20);
 s.dispatch(15,151,certificate_time::OP,[2,0,0]);assert_eq!(s.clock.snapshot(),high);assert_eq!(source_calls(),(0,2,0));assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,2),Err(certificate_time::ROLLBACK));
}

fn pending_sources()->Services{
 install();let mut s=Services::new();for(op,args)in requests(1){s.dispatch(15,150,op,args);}s
}
fn pending_words(s:&Services)->[u64;3]{[s.seed.pending_reply().unwrap().1,s.clock.pending_reply().unwrap().1,s.trust.pending_reply().unwrap().1]}
fn poll_sources(s:&mut Services){poll_source_receipts(&mut s.seed,&mut s.clock,&mut s.trust)}
#[test]fn source_poll_partial_seed_resumes_same_lease_and_cached_time_trust_after_retry(){
 let mut s=pending_sources();let ticket=browser_seed_reply(&mut s,1).unwrap();maintain_seed_source(&mut s.seed);assert_eq!(s.seed.snapshot().produced,8);
 let clock_word=s.clock.pending_reply();let trust_word=s.trust.pending_reply();let original=HOST.with(|h|h.borrow().held[0].unwrap());
 HOST.with(|h|h.borrow_mut().held[0].as_mut().unwrap().info_override=Some(Err(2)));
 for tick in 101..105{HOST.with(|h|h.borrow_mut().ticks=tick);maintain_seed_source(&mut s.seed);poll_sources(&mut s);assert_eq!(s.seed.snapshot().produced,8);assert_eq!(s.seed.snapshot().ticket,ticket);assert_eq!(s.clock.pending_reply(),clock_word);assert_eq!(s.trust.pending_reply(),trust_word);}
 assert_eq!(source_calls(),(1,1,1));HOST.with(|h|{let mut h=h.borrow_mut();assert!(h.replies.is_empty());h.held[0]=Some(original);h.seconds+=1000;});
 for _ in 0..3{maintain_seed_source(&mut s.seed);}assert_eq!(s.seed.snapshot().produced,32);assert_eq!(s.seed.snapshot().ticket,ticket);
 poll_sources(&mut s);HOST.with(|h|assert_eq!(h.borrow().replies,[(15,clock_word.unwrap().1),(15,trust_word.unwrap().1)]));
 assert_eq!(source_calls(),(4,1,1));poll_sources(&mut s);HOST.with(|h|assert_eq!(h.borrow().replies.len(),2));
 // The same query remains its historical receipt, never a fresh sample.
 s.dispatch(15,150,certificate_time::OP,[1,0,0]);s.dispatch(15,150,trust_policy::OP,[1,0,0]);assert_eq!(s.clock.pending_reply(),clock_word);assert_eq!(s.trust.pending_reply(),trust_word);assert_eq!(source_calls(),(4,1,1));
}
#[test]fn source_poll_full_retains_all_three_words_and_accepts_each_once(){
 let mut s=pending_sources();let words=pending_words(&s);HOST.with(|h|h.borrow_mut().reply_status=9);
 for _ in 0..3{poll_sources(&mut s);assert_eq!(pending_words(&s),words);}assert_eq!(source_calls(),(0,1,1));
 HOST.with(|h|{let mut h=h.borrow_mut();assert_eq!(h.replies.len(),9);for group in h.replies.chunks_exact(3){assert_eq!(group,[(15,words[0]),(15,words[1]),(15,words[2])]);}h.replies.clear();h.reply_status=0;});
 poll_sources(&mut s);assert_eq!(s.seed.pending_reply(),None);assert_eq!(s.clock.pending_reply(),None);assert_eq!(s.trust.pending_reply(),None);poll_sources(&mut s);HOST.with(|h|assert_eq!(h.borrow().replies,words.map(|word|(15,word))));
}
#[test]fn source_poll_uncertain_or_malformed_proof_retains_every_machine_until_exact_terminal(){
 for change in 0..8{let mut s=pending_sources();let words=pending_words(&s);let original=HOST.with(|h|h.borrow().held[0].unwrap());let high=s.clock.snapshot();
 HOST.with(|h|{let mut h=h.borrow_mut();let e=h.held[0].as_mut().unwrap();match change{
  0=>e.info_override=Some(Err(2)),1=>e.catalog_error=Some(2),2=>e.catalog.task_id=151,3=>e.catalog.coordinator_pd=4,
  4=>e.info_override=Some(Ok(lifecycle_wire::LifecycleInfo{target_pd:17,task_id:150,required_mask:31,acked_mask:31})),
  5=>e.info_override=Some(Ok(lifecycle_wire::LifecycleInfo{target_pd:15,task_id:151,required_mask:31,acked_mask:31})),
  6=>e.info_override=Some(Ok(lifecycle_wire::LifecycleInfo{target_pd:15,task_id:150,required_mask:31&!app_lifecycle::ROLE_NET,acked_mask:31})),
  _=>h.held[0]=None,
 }});
 for _ in 0..3{maintain_seed_source(&mut s.seed);poll_sources(&mut s);assert_eq!(pending_words(&s),words);assert_eq!(s.seed.snapshot().produced,0);assert_eq!(s.clock.snapshot(),high);}
 assert_eq!(source_calls(),(0,1,1));HOST.with(|h|{let mut h=h.borrow_mut();assert!(h.replies.is_empty());h.held[0]=Some(Held{live:false,..original});});
 maintain_seed_source(&mut s.seed);poll_sources(&mut s);assert_eq!(s.seed.owner(),None);assert_eq!(s.seed.pending_reply(),None);assert_eq!(s.clock.pending_reply(),None);assert_eq!(s.trust.pending_reply(),None);assert_eq!(s.clock.snapshot(),high);HOST.with(|h|assert!(h.borrow().replies.is_empty()));
 }}
#[test]fn source_poll_uncertainty_cannot_extend_seed_deadline_or_deliver_expired_read_payload(){
 for tick in [99,2100]{install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();
 for _ in 0..4{maintain_seed_source(&mut s.seed);}s.dispatch(15,150,entropy_seed::OP,entropy_request(2,entropy_seed::READ,ticket,0,4));let raw=s.seed.pending_reply().unwrap().1;assert_eq!(entropy_seed::decode_reply(raw,2),Ok(1));
 HOST.with(|h|{let mut h=h.borrow_mut();h.held[0].as_mut().unwrap().info_override=Some(Err(2));h.ticks=tick;});
 maintain_seed_source(&mut s.seed);poll_sources(&mut s);let snap=s.seed.snapshot();assert_eq!(snap.ticket,ticket);assert_eq!(snap.owner,Some(entropy_source::Identity{pd:15,task:150}));assert_eq!(snap.state,Some(entropy_seed::State::Failed));assert_eq!((snap.produced,snap.consumed,snap.failure),(0,0,entropy_seed::TIMEOUT));
 let redacted=s.seed.pending_reply().unwrap().1;assert_ne!(redacted,raw);assert_eq!(entropy_seed::decode_reply(redacted,2),Err(entropy_seed::INVALID));HOST.with(|h|{let mut h=h.borrow_mut();assert!(h.replies.is_empty());h.held[0].as_mut().unwrap().info_override=None;});
 maintain_seed_source(&mut s.seed);poll_sources(&mut s);assert_eq!(source_calls(),(4,0,0));HOST.with(|h|assert_eq!(h.borrow().replies,[(15,redacted)]));
 }}
#[test]fn source_poll_rotates_uncertain_clock_and_trust_without_evicting_other_custody(){
 let mut s=pending_sources();s.seed.reply_accepted();
 // Machine-only host source metadata adds a second exact identity. This is
 // not an actual Spindle source grant or fresh Main dispatch admission.
 let c=clock_source::Identity{pd:17,task:170};let t=trust_source::Identity{pd:17,task:170};
 assert!(s.clock.handle(c,certificate_time::Request{seq:1},Some(clock_source::Info{consumer:c,source:1}),||Ok(clock_source::Snapshot{seconds:clock_source::MIN_SECONDS+100,attempts:1})));
 assert!(s.trust.handle(t,trust_policy::Request{seq:1},||Ok(trust_source::Info{consumer:t,profile:trust_config::PROFILE,root_bytes:trust_config::ROOT_DER.len(),version:trust_config::VERSION})));
 HOST.with(|h|h.borrow_mut().held[0].as_mut().unwrap().info_override=Some(Err(2)));
 let first_clock=s.clock.pending_reply();let first_trust=s.trust.pending_reply();poll_sources(&mut s);HOST.with(|h|assert!(h.borrow().replies.is_empty()));poll_sources(&mut s);
 HOST.with(|h|{let h=h.borrow();assert_eq!(h.replies.len(),2);assert!(h.replies.iter().all(|(pd,_)|*pd==17));});assert_eq!(s.clock.pending_reply(),first_clock);assert_eq!(s.trust.pending_reply(),first_trust);
 HOST.with(|h|h.borrow_mut().held[0].as_mut().unwrap().info_override=None);poll_sources(&mut s);assert_eq!(s.clock.pending_reply(),None);assert_eq!(s.trust.pending_reply(),None);
}
#[test]fn source_poll_exact_terminal_retirement_preserves_global_clock_poison(){
 let mut s=pending_sources();s.clock.reply_accepted();HOST.with(|h|h.borrow_mut().seconds-=1);s.dispatch(15,150,certificate_time::OP,[2,0,0]);let poisoned=s.clock.snapshot();assert!(poisoned.rollback);
 HOST.with(|h|h.borrow_mut().held[0].as_mut().unwrap().live=false);maintain_seed_source(&mut s.seed);poll_sources(&mut s);assert_eq!(s.clock.pending_reply(),None);assert_eq!(s.clock.snapshot(),poisoned);
 HOST.with(|h|{let mut h=h.borrow_mut();h.held[0].as_mut().unwrap().live=true;h.seconds+=100;});s.dispatch(15,150,certificate_time::OP,[3,0,0]);assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,3),Err(certificate_time::ROLLBACK));assert_eq!(source_calls(),(0,2,1));assert_eq!(s.clock.snapshot(),poisoned);
}
#[test]fn source_poll_delivery_rechecks_lease_after_prior_maintenance_before_secret_copy(){
 install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();
 for _ in 0..4{maintain_seed_source(&mut s.seed);}s.dispatch(15,150,entropy_seed::OP,entropy_request(2,entropy_seed::READ,ticket,0,4));let secret=s.seed.pending_reply().unwrap().1;assert_eq!(entropy_seed::decode_reply(secret,2),Ok(1));
 // The earlier loop pass was still in lease. Other service work crosses the
 // exact deadline, so only the delivery helper can redact before taking a copy.
 HOST.with(|h|h.borrow_mut().ticks=2099);maintain_seed_source(&mut s.seed);assert_eq!(s.seed.pending_reply().unwrap().1,secret);
 HOST.with(|h|h.borrow_mut().ticks=2100);poll_sources(&mut s);
 assert_eq!(s.seed.snapshot().failure,entropy_seed::TIMEOUT);assert_eq!(s.seed.pending_reply(),None);assert_eq!(source_calls(),(4,0,0));
 HOST.with(|h|{let h=h.borrow();assert_eq!(h.replies.len(),1);assert_eq!(h.replies[0].0,15);assert_ne!(h.replies[0].1,secret);assert_eq!(entropy_seed::decode_reply(h.replies[0].1,2),Err(entropy_seed::INVALID));});
}
#[test]fn source_poll_positive_live_query_crossing_deadline_prevents_collecting_provider_call(){
 for after_query in [2100,99]{
 install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();maintain_seed_source(&mut s.seed);assert_eq!(s.seed.snapshot().produced,8);
 HOST.with(|h|{let mut h=h.borrow_mut();h.ticks=2099;h.life_tick_after=Some(after_query);h.life_calls=0;assert!(h.held[0].unwrap().live);assert!(h.held[0].unwrap().info_override.is_none());});
 maintain_seed_source(&mut s.seed);let snap=s.seed.snapshot();assert_eq!(snap.owner,Some(entropy_source::Identity{pd:15,task:150}));assert_eq!(snap.ticket,ticket);assert_eq!(snap.state,Some(entropy_seed::State::Failed));assert_eq!((snap.produced,snap.failure),(0,entropy_seed::TIMEOUT));assert_eq!(source_calls(),(1,0,0));HOST.with(|h|{let h=h.borrow();assert_eq!(h.life_calls,1);assert_eq!(h.ticks,after_query);assert!(h.replies.is_empty());});
 }
}
#[test]fn source_poll_positive_live_query_crossing_deadline_cannot_copy_or_deliver_old_read(){
 for after_query in [2100,99]{
 install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();for _ in 0..4{maintain_seed_source(&mut s.seed);}
 s.dispatch(15,150,entropy_seed::OP,entropy_request(2,entropy_seed::READ,ticket,0,4));let secret=s.seed.pending_reply().unwrap().1;assert_eq!(entropy_seed::decode_reply(secret,2),Ok(1));
 HOST.with(|h|{let mut h=h.borrow_mut();h.ticks=2099;h.life_tick_after=Some(after_query);h.life_calls=0;assert!(h.held[0].unwrap().live);assert!(h.held[0].unwrap().info_override.is_none());});
 poll_sources(&mut s);assert_eq!(s.seed.snapshot().failure,entropy_seed::TIMEOUT);assert_eq!(s.seed.pending_reply(),None);assert_eq!(source_calls(),(4,0,0));
 HOST.with(|h|{let h=h.borrow();assert_eq!(h.life_calls,1);assert_eq!(h.ticks,after_query);assert_eq!(h.replies.len(),1);assert_eq!(h.replies[0].0,15);assert_ne!(h.replies[0].1,secret);assert_eq!(entropy_seed::decode_reply(h.replies[0].1,2),Err(entropy_seed::INVALID));});
 }
}
