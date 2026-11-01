//! Actual extracted Net dispatch/helpers, actual SDK and source Machines.
//! Finite held-query/provider/IPC callbacks are host controls, not authority.
#![allow(dead_code)]
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[cfg(not(browser_source_catalog))]
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
// The gate emits only a path declaration here; the complete fresh catalog
// copy remains a module, preserving its actual SDK inner documentation.
#[cfg(browser_source_catalog)]include!(env!("BROWSER_SOURCE_CATALOG_MODULE"));
#[path="../crates/sex-pdx/src/browser_sources.rs"]pub mod browser_sources;
#[path="../servers/sexnet/src/source_binding.rs"]mod source_binding;
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
struct Host{held:[Option<Held>;app_catalog::COUNT],words:usize,times:usize,trusts:usize,seconds:u64,ticks:u64,life_tick_after:Option<u64>,life_calls:usize,source_info:Vec<(u64,SourceInfo)>,queries:Vec<(&'static str,u64)>,effects:Vec<(&'static str,u64)>,reply_status:u64,replies:Vec<(u32,u64)>,logs:Vec<String>}
impl Host{fn new()->Self{Self{held:[None;app_catalog::COUNT],words:0,times:0,trusts:0,seconds:clock_source::MIN_SECONDS+100,ticks:100,life_tick_after:None,life_calls:0,source_info:Vec::new(),queries:Vec::new(),effects:Vec::new(),reply_status:0,replies:Vec::new(),logs:Vec::new()}}}
thread_local!{static HOST:RefCell<Host>=RefCell::new(Host::new());}
pub mod app_lifecycle{
 pub use crate::lifecycle_wire::{SLOTS,LIVE,ROLE_NET,CatalogInfo,LifecycleInfo};
 pub fn catalog(slot:usize)->Result<CatalogInfo,u64>{crate::HOST.with(|h|h.borrow().held.iter().flatten().find(|e|e.slot==slot).map_or(Err(1),|e|e.catalog_error.map_or(Ok(e.catalog),Err)))}
 pub fn info(slot:usize)->Result<LifecycleInfo,u64>{crate::HOST.with(|h|{let mut h=h.borrow_mut();h.life_calls+=1;if let Some(ticks)=h.life_tick_after{h.ticks=ticks;}h.held.iter().flatten().find(|e|e.slot==slot).map_or(Err(1),|e|e.info_override.unwrap_or_else(||if e.live{Err(LIVE)}else{Ok(LifecycleInfo{target_pd:e.catalog.target_pd,task_id:e.catalog.task_id,required_mask:31,acked_mask:31})}))})}
}
pub fn get_ticks()->u64{HOST.with(|h|h.borrow().ticks)}
fn pdx_reply(pd:u32,word:u64)->u64{HOST.with(|h|{let mut h=h.borrow_mut();h.replies.push((pd,word));h.reply_status})}
#[derive(Clone,Copy)]enum SourceInfo{Entropy(Result<entropy_source::Info,u64>),Clock(Result<clock_source::Info,u64>),Trust(Result<trust_source::Info,u64>)}
pub fn entropy_info(slot:u64)->Result<entropy_source::Info,u64>{HOST.with(|h|{let mut h=h.borrow_mut();h.queries.push(("entropy",slot));match h.source_info.iter().find(|e|e.0==slot){Some((_,SourceInfo::Entropy(info)))=>*info,_=>Err(entropy_source::DENIED)}})}
pub fn clock_info(slot:u64)->Result<clock_source::Info,u64>{HOST.with(|h|{let mut h=h.borrow_mut();h.queries.push(("clock",slot));match h.source_info.iter().find(|e|e.0==slot){Some((_,SourceInfo::Clock(info)))=>*info,_=>Err(clock_source::DENIED)}})}
pub fn trust_info(slot:u64)->Result<trust_source::Info,u64>{HOST.with(|h|{let mut h=h.borrow_mut();h.queries.push(("trust",slot));h.effects.push(("trust",slot));h.trusts+=1;match h.source_info.iter().find(|e|e.0==slot){Some((_,SourceInfo::Trust(info)))=>*info,_=>Err(trust_source::DENIED)}})}
pub fn clock_snapshot(slot:u64)->Result<clock_source::Snapshot,u64>{HOST.with(|h|{let mut h=h.borrow_mut();h.effects.push(("clock",slot));h.times+=1;match h.source_info.iter().find(|e|e.0==slot){Some((_,SourceInfo::Clock(Ok(_))))=>Ok(clock_source::Snapshot{seconds:h.seconds,attempts:1}),_=>Err(clock_source::DENIED)}})}
pub fn entropy_word(slot:u64)->Result<entropy_source::Word,u64>{HOST.with(|h|{let mut h=h.borrow_mut();h.effects.push(("entropy",slot));h.words+=1;match h.source_info.iter().find(|e|e.0==slot){Some((_,SourceInfo::Entropy(Ok(info))))if info.available=>Ok(entropy_source::Word{value:if slot==60{h.words as u64}else{(slot<<32)|h.words as u64},attempts:1}),_=>Err(entropy_source::DENIED)}})}
fn record(text:String){HOST.with(|h|h.borrow_mut().logs.push(text));}
#[macro_export]macro_rules! serial_record{($($t:tt)*)=>{$crate::record(format!($($t)*))}}
include!(env!("SOURCE_RECEIPT_PRODUCTION"));

struct Services{denials:source_denial::Outbox,seed:entropy::Machine,clock:clock_machine::Machine,trust:trust_machine::Machine,bindings:[Option<source_binding::Binding>;browser_sources::BROWSER_COUNT]}
impl Services{
 fn new()->Self{let bindings=load_source_bindings();
 // Separate startup INFO observations (retained in queries) from later work.
 HOST.with(|h|{let mut h=h.borrow_mut();h.trusts=0;h.effects.clear();});
 Self{denials:source_denial::Outbox::new(),seed:entropy::Machine::new(),clock:clock_machine::Machine::new(),trust:trust_machine::Machine::new(),bindings}}

 fn dispatch(&mut self,pd:u32,task:u32,op:u64,args:[u64;3]){actual_dispatch(PdxMessage{caller_pd:pd,caller_task:task,type_id:op,arg0:args[0],arg1:args[1],arg2:args[2]},&mut self.denials,&mut self.seed,&mut self.clock,&mut self.trust,&self.bindings)}
 fn no_source_receipts(&self){assert_eq!(self.seed.pending_reply(),None);assert_eq!(self.seed.owner(),None);assert_eq!(self.clock.pending_reply(),None);assert_eq!(self.clock.snapshot().high_water,None);assert_eq!(self.trust.pending_reply(),None)}
}
const SPINDLES:[u32;8]=[12,17,24,25,26,27,28,29];
fn install(){HOST.with(|h|{let mut state=Host::new();for(i,pd)in [15,12,17,24,25,26,27,28,29].into_iter().enumerate(){let row=app_catalog::by_target(pd).unwrap();state.held[i]=Some(Held{slot:row.lifecycle_slot,catalog:lifecycle_wire::CatalogInfo{target_pd:pd,task_id:pd*10,surface_id:row.surface,coordinator_pd:3},live:true,catalog_error:None,info_override:None});}
 for plan in browser_sources::PLANS{
  let pd=plan.target();let task=pd*10;
  if !state.held.iter().flatten().any(|e|e.catalog.target_pd==pd){let empty=state.held.iter_mut().find(|e|e.is_none()).unwrap();*empty=Some(Held{slot:plan.lifecycle_slot(),catalog:lifecycle_wire::CatalogInfo{target_pd:pd,task_id:task,surface_id:plan.surface(),coordinator_pd:3},live:true,catalog_error:None,info_override:None});}
  state.source_info.push((plan.entropy_slot(),SourceInfo::Entropy(Ok(entropy_source::Info{consumer:entropy_source::Identity{pd,task},available:true}))));
  state.source_info.push((plan.clock_slot(),SourceInfo::Clock(Ok(clock_source::Info{consumer:clock_source::Identity{pd,task},source:clock_source::CMOS_UTC}))));
  state.source_info.push((plan.trust_slot(),SourceInfo::Trust(Ok(trust_source::Info{consumer:trust_source::Identity{pd,task},profile:trust_config::PROFILE,root_bytes:trust_config::ROOT_DER.len(),version:trust_config::VERSION}))));
 }*h.borrow_mut()=state;});}

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
 for _ in 0..4{maintain_seed_source(&mut s.seed,&s.bindings);}
 assert_eq!(s.seed.snapshot().produced,32);let mut bytes=Vec::new();
 for n in 0..8{let seq=n+2;s.dispatch(15,150,entropy_seed::OP,entropy_request(seq,entropy_seed::READ,ticket,(n*4)as u16,4));bytes.extend_from_slice(&browser_seed_reply(&mut s,seq).unwrap().to_le_bytes());}
 let expected:Vec<u8>=(1u64..=4).flat_map(u64::to_le_bytes).collect();assert_eq!(bytes,expected);
 s.dispatch(15,150,entropy_seed::OP,entropy_request(10,entropy_seed::RELEASE,ticket,0,0));assert_eq!(browser_seed_reply(&mut s,10),Ok(0));assert_eq!(s.seed.owner(),None);
 s.dispatch(15,150,certificate_time::OP,[1,0,0]);let(o,w)=s.clock.pending_reply().unwrap();assert_eq!(o,clock_source::Identity{pd:15,task:150});assert_eq!(certificate_time::decode_reply(w,1),Ok((clock_source::MIN_SECONDS+100)as u32));
 s.dispatch(15,150,trust_policy::OP,[1,0,0]);let(o,w)=s.trust.pending_reply().unwrap();assert_eq!(o,trust_source::Identity{pd:15,task:150});assert_eq!(trust_policy::decode_reply(w,1),Ok(trust_config::PROFILE));assert_eq!(source_calls(),(4,1,1));
}
#[test]fn dispatch_family_budget_is_not_typed_source_authority(){
 install();assert!(source_receipt_member(jobs::Identity{pd:15,task:150}));
 HOST.with(|h|for (_,info)in &mut h.borrow_mut().source_info{match info{SourceInfo::Entropy(Ok(i))if i.consumer.pd==15=>i.consumer.task=15,SourceInfo::Clock(Ok(i))if i.consumer.pd==15=>i.consumer.task=15,SourceInfo::Trust(Ok(i))if i.consumer.pd==15=>i.consumer.task=15,_=>{}}});
 let mut s=Services::new();assert!(s.bindings[0].is_none());
 for(op,args)in requests(1){s.dispatch(15,150,op,args);}
 assert_eq!(browser_seed_reply(&mut s,1),Err(entropy_seed::DENIED));assert_eq!(s.seed.owner(),None);
 assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,1),Err(certificate_time::DENIED));
 assert_eq!(trust_policy::decode_reply(s.trust.pending_reply().unwrap().1,1),Err(trust_policy::DENIED));
 assert_eq!(source_calls(),(0,0,0));assert_eq!(s.denials.pending_reply(),None);
}
#[test]fn dispatch_wrong_stamped_task_dead_unknown_and_noncanonical_requests_never_consume_denial_budget(){
 install();let mut s=Services::new();
 for(pd,task)in [(12,12),(12,0),(12,170),(15,15),(32,320)]{for(op,args)in requests(1){s.dispatch(pd,task,op,args);}}
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
 // Reusing a host machine after explicit owner retirement must retain the
 // boot-global high-water even without changing the immutable association.
 HOST.with(|h|h.borrow_mut().seconds-=1);
 s.dispatch(15,150,certificate_time::OP,[2,0,0]);assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,2),Err(certificate_time::ROLLBACK));
 let high=s.clock.snapshot();assert_eq!(high.high_water,Some(clock_source::MIN_SECONDS+100));assert!(high.rollback);
 s.clock.retire(a);HOST.with(|h|h.borrow_mut().seconds+=20);
 s.dispatch(15,150,certificate_time::OP,[3,0,0]);assert_eq!(s.clock.snapshot(),high);assert_eq!(source_calls(),(0,2,0));assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,3),Err(certificate_time::ROLLBACK));

}

fn pending_sources()->Services{
 install();let mut s=Services::new();for(op,args)in requests(1){s.dispatch(15,150,op,args);}s
}
fn pending_words(s:&Services)->[u64;3]{[s.seed.pending_reply().unwrap().1,s.clock.pending_reply().unwrap().1,s.trust.pending_reply().unwrap().1]}
fn poll_sources(s:&mut Services){poll_source_receipts(&mut s.seed,&mut s.clock,&mut s.trust)}
#[test]fn source_poll_partial_seed_resumes_same_lease_and_cached_time_trust_after_retry(){
 let mut s=pending_sources();let ticket=browser_seed_reply(&mut s,1).unwrap();maintain_seed_source(&mut s.seed,&s.bindings);assert_eq!(s.seed.snapshot().produced,8);
 let clock_word=s.clock.pending_reply();let trust_word=s.trust.pending_reply();let original=HOST.with(|h|h.borrow().held[0].unwrap());
 HOST.with(|h|h.borrow_mut().held[0].as_mut().unwrap().info_override=Some(Err(2)));
 for tick in 101..105{HOST.with(|h|h.borrow_mut().ticks=tick);maintain_seed_source(&mut s.seed,&s.bindings);poll_sources(&mut s);assert_eq!(s.seed.snapshot().produced,8);assert_eq!(s.seed.snapshot().ticket,ticket);assert_eq!(s.clock.pending_reply(),clock_word);assert_eq!(s.trust.pending_reply(),trust_word);}
 assert_eq!(source_calls(),(1,1,1));HOST.with(|h|{let mut h=h.borrow_mut();assert!(h.replies.is_empty());h.held[0]=Some(original);h.seconds+=1000;});
 for _ in 0..3{maintain_seed_source(&mut s.seed,&s.bindings);}assert_eq!(s.seed.snapshot().produced,32);assert_eq!(s.seed.snapshot().ticket,ticket);
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
 for _ in 0..3{maintain_seed_source(&mut s.seed,&s.bindings);poll_sources(&mut s);assert_eq!(pending_words(&s),words);assert_eq!(s.seed.snapshot().produced,0);assert_eq!(s.clock.snapshot(),high);}
 assert_eq!(source_calls(),(0,1,1));HOST.with(|h|{let mut h=h.borrow_mut();assert!(h.replies.is_empty());h.held[0]=Some(Held{live:false,..original});});
 maintain_seed_source(&mut s.seed,&s.bindings);poll_sources(&mut s);assert_eq!(s.seed.owner(),None);assert_eq!(s.seed.pending_reply(),None);assert_eq!(s.clock.pending_reply(),None);assert_eq!(s.trust.pending_reply(),None);assert_eq!(s.clock.snapshot(),high);HOST.with(|h|assert!(h.borrow().replies.is_empty()));
 }}
#[test]fn source_poll_uncertainty_cannot_extend_seed_deadline_or_deliver_expired_read_payload(){
 for tick in [99,2100]{install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();
 for _ in 0..4{maintain_seed_source(&mut s.seed,&s.bindings);}s.dispatch(15,150,entropy_seed::OP,entropy_request(2,entropy_seed::READ,ticket,0,4));let raw=s.seed.pending_reply().unwrap().1;assert_eq!(entropy_seed::decode_reply(raw,2),Ok(1));
 HOST.with(|h|{let mut h=h.borrow_mut();h.held[0].as_mut().unwrap().info_override=Some(Err(2));h.ticks=tick;});
 maintain_seed_source(&mut s.seed,&s.bindings);poll_sources(&mut s);let snap=s.seed.snapshot();assert_eq!(snap.ticket,ticket);assert_eq!(snap.owner,Some(entropy_source::Identity{pd:15,task:150}));assert_eq!(snap.state,Some(entropy_seed::State::Failed));assert_eq!((snap.produced,snap.consumed,snap.failure),(0,0,entropy_seed::TIMEOUT));
 let redacted=s.seed.pending_reply().unwrap().1;assert_ne!(redacted,raw);assert_eq!(entropy_seed::decode_reply(redacted,2),Err(entropy_seed::INVALID));HOST.with(|h|{let mut h=h.borrow_mut();assert!(h.replies.is_empty());h.held[0].as_mut().unwrap().info_override=None;});
 maintain_seed_source(&mut s.seed,&s.bindings);poll_sources(&mut s);assert_eq!(source_calls(),(4,0,0));HOST.with(|h|assert_eq!(h.borrow().replies,[(15,redacted)]));
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
 HOST.with(|h|h.borrow_mut().held[0].as_mut().unwrap().live=false);maintain_seed_source(&mut s.seed,&s.bindings);poll_sources(&mut s);assert_eq!(s.clock.pending_reply(),None);assert_eq!(s.clock.snapshot(),poisoned);
 HOST.with(|h|{let mut h=h.borrow_mut();h.held[0].as_mut().unwrap().live=true;h.seconds+=100;});s.dispatch(15,150,certificate_time::OP,[3,0,0]);assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,3),Err(certificate_time::ROLLBACK));assert_eq!(source_calls(),(0,2,1));assert_eq!(s.clock.snapshot(),poisoned);
}
#[test]fn source_poll_delivery_rechecks_lease_after_prior_maintenance_before_secret_copy(){
 install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();
 for _ in 0..4{maintain_seed_source(&mut s.seed,&s.bindings);}s.dispatch(15,150,entropy_seed::OP,entropy_request(2,entropy_seed::READ,ticket,0,4));let secret=s.seed.pending_reply().unwrap().1;assert_eq!(entropy_seed::decode_reply(secret,2),Ok(1));
 // The earlier loop pass was still in lease. Other service work crosses the
 // exact deadline, so only the delivery helper can redact before taking a copy.
 HOST.with(|h|h.borrow_mut().ticks=2099);maintain_seed_source(&mut s.seed,&s.bindings);assert_eq!(s.seed.pending_reply().unwrap().1,secret);
 HOST.with(|h|h.borrow_mut().ticks=2100);poll_sources(&mut s);
 assert_eq!(s.seed.snapshot().failure,entropy_seed::TIMEOUT);assert_eq!(s.seed.pending_reply(),None);assert_eq!(source_calls(),(4,0,0));
 HOST.with(|h|{let h=h.borrow();assert_eq!(h.replies.len(),1);assert_eq!(h.replies[0].0,15);assert_ne!(h.replies[0].1,secret);assert_eq!(entropy_seed::decode_reply(h.replies[0].1,2),Err(entropy_seed::INVALID));});
}
#[test]fn source_poll_positive_live_query_crossing_deadline_prevents_collecting_provider_call(){
 for after_query in [2100,99]{
 install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();maintain_seed_source(&mut s.seed,&s.bindings);assert_eq!(s.seed.snapshot().produced,8);
 HOST.with(|h|{let mut h=h.borrow_mut();h.ticks=2099;h.life_tick_after=Some(after_query);h.life_calls=0;assert!(h.held[0].unwrap().live);assert!(h.held[0].unwrap().info_override.is_none());});
 maintain_seed_source(&mut s.seed,&s.bindings);let snap=s.seed.snapshot();assert_eq!(snap.owner,Some(entropy_source::Identity{pd:15,task:150}));assert_eq!(snap.ticket,ticket);assert_eq!(snap.state,Some(entropy_seed::State::Failed));assert_eq!((snap.produced,snap.failure),(0,entropy_seed::TIMEOUT));assert_eq!(source_calls(),(1,0,0));HOST.with(|h|{let h=h.borrow();assert_eq!(h.life_calls,1);assert_eq!(h.ticks,after_query);assert!(h.replies.is_empty());});
 }
}
#[test]fn source_poll_positive_live_query_crossing_deadline_cannot_copy_or_deliver_old_read(){
 for after_query in [2100,99]{
 install();let mut s=Services::new();s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let ticket=browser_seed_reply(&mut s,1).unwrap();for _ in 0..4{maintain_seed_source(&mut s.seed,&s.bindings);}
 s.dispatch(15,150,entropy_seed::OP,entropy_request(2,entropy_seed::READ,ticket,0,4));let secret=s.seed.pending_reply().unwrap().1;assert_eq!(entropy_seed::decode_reply(secret,2),Ok(1));
 HOST.with(|h|{let mut h=h.borrow_mut();h.ticks=2099;h.life_tick_after=Some(after_query);h.life_calls=0;assert!(h.held[0].unwrap().live);assert!(h.held[0].unwrap().info_override.is_none());});
 poll_sources(&mut s);assert_eq!(s.seed.snapshot().failure,entropy_seed::TIMEOUT);assert_eq!(s.seed.pending_reply(),None);assert_eq!(source_calls(),(4,0,0));
 HOST.with(|h|{let h=h.borrow();assert_eq!(h.life_calls,1);assert_eq!(h.ticks,after_query);assert_eq!(h.replies.len(),1);assert_eq!(h.replies[0].0,15);assert_ne!(h.replies[0].1,secret);assert_eq!(entropy_seed::decode_reply(h.replies[0].1,2),Err(entropy_seed::INVALID));});
 }
}

#[test]fn source_binding_startup_queries_exact_held_plan_and_task(){
 install();let s=Services::new();assert_eq!(s.bindings.len(),browser_sources::BROWSER_COUNT);
 let expected=if browser_sources::BROWSER_COUNT==1{vec![(15,150,300,40,[60,61,62])]}else{vec![(15,150,300,40,[60,61,62]),(30,300,343,154,[70,71,72]),(31,310,344,158,[73,74,75])]};
 for(i,(pd,task,sid,slot,triple))in expected.iter().copied().enumerate(){let b=s.bindings[i].unwrap();assert_eq!((b.target(),b.task(),b.plan().surface(),b.plan().lifecycle_slot()),(pd,task,sid,slot));assert_eq!([b.entropy_slot(),b.clock_slot(),b.trust_slot()],triple);}
 let expected_queries:Vec<_>=expected.iter().flat_map(|e|[("entropy",e.4[0]),("clock",e.4[1]),("trust",e.4[2])]).collect();
 HOST.with(|h|{let h=h.borrow();assert_eq!(h.queries,expected_queries);assert_eq!(h.logs.iter().filter(|s|s.starts_with("[net.source.binding.held]")).count(),expected.len());assert!(h.effects.is_empty());});
 assert_eq!(source_calls(),(0,0,0));
}
#[test]fn source_binding_failed_startup_metadata_never_falls_back_or_calls_provider(){
 for change in 0..10{install();HOST.with(|h|{let mut h=h.borrow_mut();match change{
  0=>{h.source_info.retain(|e|e.0!=60);},
  1=>{let wrong=h.source_info.iter().find(|e|e.0==61).unwrap().1;h.source_info.iter_mut().find(|e|e.0==60).unwrap().1=wrong;},
  2=>h.source_info.iter_mut().find(|e|e.0==60).unwrap().1=SourceInfo::Entropy(Err(entropy_source::DENIED)),
  3=>if let SourceInfo::Entropy(Ok(i))=&mut h.source_info.iter_mut().find(|e|e.0==60).unwrap().1{i.consumer.task=15;},
  4=>if let SourceInfo::Clock(Ok(i))=&mut h.source_info.iter_mut().find(|e|e.0==61).unwrap().1{i.consumer.pd=30;i.consumer.task=300;},
  5=>if let SourceInfo::Trust(Ok(i))=&mut h.source_info.iter_mut().find(|e|e.0==62).unwrap().1{i.profile+=1;},
  6=>h.held[0].as_mut().unwrap().catalog_error=Some(2),
  7=>h.held[0].as_mut().unwrap().catalog.coordinator_pd=4,
  8=>h.held[0].as_mut().unwrap().catalog.surface_id=343,
  _=>h.held[0].as_mut().unwrap().catalog.task_id=0,
 }});
 let mut s=Services::new();assert!(s.bindings[0].is_none());
 // Restore lifecycle only: an initially missing binding never binds itself
 // from a later request, even when that incoming identity is now authentic.
 HOST.with(|h|{let mut h=h.borrow_mut();let c=h.held[0].as_mut().unwrap();c.catalog_error=None;c.catalog.coordinator_pd=3;c.catalog.surface_id=300;c.catalog.task_id=150;});
 for(op,args)in requests(1){s.dispatch(15,150,op,args);}assert_eq!(browser_seed_reply(&mut s,1),Err(entropy_seed::DENIED));maintain_seed_source(&mut s.seed,&s.bindings);
 assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,1),Err(certificate_time::DENIED));assert_eq!(trust_policy::decode_reply(s.trust.pending_reply().unwrap().1,1),Err(trust_policy::DENIED));assert_eq!(source_calls(),(0,0,0));HOST.with(|h|assert!(h.borrow().effects.is_empty()));
 }
}
#[test]fn source_binding_unavailable_entropy_still_binds_without_alternate_provider(){
 install();HOST.with(|h|if let SourceInfo::Entropy(Ok(i))=&mut h.borrow_mut().source_info.iter_mut().find(|e|e.0==60).unwrap().1{i.available=false;});
 let mut s=Services::new();assert!(s.bindings[0].is_some());assert!(!s.bindings[0].unwrap().entropy_info().available);s.dispatch(15,150,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));assert_eq!(browser_seed_reply(&mut s,1),Err(entropy_seed::OFFLINE));maintain_seed_source(&mut s.seed,&s.bindings);assert_eq!(source_calls(),(0,0,0));
 s.dispatch(15,150,certificate_time::OP,[1,0,0]);s.dispatch(15,150,trust_policy::OP,[1,0,0]);assert!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,1).is_ok());assert!(trust_policy::decode_reply(s.trust.pending_reply().unwrap().1,1).is_ok());
}
#[cfg(browser_source_catalog)]
#[test]fn source_binding_future_captured_owner_controls_entropy_slot_despite_other_requests(){
 install();let mut s=Services::new();assert_eq!(browser_sources::BROWSER_COUNT,3);
 s.dispatch(30,300,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let(owner,word)=s.seed.pending_reply().unwrap();assert_eq!(owner,entropy_source::Identity{pd:30,task:300});let ticket=entropy_seed::decode_reply(word,1).unwrap();s.seed.reply_accepted();
 maintain_seed_source(&mut s.seed,&s.bindings);assert_eq!(s.seed.snapshot().produced,8);
 // A different authenticated incoming request cannot change the active lease
 // owner or the slot used by the next asynchronous collection turn.
 s.dispatch(31,310,certificate_time::OP,[1,0,0]);s.dispatch(31,310,trust_policy::OP,[1,0,0]);
 s.dispatch(31,310,entropy_seed::OP,entropy_request(1,entropy_seed::BEGIN,0,0,32));let(other,word)=s.seed.pending_reply().unwrap();assert_eq!(other,entropy_source::Identity{pd:31,task:310});assert_eq!(entropy_seed::decode_reply(word,1),Err(entropy_seed::BUSY));s.seed.reply_accepted();
 for _ in 0..3{maintain_seed_source(&mut s.seed,&s.bindings);}assert_eq!(s.seed.owner(),Some(owner));assert_eq!(s.seed.snapshot().produced,32);
 s.dispatch(30,300,entropy_seed::OP,entropy_request(2,entropy_seed::READ,ticket,0,4));let(_,word)=s.seed.pending_reply().unwrap();assert_eq!(entropy_seed::decode_reply(word,2),Ok(1));s.seed.reply_accepted();
 s.dispatch(30,300,entropy_seed::OP,entropy_request(3,entropy_seed::READ,ticket,4,4));let(_,word)=s.seed.pending_reply().unwrap();assert_eq!(entropy_seed::decode_reply(word,3),Ok(70));s.seed.reply_accepted();
 HOST.with(|h|assert_eq!(h.borrow().effects,[("entropy",70),("clock",74),("trust",75),("entropy",70),("entropy",70),("entropy",70)]));
 s.dispatch(30,300,entropy_seed::OP,entropy_request(4,entropy_seed::RELEASE,ticket,0,0));let(_,word)=s.seed.pending_reply().unwrap();assert_eq!(entropy_seed::decode_reply(word,4),Ok(0));s.seed.reply_accepted();
 // Each future consumer's trust closure also selects its own immutable slot;
 // pending owner31 must not redirect a later owner30 INFO request.
 s.dispatch(30,300,trust_policy::OP,[1,0,0]);
 let(o,w)=s.trust.pending_reply().unwrap();assert_eq!((o.pd,o.task),(31,310));assert_eq!(trust_policy::decode_reply(w,1),Ok(trust_config::PROFILE));s.trust.reply_accepted();
 let(o,w)=s.trust.pending_reply().unwrap();assert_eq!((o.pd,o.task),(30,300));assert_eq!(trust_policy::decode_reply(w,1),Ok(trust_config::PROFILE));s.trust.reply_accepted();HOST.with(|h|assert_eq!(h.borrow().effects.last(),Some(&("trust",72))));
 s.dispatch(31,310,entropy_seed::OP,entropy_request(2,entropy_seed::BEGIN,0,0,32));s.seed.reply_accepted();maintain_seed_source(&mut s.seed,&s.bindings);assert_eq!(s.seed.owner(),Some(entropy_source::Identity{pd:31,task:310}));HOST.with(|h|assert_eq!(h.borrow().effects.last(),Some(&("entropy",73))));
}
#[cfg(browser_source_catalog)]
#[test]fn source_binding_future_clock_highwater_is_global_equal_forward_rollback_and_retirement(){
 install();let mut s=Services::new();let baseline=clock_source::MIN_SECONDS+100;
 for (pd,task,seconds)in [(15,150,baseline),(30,300,baseline),(31,310,baseline+20)]{
  HOST.with(|h|h.borrow_mut().seconds=seconds);s.dispatch(pd,task,certificate_time::OP,[1,0,0]);let(o,w)=s.clock.pending_reply().unwrap();assert_eq!(o,clock_source::Identity{pd,task});assert_eq!(certificate_time::decode_reply(w,1),Ok(seconds as u32));s.clock.reply_accepted();
 }
 assert_eq!(s.clock.snapshot().high_water,Some(baseline+20));HOST.with(|h|h.borrow_mut().seconds=baseline+19);
 s.dispatch(30,300,certificate_time::OP,[2,0,0]);assert_eq!(certificate_time::decode_reply(s.clock.pending_reply().unwrap().1,2),Err(certificate_time::ROLLBACK));s.clock.reply_accepted();
 let poisoned=s.clock.snapshot();assert!(poisoned.rollback);assert!(s.clock.retire(clock_source::Identity{pd:30,task:300}));assert_eq!(s.clock.snapshot(),poisoned);
 HOST.with(|h|h.borrow_mut().seconds=baseline+1000);
 for (pd,task,seq)in [(15,150,1),(31,310,2),(30,300,3)]{s.dispatch(pd,task,certificate_time::OP,[seq,0,0]);let(o,w)=s.clock.pending_reply().unwrap();assert_eq!(o,clock_source::Identity{pd,task});assert_eq!(certificate_time::decode_reply(w,seq as u32),Err(certificate_time::ROLLBACK));s.clock.reply_accepted();}
 assert_eq!(s.clock.snapshot(),poisoned);assert_eq!(source_calls(),(0,4,0));HOST.with(|h|assert_eq!(h.borrow().effects,[("clock",61),("clock",71),("clock",74),("clock",71)]));
}
