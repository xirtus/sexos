//! Actual Browser command coordinator + TLS preparation/provider lifecycles.
//! Public host fixtures supply policy, seed words and UTC. No network, native
//! memory reclamation, peer authentication or source authority is simulated.
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[path="../../sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../../sex-pdx/src/trust_source.rs"]pub mod trust_source;
#[path="../../sex-pdx/src/trust_policy.rs"]pub mod trust_policy;
#[path="../../sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../../sex-pdx/src/entropy_seed.rs"]pub mod entropy_seed;
#[path="../../sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../../sex-pdx/src/certificate_time.rs"]pub mod certificate_time;
#[path="../../../servers/sexnet/src/trust_policy.rs"]mod policies;
#[path="../../../servers/sexnet/src/entropy.rs"]mod seeds;
#[path="../../../servers/sexnet/src/certificate_time.rs"]mod clocks;
#[path="../../../apps/kaleidoscope/src/trust_probe.rs"]mod trust_probe;
#[path="../../../apps/kaleidoscope/src/entropy_probe.rs"]mod entropy_probe;
#[path="../../../apps/kaleidoscope/src/clock_probe.rs"]mod clock_probe;
#[path="../../../apps/kaleidoscope/src/tls_probe.rs"]mod tls_probe;
#[path="../../../apps/kaleidoscope/src/browser_command.rs"]mod browser_command;
use browser_command::{Busy,Intent};
use std::sync::Mutex;
pub const SLOT_NET:u64=18;pub const ERR_SERVICE_NOT_READY:u64=9;
const UTC:u64=1_793_367_907;
const OWNER:entropy_source::Identity=entropy_source::Identity{pd:15,task:15};
const TRUST_OWNER:trust_source::Identity=trust_source::Identity{pd:15,task:15};
const CLOCK_OWNER:clock_source::Identity=clock_source::Identity{pd:15,task:15};
static SERIAL:Mutex<()>=Mutex::new(());
static CALLS:Mutex<Vec<(u64,[u64;3])>>=Mutex::new(Vec::new());
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
    assert_eq!(slot,SLOT_NET);assert!(matches!(op,trust_policy::OP|entropy_seed::OP|certificate_time::OP));
    CALLS.lock().unwrap().push((op,[a,b,c]));(0,0)
}
struct World{
    p:tls_probe::Probe,v:trust_probe::Client,s:entropy_probe::Client,t:clock_probe::Client,
    policy:policies::Machine,seed:seeds::Machine,clock:clocks::Machine,
    pending:Option<Intent>,hold:Option<(u64,u8)>,held:Option<(u64,u64)>,
    calls:Vec<(u64,u32,u8)>,words:u64,now:u64,
}
impl World{
    fn new()->Self{CALLS.lock().unwrap().clear();Self{p:tls_probe::Probe::new(),v:trust_probe::Client::new(),s:entropy_probe::Client::new(),t:clock_probe::Client::new(),policy:policies::Machine::new(),seed:seeds::Machine::new(),clock:clocks::Machine::new(),pending:None,hold:None,held:None,calls:vec![],words:0,now:0}}
    fn providers(&self)->bool{self.v.busy()||self.s.busy()||self.t.busy()}
    fn ready(&mut self,route:bool)->Option<Intent>{let busy=Busy{route,providers:self.providers(),tls:self.p.busy()};browser_command::take_ready(&mut self.pending,busy)}
    fn replace(&mut self,next:Intent)->usize{browser_command::replace(&mut self.pending,next,&mut self.p,&mut self.v,&mut self.s,&mut self.t)}
    fn start_document(&mut self){assert!(self.p.start_document(&mut self.v,&mut self.s,&mut self.t,self.now));}
    fn deliver(&mut self,op:u64,word:u64){match op{trust_policy::OP=>self.v.reply(word,self.now),entropy_seed::OP=>self.s.reply(word,self.now),certificate_time::OP=>self.t.reply(word,self.now),_=>unreachable!()}}
    fn receipt(&mut self,op:u64,kind:u8,word:u64){if self.hold==Some((op,kind)){assert!(self.held.is_none());self.held=Some((op,word));}else{self.deliver(op,word);}}
    fn turn(&mut self)->Option<sex_tls::Stats>{
        self.now+=1;assert!(self.now<10_000);self.v.advance(self.now);self.s.advance(self.now);self.t.advance(self.now);
        let calls:Vec<_>=CALLS.lock().unwrap().drain(..).collect();
        for(op,args)in calls{
            if op==trust_policy::OP{
                let r=trust_policy::Request::decode(args).unwrap();self.record(op,r.seq,0);
                assert!(self.policy.handle(TRUST_OWNER,r,||Ok(trust_source::Info{consumer:TRUST_OWNER,profile:trust_config::PROFILE,root_bytes:trust_config::ROOT_DER.len(),version:trust_config::VERSION})));
                let(_,word)=self.policy.pending_reply().unwrap();self.policy.reply_accepted();self.receipt(op,0,word);
            }else if op==entropy_seed::OP{
                let r=entropy_seed::Request::decode(args).unwrap();self.record(op,r.seq,r.kind);
                assert!(self.seed.handle(OWNER,r,self.now,Some(entropy_source::Info{consumer:OWNER,available:true})));
                let words=&mut self.words;self.seed.advance(self.now,||{*words+=1;Ok(*words)});
                let(_,word)=self.seed.pending_reply().unwrap();self.seed.reply_accepted();self.receipt(op,r.kind,word);
            }else{
                let r=certificate_time::Request::decode(args).unwrap();self.record(op,r.seq,0);
                assert!(self.clock.handle(CLOCK_OWNER,r,Some(clock_source::Info{consumer:CLOCK_OWNER,source:clock_source::CMOS_UTC}),||Ok(clock_source::Snapshot{seconds:UTC,attempts:1})));
                let(_,word)=self.clock.pending_reply().unwrap();self.clock.reply_accepted();self.receipt(op,0,word);
            }
        }
        let words=&mut self.words;self.seed.advance(self.now,||{*words+=1;Ok(*words)});
        let(stats,erased)=self.p.advance(&mut self.v,&mut self.s,&mut self.t,self.now);assert_eq!(erased,0);stats
    }
    fn record(&mut self,op:u64,seq:u32,kind:u8){assert!(self.calls.iter().rev().find(|r|r.0==op).is_none_or(|r|seq>r.1),"accepted provider operation was replayed");self.calls.push((op,seq,kind));}
    fn count(&self,op:u64,kind:u8)->usize{self.calls.iter().filter(|r|r.0==op&&r.2==kind).count()}
    fn until_held(&mut self){for _ in 0..1000{self.turn();if self.held.is_some(){return;}}panic!("provider receipt never accepted");}
    fn deliver_held(&mut self){self.hold=None;let(op,word)=self.held.take().unwrap();self.deliver(op,word);}
    fn idle_providers(&mut self){for _ in 0..1000{if !self.providers(){return;}self.turn();}panic!("accepted provider cleanup did not settle");}
    fn hello(&mut self)->sex_tls::Stats{for _ in 0..1000{if let Some(stats)=self.turn(){assert_eq!(self.p.view(),tls_probe::View::HelloPending);return stats;}}panic!("no fresh actual ClientHello");}
}
#[test]fn document_to_local_prepare_erases_old_future_and_uses_fresh_policy_seed_time(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();w.start_document();let old=w.hello();assert_eq!(w.p.allocated_payload_bytes(),old.task_bytes+8192+sex_tls::MAX_DOCUMENT);
    // Negative control: the pre96 direct start rejects the requested L and
    // leaves the document-mode preparation retained instead of superseding it.
    assert!(!w.p.start(&mut w.v,&mut w.s,&mut w.t,w.now));assert_eq!(w.p.view(),tls_probe::View::HelloPending);
    let fence=(w.v.next_sequence(),w.s.next_sequence(),w.t.next_sequence());
    assert_eq!(w.replace(Intent::TlsPrepare),old.task_bytes+8192+sex_tls::MAX_DOCUMENT);assert_eq!(w.p.allocated_payload_bytes(),0);assert_eq!(w.p.view(),tls_probe::View::Cancelled);assert!(w.p.take_exchange().is_none());
    assert_eq!(w.ready(true),None);assert_eq!(w.pending,Some(Intent::TlsPrepare));assert_eq!(w.ready(false),Some(Intent::TlsPrepare));assert!(w.pending.is_none());
    assert!(w.p.start(&mut w.v,&mut w.s,&mut w.t,w.now));let fresh=w.hello();assert_eq!(w.p.allocated_payload_bytes(),fresh.task_bytes+8192);assert_eq!(fresh.utc_snapshot,UTC as u32);assert!(fresh.private_bytes<old.private_bytes);
    assert!(w.v.next_sequence()>fence.0&&w.s.next_sequence()>fence.1&&w.t.next_sequence()>fence.2);assert_eq!(w.count(trust_policy::OP,0),2);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),2);assert_eq!(w.count(certificate_time::OP,0),2);assert!(w.p.take_exchange().is_none());assert!(w.p.close()>0);assert_eq!(w.p.allocated_payload_bytes(),0);
}
#[test]fn local_prepare_to_http_immediately_erases_unused_session_without_waiting_for_deadline(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();assert!(w.p.start(&mut w.v,&mut w.s,&mut w.t,0));let old=w.hello();let calls=w.calls.len();assert_eq!(w.p.allocated_payload_bytes(),old.task_bytes+8192);
    assert_eq!(w.replace(Intent::HttpDns),old.task_bytes+8192);assert_eq!(w.p.allocated_payload_bytes(),0);assert!(!w.p.busy());assert_eq!(w.p.view(),tls_probe::View::Cancelled);assert_eq!(w.p.close(),0);assert_eq!(w.p.allocated_payload_bytes(),0);assert_eq!(w.ready(false),Some(Intent::HttpDns));
    for _ in 0..10{assert_eq!(w.turn(),None);}assert_eq!(w.calls.len(),calls);assert!(w.s.private_scrubbed());assert_eq!(w.v.take_profile(),None);assert_eq!(w.t.take_snapshot(),None);
}
#[test]fn accepted_policy_supersession_keeps_correlation_and_only_latest_intent_can_start(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();w.hold=Some((trust_policy::OP,0));w.start_document();w.until_held();assert_eq!(w.words,0);assert_eq!(w.p.allocated_payload_bytes(),0);
    assert_eq!(w.replace(Intent::TlsPrepare),0);assert_eq!(w.p.allocated_payload_bytes(),0);assert_eq!(w.replace(Intent::Seed),0);assert_eq!(w.replace(Intent::Clock),0);assert_eq!(w.pending,Some(Intent::Clock));
    let(_,word)=w.held.unwrap();w.v.reply(word^(1u64<<32),w.now);assert!(w.v.busy());
    for _ in 0..10{assert_eq!(w.ready(false),None);assert_eq!(w.turn(),None);}assert_eq!(w.count(trust_policy::OP,0),1);assert_eq!(w.words,0);assert_eq!(w.count(certificate_time::OP,0),0);
    w.deliver_held();assert_eq!(w.v.take_profile(),None);assert!(!w.v.busy());assert_eq!(w.ready(true),None);assert_eq!(w.ready(false),Some(Intent::Clock));assert_eq!(w.ready(false),None);
    assert!(w.t.start());w.turn();assert_eq!(w.t.take_snapshot(),Some(UTC as u32));assert_eq!(w.t.take_snapshot(),None);assert_eq!(w.count(certificate_time::OP,0),1);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),0);assert!(!w.p.busy());
}
#[test]fn accepted_seed_begin_read_or_release_settles_before_replacement_seed_begin(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());for kind in [entropy_seed::BEGIN,entropy_seed::READ,entropy_seed::RELEASE]{
        let mut w=World::new();w.hold=Some((entropy_seed::OP,kind));w.start_document();w.until_held();assert!(w.s.busy());assert!(!w.s.start()); // old direct-E negative control
        let old_fence=w.s.next_sequence();assert_eq!(w.replace(Intent::Seed),0);assert_eq!(w.p.allocated_payload_bytes(),0);assert!(w.s.private_scrubbed());assert_eq!(w.s.take_seed(),None);assert_eq!(w.t.take_snapshot(),None);
        for _ in 0..5{assert_eq!(w.ready(false),None);assert_eq!(w.turn(),None);}assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),1);
        w.deliver_held();w.idle_providers();assert_eq!(w.s.take_seed(),None);assert!(w.s.private_scrubbed());assert!(w.seed.owner().is_none());assert!(w.s.next_sequence()>=old_fence);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),1);
        assert_eq!(w.ready(false),Some(Intent::Seed));assert!(w.s.start());for _ in 0..1000{w.turn();if !w.s.busy(){break;}}
        assert_eq!(w.s.view(),entropy_probe::View::Ready);let mut fresh=w.s.take_seed().expect("only a new positively released seed can satisfy E");assert!(fresh.iter().any(|b|*b!=0));fresh.fill(0);assert!(w.s.private_scrubbed());assert!(w.seed.owner().is_none());assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),2);assert_eq!(w.count(trust_policy::OP,0),1);assert!(!w.p.busy());
    }
}
#[test]fn accepted_utc_supersession_discards_old_snapshot_then_issues_fresh_query(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();w.hold=Some((certificate_time::OP,0));w.start_document();w.until_held();assert!(w.t.busy());assert!(!w.t.start()); // old direct-T negative control
    w.replace(Intent::Seed);w.replace(Intent::Clock);let fence=w.t.next_sequence();for _ in 0..5{assert_eq!(w.ready(false),None);w.turn();}assert_eq!(w.count(certificate_time::OP,0),1);
    w.deliver_held();w.idle_providers();assert_eq!(w.t.take_snapshot(),None);assert_eq!(w.s.take_seed(),None);assert!(w.s.private_scrubbed());assert_eq!(w.ready(false),Some(Intent::Clock));assert!(w.t.start());w.turn();assert!(w.t.next_sequence()>fence);assert_eq!(w.t.take_snapshot(),Some(UTC as u32));assert_eq!(w.t.take_snapshot(),None);assert_eq!(w.count(certificate_time::OP,0),2);assert!(!w.p.busy());
}
#[test]fn closed_probe_and_clients_cannot_resurrect_from_queued_intent_or_accepted_late_receipt(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());for op in [trust_policy::OP,entropy_seed::OP,certificate_time::OP]{
        let mut w=World::new();w.hold=Some((op,if op==entropy_seed::OP{entropy_seed::BEGIN}else{0}));w.start_document();w.until_held();let calls=w.calls.len();
        w.p.close();assert_eq!(w.p.allocated_payload_bytes(),0);w.v.close();w.s.close();w.t.close();w.pending=None;w.deliver_held();
        for next in [Intent::TlsDns,Intent::TlsPrepare,Intent::Seed,Intent::Clock]{assert_eq!(w.replace(next),0);assert_eq!(w.ready(false),Some(next));assert!(!w.p.start_document(&mut w.v,&mut w.s,&mut w.t,w.now));assert!(!w.p.start(&mut w.v,&mut w.s,&mut w.t,w.now));assert!(!w.s.start());assert!(!w.t.start());w.turn();}
        assert_eq!(w.calls.len(),calls);assert!(CALLS.lock().unwrap().is_empty());assert!(w.p.take_exchange().is_none());assert!(w.s.private_scrubbed());assert_eq!(w.s.take_seed(),None);assert_eq!(w.t.take_snapshot(),None);assert_eq!(w.v.take_profile(),None);assert_eq!(w.p.close(),0);
        if op==entropy_seed::OP{assert!(w.seed.retire(OWNER));assert!(w.seed.owner().is_none());}
    }
}
#[test]fn repeated_document_exchange_supersession_transfers_only_latest_fresh_hello(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();
    for document in [true,false,true,false]{
        // Prepare the opposite mode first, then replace it as B <-> Y does.
        if document{assert!(w.p.start_exchange(&mut w.v,&mut w.s,&mut w.t,w.now));}else{w.start_document();}
        let abandoned=w.hello();assert_eq!(w.p.allocated_payload_bytes(),abandoned.task_bytes+8192+if document{0}else{sex_tls::MAX_DOCUMENT});let erased=w.replace(Intent::TlsDns);assert_eq!(erased,abandoned.task_bytes+8192+if document{0}else{sex_tls::MAX_DOCUMENT});assert_eq!(w.p.allocated_payload_bytes(),0);
        assert_eq!(w.ready(true),None);assert_eq!(w.ready(false),Some(Intent::TlsDns));
        if document{w.start_document();}else{assert!(w.p.start_exchange(&mut w.v,&mut w.s,&mut w.t,w.now));}
        let fresh=w.hello();assert_eq!(w.p.allocated_payload_bytes(),fresh.task_bytes+8192+if document{sex_tls::MAX_DOCUMENT}else{0});w.pending=Some(Intent::TlsStart);assert!(w.p.busy());
        assert_eq!(w.ready(true),None);assert_eq!(w.ready(false),Some(Intent::TlsStart));
        let session=w.p.take_exchange().expect("only fresh positive policy/seed/time preparation transfers");
        assert_eq!(w.p.allocated_payload_bytes(),0);assert_eq!(session.allocated_payload_bytes(),session.stats().task_bytes+8192+if document{sex_tls::MAX_DOCUMENT}else{0});assert_eq!(session.state(),sex_tls::State::HelloPending);assert_eq!(session.is_document(),document);assert!(session.outgoing_len()>100);assert!(session.failure().is_none());assert_eq!(w.p.view(),tls_probe::View::Idle);assert!(!w.p.busy());
        assert!(w.v.take_profile().is_none()&&w.s.take_seed().is_none()&&w.t.take_snapshot().is_none());drop(session);
    }
    assert_eq!(w.count(trust_policy::OP,0),8);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),8);assert_eq!(w.count(certificate_time::OP,0),8);assert!(w.s.private_scrubbed());assert!(w.seed.owner().is_none());assert_eq!(w.p.close(),0);assert_eq!(w.p.allocated_payload_bytes(),0);
}
