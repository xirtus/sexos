//! Actual TLS preparation turns, held clients and provider brokers.
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
    calls:Vec<(u64,u32,u8)>,words:u64,now:u64,utc:u64,
}
impl World{
    fn new()->Self{CALLS.lock().unwrap().clear();Self{p:tls_probe::Probe::new(),v:trust_probe::Client::new(),s:entropy_probe::Client::new(),t:clock_probe::Client::new(),policy:policies::Machine::new(),seed:seeds::Machine::new(),clock:clocks::Machine::new(),pending:None,hold:None,held:None,calls:vec![],words:0,now:0,utc:UTC}}
    fn providers(&self)->bool{self.v.busy()||self.s.busy()||self.t.busy()}
    fn ready(&mut self,route:bool)->Option<Intent>{let busy=Busy{route,providers:self.providers(),tls:self.p.busy()};browser_command::take_ready(&mut self.pending,busy)}
    fn replace(&mut self,next:Intent)->usize{browser_command::replace(&mut self.pending,next,&mut self.p,&mut self.v,&mut self.s,&mut self.t)}
    fn start_document(&mut self){assert!(self.p.start_document(&mut self.v,&mut self.s,&mut self.t,self.now));}
    fn deliver(&mut self,op:u64,word:u64){match op{trust_policy::OP=>self.v.reply(word,self.now),entropy_seed::OP=>self.s.reply(word,self.now),certificate_time::OP=>self.t.reply(word,self.now),_=>unreachable!()}}
    fn receipt(&mut self,op:u64,kind:u8,word:u64){if self.hold==Some((op,kind)){assert!(self.held.is_none());self.held=Some((op,word));}else{self.deliver(op,word);}}
    fn turn(&mut self)->Option<sex_tls::Stats>{
        self.now+=1;assert!(self.now<100_000);self.v.advance(self.now);self.s.advance(self.now);self.t.advance(self.now);
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
                assert!(self.clock.handle(CLOCK_OWNER,r,Some(clock_source::Info{consumer:CLOCK_OWNER,source:clock_source::CMOS_UTC}),||Ok(clock_source::Snapshot{seconds:self.utc,attempts:1})));
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

#[derive(Clone,Copy)]enum Mode{Local,Exchange,Document}
impl World{
    fn start(&mut self,mode:Mode){let ok=match mode{
        Mode::Local=>self.p.start(&mut self.v,&mut self.s,&mut self.t,self.now),
        Mode::Exchange=>self.p.start_exchange(&mut self.v,&mut self.s,&mut self.t,self.now),
        Mode::Document=>self.p.start_document(&mut self.v,&mut self.s,&mut self.t,self.now),
    };assert!(ok);}
    fn fixture(&mut self)->tls_probe::Work{
        for _ in 0..1000{
            let before=self.p.turn();assert!(self.turn().is_none());assert_eq!(self.p.turn(),before+1);
            if let Some(work)=self.p.take_work(){
                assert_eq!(work.kind,tls_probe::PrepWork::FixtureCheck);assert_eq!(work.sequence,self.p.turn());assert!(self.p.take_work().is_none());
                assert_eq!(self.p.take_self_check(),Some(trust_config::PROFILE));assert_eq!(self.p.take_self_check(),None);
                self.assert_interstage();return work;
            }
            assert_eq!(self.p.take_self_check(),None);assert_eq!(self.p.allocated_payload_bytes(),0);
        }panic!("no actual fixture-check work");
    }
    fn assert_interstage(&mut self){
        assert_eq!(self.p.view(),tls_probe::View::Preparing);assert_eq!(self.p.allocated_payload_bytes(),0);assert!(self.p.take_exchange().is_none());
        assert_eq!(self.s.view(),entropy_probe::View::Ready);assert!(!self.s.busy());assert!(!self.s.private_scrubbed());assert!(self.seed.owner().is_none());
        assert_eq!(self.t.take_snapshot(),None);assert_eq!(self.v.take_profile(),None);
    }
    fn next_hello(&mut self,fixture:tls_probe::Work)->sex_tls::Stats{
        let calls=self.calls.len();let stats=self.turn().expect("the following explicit turn constructs actual ClientHello");
        let work=self.p.take_work().expect("ClientHello work is observable once");assert_eq!(work.kind,tls_probe::PrepWork::ClientHello);
        assert_eq!(work.sequence,fixture.sequence+1);assert_eq!(work.sequence,self.p.turn());assert!(self.p.take_work().is_none());assert_eq!(self.p.take_self_check(),None);
        assert_eq!(self.p.view(),tls_probe::View::HelloPending);assert_eq!(stats.utc_snapshot,self.utc as u32);assert!(self.p.allocated_payload_bytes()>8192);
        assert_eq!(self.s.take_seed(),None);assert!(self.s.private_scrubbed());assert_eq!(self.calls.len(),calls);stats
    }
    fn idle_stays_erased(&mut self){
        let turn=self.p.turn();let calls=self.calls.len();for _ in 0..4{assert!(self.turn().is_none());assert_eq!(self.p.turn(),turn);assert!(self.p.take_work().is_none());assert_eq!(self.p.take_self_check(),None);assert_eq!(self.p.allocated_payload_bytes(),0);}
        assert_eq!(self.calls.len(),calls);assert!(self.s.private_scrubbed());assert_eq!(self.s.take_seed(),None);assert!(self.p.take_exchange().is_none());
    }
}
#[test]fn repeated_l_b_y_separate_public_fixture_check_and_hello_into_distinct_active_turns(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();let mut last=0;
    for mode in [Mode::Local,Mode::Document,Mode::Exchange,Mode::Document,Mode::Local,Mode::Exchange]{
        w.start(mode);let fixture=w.fixture();assert!(fixture.sequence>last);let stats=w.next_hello(fixture);last=w.p.turn();
        match mode{
            Mode::Local=>{assert!(w.p.take_exchange().is_none());assert_eq!(w.p.cancel(&mut w.v,&mut w.s,&mut w.t),stats.task_bytes+8192);},
            Mode::Document|Mode::Exchange=>{let session=w.p.take_exchange().unwrap();assert_eq!(session.state(),sex_tls::State::HelloPending);assert_eq!(session.is_document(),matches!(mode,Mode::Document));assert!(session.outgoing_len()>100);drop(session);},
        }
        assert_eq!(w.p.allocated_payload_bytes(),0);assert!(w.p.take_work().is_none());assert_eq!(w.p.turn(),last);
    }
    assert_eq!(w.count(trust_policy::OP,0),6);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),6);assert_eq!(w.count(certificate_time::OP,0),6);assert_eq!(w.p.close(),0);
}
#[test]fn interstage_q_wipes_provider_seed_and_cannot_produce_late_hello_or_reuse_work(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());for mode in [Mode::Local,Mode::Exchange,Mode::Document]{
        let mut w=World::new();w.start(mode);let old=w.fixture();assert_eq!(w.p.cancel(&mut w.v,&mut w.s,&mut w.t),0);assert_eq!(w.p.view(),tls_probe::View::Cancelled);w.idle_stays_erased();
        w.utc+=1;w.start(Mode::Document);let fresh=w.fixture();assert!(fresh.sequence>old.sequence);w.next_hello(fresh);assert!(w.p.take_exchange().unwrap().is_document());assert_eq!(w.p.allocated_payload_bytes(),0);
        assert_eq!(w.count(trust_policy::OP,0),2);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),2);assert_eq!(w.count(certificate_time::OP,0),2);
    }
}
#[test]fn interstage_close_clears_unconsumed_work_and_fixture_receipts_and_never_resurrects(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();w.start(Mode::Document);
    for _ in 0..1000{assert!(w.turn().is_none());if w.s.view()==entropy_probe::View::Ready{break;}}
    // Leave both one-shot diagnostic receipts unconsumed until terminal Close.
    w.assert_interstage();let turn=w.p.turn();assert_eq!(w.p.close(),0);w.v.close();w.s.close();w.t.close();assert!(w.p.take_work().is_none());assert_eq!(w.p.take_self_check(),None);w.idle_stays_erased();
    assert!(!w.p.start_document(&mut w.v,&mut w.s,&mut w.t,w.now));assert!(!w.p.start(&mut w.v,&mut w.s,&mut w.t,w.now));assert_eq!(w.p.turn(),turn);assert_eq!(w.p.allocated_payload_bytes(),0);assert_eq!(w.p.close(),0);
}
#[test]fn interstage_latest_command_replacement_discards_old_seed_and_checked_time(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();w.start(Mode::Document);let old=w.fixture();
    assert_eq!(w.replace(Intent::Seed),0);assert_eq!(w.replace(Intent::TlsPrepare),0);assert_eq!(w.ready(true),None);assert_eq!(w.ready(false),Some(Intent::TlsPrepare));assert!(w.s.private_scrubbed());assert!(w.p.take_work().is_none());assert_eq!(w.p.take_self_check(),None);
    w.utc+=2;w.start(Mode::Local);let fresh=w.fixture();assert!(fresh.sequence>old.sequence);let stats=w.next_hello(fresh);assert_eq!(stats.utc_snapshot,(UTC+2)as u32);assert!(w.p.take_exchange().is_none());assert!(w.p.close()>0);
    assert_eq!(w.count(trust_policy::OP,0),2);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),2);assert_eq!(w.count(certificate_time::OP,0),2);
}
#[test]fn changed_source_date_rechecks_same_public_profile_and_rejects_expired_fixture(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();w.start(Mode::Local);let old=w.fixture();w.p.cancel(&mut w.v,&mut w.s,&mut w.t);
    w.utc=clock_source::MAX_SECONDS;w.start(Mode::Document);let mut work=vec![];for _ in 0..1000{assert!(w.turn().is_none());if let Some(item)=w.p.take_work(){work.push(item);}assert_eq!(w.p.take_self_check(),None);if !w.p.busy(){break;}}
    assert_eq!(w.p.view(),tls_probe::View::Rejected);assert_eq!(w.p.allocated_payload_bytes(),0);assert!(w.s.private_scrubbed());assert_eq!(w.s.take_seed(),None);assert!(w.p.take_exchange().is_none());assert!(w.p.turn()>old.sequence);assert!(work.iter().all(|item|item.kind==tls_probe::PrepWork::FixtureCheck));
    assert_eq!(w.count(trust_policy::OP,0),2);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),2);assert_eq!(w.count(certificate_time::OP,0),2);assert_eq!(w.p.close(),0);
}
#[test]fn accepted_seed_release_and_policy_receipts_never_start_preparation_work_early(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());for(op,kind)in[(trust_policy::OP,0),(entropy_seed::OP,entropy_seed::RELEASE)]{
        let mut w=World::new();w.hold=Some((op,kind));w.start(Mode::Document);w.until_held();let calls=w.calls.len();for _ in 0..5{assert!(w.turn().is_none());assert!(w.p.take_work().is_none());assert_eq!(w.p.take_self_check(),None);assert_eq!(w.p.allocated_payload_bytes(),0);}assert_eq!(w.calls.len(),calls);
        w.deliver_held();let fixture=w.fixture();w.next_hello(fixture);assert_eq!(w.count(trust_policy::OP,0),1);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),1);assert_eq!(w.count(certificate_time::OP,0),1);w.p.cancel(&mut w.v,&mut w.s,&mut w.t);
    }
}
#[test]fn interstage_deadline_or_backwards_time_erases_seed_without_clienthello(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());for backwards in [false,true]{
        let mut w=World::new();w.start(Mode::Document);w.fixture();let at=if backwards{w.now-1}else{30_000};
        assert_eq!(w.p.advance(&mut w.v,&mut w.s,&mut w.t,at),(None,0));assert_eq!(w.p.view(),tls_probe::View::TimedOut);assert!(w.p.take_work().is_none());assert_eq!(w.p.take_self_check(),None);assert_eq!(w.p.allocated_payload_bytes(),0);assert!(w.s.private_scrubbed());assert_eq!(w.s.take_seed(),None);assert!(w.p.take_exchange().is_none());assert_eq!(w.p.close(),0);
    }
}
#[test]fn qoi_resource_preparation_joins_fresh_providers_and_preserves_typed_transfer(){
    let _serial=SERIAL.lock().unwrap_or_else(|e|e.into_inner());let mut w=World::new();
    assert!(w.p.start_resource(&mut w.v,&mut w.s,&mut w.t,w.now,sex_tls::Resource::ArtworkQoi));let fixture=w.fixture();let stats=w.next_hello(fixture);
    let mut session=w.p.take_exchange().unwrap();assert_eq!(session.resource_kind(),Some(sex_tls::Resource::ArtworkQoi));assert!(!session.is_document());assert!(session.take_asset().is_none());assert!(session.take_document().is_none());assert!(stats.task_bytes<=sex_tls::MAX_TASK_BYTES);drop(session);assert_eq!(w.p.allocated_payload_bytes(),0);
    assert_eq!(w.count(trust_policy::OP,0),1);assert_eq!(w.count(entropy_seed::OP,entropy_seed::BEGIN),1);assert_eq!(w.count(certificate_time::OP,0),1);
    w.start(Mode::Document);let fixture=w.fixture();w.next_hello(fixture);assert_eq!(w.p.take_exchange().unwrap().resource_kind(),Some(sex_tls::Resource::WelcomeText));
    assert!(w.p.start_resource(&mut w.v,&mut w.s,&mut w.t,w.now,sex_tls::Resource::ArtworkQoi));w.fixture();assert_eq!(w.p.cancel(&mut w.v,&mut w.s,&mut w.t),0);w.idle_stays_erased();assert_eq!(w.p.close(),0);assert!(!w.p.start_resource(&mut w.v,&mut w.s,&mut w.t,w.now,sex_tls::Resource::ArtworkQoi));
}
