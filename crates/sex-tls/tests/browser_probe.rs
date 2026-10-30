//! Actual Browser coordinator, held clients, ABI and brokers. Source providers
//! are public fixtures; this is not native entropy, time or network evidence.
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[path="../../sex-pdx/src/trust_source.rs"]pub mod trust_source;
#[path="../../sex-pdx/src/trust_policy.rs"]pub mod trust_policy;
#[path="../../../servers/sexnet/src/trust_policy.rs"]mod policies;
#[path="../../../apps/kaleidoscope/src/trust_probe.rs"]mod trust_probe;
#[path="../../sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../../sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../../sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../../sex-pdx/src/entropy_seed.rs"]pub mod entropy_seed;
#[path="../../sex-pdx/src/certificate_time.rs"]pub mod certificate_time;
#[path="../../../servers/sexnet/src/entropy.rs"]mod seeds;
#[path="../../../servers/sexnet/src/certificate_time.rs"]mod clocks;
#[path="../../../apps/kaleidoscope/src/entropy_probe.rs"]mod entropy_probe;
#[path="../../../apps/kaleidoscope/src/clock_probe.rs"]mod clock_probe;
#[path="../../../apps/kaleidoscope/src/tls_probe.rs"]mod tls_probe;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;
pub const ERR_SERVICE_NOT_READY:u64=9;
const UTC:u64=1_793_367_907;
const OWNER:entropy_source::Identity=entropy_source::Identity{pd:15,task:15};
const TRUST_OWNER:trust_source::Identity=trust_source::Identity{pd:15,task:15};
const CLOCK_OWNER:clock_source::Identity=clock_source::Identity{pd:15,task:15};
static SERIAL:Mutex<()>=Mutex::new(());
static CALLS:Mutex<Vec<(u64,[u64;3])>>=Mutex::new(Vec::new());
pub fn pdx_call(slot:u64,op:u64,x:u64,y:u64,z:u64)->(u64,u64) {
    assert_eq!(slot,SLOT_NET);assert!(matches!(op,entropy_seed::OP|certificate_time::OP|trust_policy::OP));
    CALLS.lock().unwrap().push((op,[x,y,z]));(0,0)
}
struct World { utc:u64,policy_queries:Vec<u32>,policy:policies::Machine,fail_policy:bool,hold_policy:bool,held_policy:Option<u64>, seed:seeds::Machine,clock:clocks::Machine,words:u64,fail_seed:bool,fail_clock:bool,hold_release:Option<u64> }
impl World {
    fn new()->Self { CALLS.lock().unwrap().clear();Self{utc:UTC,policy_queries:vec![],policy:policies::Machine::new(),fail_policy:false,hold_policy:false,held_policy:None,seed:seeds::Machine::new(),clock:clocks::Machine::new(),words:0,fail_seed:false,fail_clock:false,hold_release:None} }
    fn turn(&mut self,policy:&mut trust_probe::Client,seed:&mut entropy_probe::Client,time:&mut clock_probe::Client,now:u64,hold:bool) {
        policy.advance(now);seed.advance(now);time.advance(now);
        for(op,args)in CALLS.lock().unwrap().drain(..) {
            if op==entropy_seed::OP {
                let r=entropy_seed::Request::decode(args).unwrap();
                assert!(self.seed.handle(OWNER,r,now,Some(entropy_source::Info{consumer:OWNER,available:!self.fail_seed})));
                self.seed.advance(now,||{self.words+=1;Ok(self.words)});
                if let Some((_,word))=self.seed.pending_reply(){self.seed.reply_accepted();
                    if hold&&r.kind==entropy_seed::RELEASE{self.hold_release=Some(word);}else{seed.reply(word,now);}}
            }else if op==trust_policy::OP {
                let r=trust_policy::Request::decode(args).unwrap();self.policy_queries.push(r.seq);
                assert!(self.policy.handle(TRUST_OWNER,r,||if self.fail_policy{Err(trust_source::DENIED)}else{
                    Ok(trust_source::Info{consumer:TRUST_OWNER,profile:trust_config::PROFILE,root_bytes:trust_config::ROOT_DER.len(),version:trust_config::VERSION})
                }));
                if let Some((_,word))=self.policy.pending_reply(){self.policy.reply_accepted();if self.hold_policy{self.held_policy=Some(word);}else{policy.reply(word,now);}}
            }else{
                let r=certificate_time::Request::decode(args).unwrap();
                assert!(self.clock.handle(CLOCK_OWNER,r,Some(clock_source::Info{consumer:CLOCK_OWNER,source:clock_source::CMOS_UTC}),||{
                    if self.fail_clock{Err(clock_source::UNAVAILABLE)}else{Ok(clock_source::Snapshot{seconds:self.utc,attempts:1})}
                }));
                if let Some((_,word))=self.clock.pending_reply(){self.clock.reply_accepted();time.reply(word,now);}
            }
        }
        self.seed.advance(now,||{self.words+=1;Ok(self.words)});
    }
    fn finish(&mut self,p:&mut tls_probe::Probe,policy:&mut trust_probe::Client,seed:&mut entropy_probe::Client,time:&mut clock_probe::Client,start:u64)->sex_tls::Stats {
        for now in start..start+1000 {
            self.turn(policy,seed,time,now,false);
            if let(Some(stats),0)=p.advance(policy,seed,time,now){return stats;}
        }panic!("no private TLS hello");
    }
}
#[test] fn only_positive_release_and_fresh_time_allow_hello_then_cancel_and_fresh_retry() {
    let _serial=SERIAL.lock().unwrap();let mut w=World::new();let mut s=entropy_probe::Client::new();let mut t=clock_probe::Client::new();let mut p=tls_probe::Probe::new();let mut v=trust_probe::Client::new();
    assert!(p.start(&mut v,&mut s,&mut t,0));assert!(!p.start(&mut v,&mut s,&mut t,0));
    for now in 0..1000 {
        w.turn(&mut v,&mut s,&mut t,now,true);assert_eq!(p.advance(&mut v,&mut s,&mut t,now),(None,0));
        if w.hold_release.is_some(){break;}
    }
    assert!(s.busy());assert_eq!(p.view(),tls_probe::View::Preparing);
    s.reply(w.hold_release.take().unwrap(),1000);
    let stats=w.finish(&mut p,&mut v,&mut s,&mut t,1001);assert_eq!(stats.utc_snapshot,UTC as u32);
    assert!(s.take_seed().is_none()&&s.private_scrubbed());assert_eq!(t.take_snapshot(),None);assert_eq!(v.take_profile(),None);assert_eq!(p.take_self_check(),Some(trust_config::PROFILE));assert_eq!(p.take_self_check(),None);
    let old=s.next_sequence();assert_eq!(p.cancel(&mut v,&mut s,&mut t),stats.task_bytes+8192);
    assert_eq!(p.view(),tls_probe::View::Cancelled);assert!(p.start(&mut v,&mut s,&mut t,1100));
    assert!(s.next_sequence()>=old);assert_eq!(w.finish(&mut p,&mut v,&mut s,&mut t,1101).utc_snapshot,UTC as u32);
    assert_eq!(w.policy_queries,vec![1,2]);assert_eq!(p.close(),stats.task_bytes+8192);assert!(!p.start(&mut v,&mut s,&mut t,1200));
    assert_eq!(p.advance(&mut v,&mut s,&mut t,1200),(None,0));
}
#[test] fn missing_either_prerequisite_never_constructs_protocol_state() {
    let _serial=SERIAL.lock().unwrap();for fail_clock in [false,true] {
        let mut w=World::new();w.fail_clock=fail_clock;w.fail_seed=!fail_clock;
        let mut s=entropy_probe::Client::new();let mut t=clock_probe::Client::new();let mut p=tls_probe::Probe::new();let mut v=trust_probe::Client::new();assert!(p.start(&mut v,&mut s,&mut t,0));
        for now in 0..1000 {w.turn(&mut v,&mut s,&mut t,now,false);assert_eq!(p.advance(&mut v,&mut s,&mut t,now),(None,0));if !p.busy(){break;}}
        assert_eq!(p.view(),tls_probe::View::Unavailable);assert!(s.private_scrubbed());assert_eq!(p.close(),0);
    }
}
#[test] fn cancel_before_effect_late_reply_timeout_and_backwards_turn_never_restore_state() {
    let _serial=SERIAL.lock().unwrap();for (cancel,now) in [(true,2),(false,30_000),(false,0)] {
        let mut w=World::new();let mut s=entropy_probe::Client::new();let mut t=clock_probe::Client::new();let mut p=tls_probe::Probe::new();let mut v=trust_probe::Client::new();assert!(p.start(&mut v,&mut s,&mut t,1));
        if cancel {assert_eq!(p.cancel(&mut v,&mut s,&mut t),0);}else {assert_eq!(p.advance(&mut v,&mut s,&mut t,now),(None,0));}
        for at in 30_002..30_010 {w.turn(&mut v,&mut s,&mut t,at,false);assert_eq!(p.advance(&mut v,&mut s,&mut t,at),(None,0));}
        assert!(matches!(p.view(),tls_probe::View::Cancelled|tls_probe::View::TimedOut));assert!(s.private_scrubbed());
    }
}

#[test]fn policy_is_positive_before_seed_or_time_start_and_public_check_is_consumed_once(){
    let _serial=SERIAL.lock().unwrap();let mut w=World::new();w.hold_policy=true;
    let(mut v,mut s,mut t,mut p)=(trust_probe::Client::new(),entropy_probe::Client::new(),clock_probe::Client::new(),tls_probe::Probe::new());
    assert!(p.start(&mut v,&mut s,&mut t,0));assert!(!s.busy()&&!t.busy());
    for now in 0..10{w.turn(&mut v,&mut s,&mut t,now,false);assert_eq!(p.advance(&mut v,&mut s,&mut t,now),(None,0));}
    assert_eq!(w.policy_queries,vec![1]);assert_eq!(w.words,0);assert_eq!((s.next_sequence(),t.next_sequence()),(1,1));assert_eq!(p.take_self_check(),None);
    v.reply(w.held_policy.take().unwrap(),10);assert_eq!(p.advance(&mut v,&mut s,&mut t,10),(None,0));assert!(s.busy()&&t.busy());
    w.finish(&mut p,&mut v,&mut s,&mut t,11);assert_eq!(p.take_self_check(),Some(trust_config::PROFILE));assert_eq!(p.take_self_check(),None);
}
#[test]fn denied_or_unknown_policy_never_starts_seed_time_or_private_protocol(){
    let _serial=SERIAL.lock().unwrap();for unknown in [false,true]{let mut w=World::new();w.fail_policy=!unknown;w.hold_policy=unknown;
        let(mut v,mut s,mut t,mut p)=(trust_probe::Client::new(),entropy_probe::Client::new(),clock_probe::Client::new(),tls_probe::Probe::new());
        assert!(p.start(&mut v,&mut s,&mut t,0));w.turn(&mut v,&mut s,&mut t,1,false);
        if unknown{let word=w.held_policy.take().unwrap();v.reply((word&!0xffff_ffff)|2,2);}
        assert_eq!(p.advance(&mut v,&mut s,&mut t,2),(None,0));assert_eq!(p.view(),tls_probe::View::Unavailable);
        assert_eq!(w.words,0);assert_eq!((s.next_sequence(),t.next_sequence()),(1,1));assert_eq!(p.take_self_check(),None);assert_eq!(p.close(),0);
    }
}
#[test]fn accepted_policy_cancel_and_timeout_retain_client_fence_without_late_protocol_state(){
    let _serial=SERIAL.lock().unwrap();for cancel in [false,true]{let mut w=World::new();w.hold_policy=true;
        let(mut v,mut s,mut t,mut p)=(trust_probe::Client::new(),entropy_probe::Client::new(),clock_probe::Client::new(),tls_probe::Probe::new());
        assert!(p.start(&mut v,&mut s,&mut t,0));w.turn(&mut v,&mut s,&mut t,1,false);
        if cancel{p.cancel(&mut v,&mut s,&mut t);}else{p.advance(&mut v,&mut s,&mut t,30_000);}
        assert!(v.busy());assert!(!p.start(&mut v,&mut s,&mut t,30_001));v.reply(w.held_policy.take().unwrap(),30_002);
        assert_eq!(p.advance(&mut v,&mut s,&mut t,30_002),(None,0));assert_eq!(p.take_self_check(),None);assert_eq!(w.words,0);
        assert_eq!((s.next_sequence(),t.next_sequence()),(1,1));assert!(!v.busy());assert!(p.start(&mut v,&mut s,&mut t,30_003));
        w.hold_policy=false;w.finish(&mut p,&mut v,&mut s,&mut t,30_004);assert_eq!(w.policy_queries,vec![1,2]);
    }
}
#[test]fn terminal_close_during_policy_query_requires_parent_client_retirement_and_never_restarts(){
    let _serial=SERIAL.lock().unwrap();let mut w=World::new();w.hold_policy=true;
    let(mut v,mut s,mut t,mut p)=(trust_probe::Client::new(),entropy_probe::Client::new(),clock_probe::Client::new(),tls_probe::Probe::new());
    assert!(p.start(&mut v,&mut s,&mut t,0));w.turn(&mut v,&mut s,&mut t,1,false);assert_eq!(p.close(),0);v.close();s.close();t.close();
    v.reply(w.held_policy.take().unwrap(),2);assert_eq!(p.advance(&mut v,&mut s,&mut t,2),(None,0));assert!(!p.start(&mut v,&mut s,&mut t,3));assert_eq!(p.take_self_check(),None);assert_eq!(w.words,0);
    assert!(w.policy.retire(TRUST_OWNER));
}

#[test]fn previously_unconsumed_policy_cannot_replace_a_fresh_held_query(){
    let _serial=SERIAL.lock().unwrap();let mut w=World::new();
    let(mut v,mut s,mut t,mut p)=(trust_probe::Client::new(),entropy_probe::Client::new(),clock_probe::Client::new(),tls_probe::Probe::new());
    assert!(v.start());w.turn(&mut v,&mut s,&mut t,0,false);assert_eq!(v.view(),trust_probe::View::Ready(trust_config::PROFILE));
    assert!(p.start(&mut v,&mut s,&mut t,1));assert_eq!(v.take_profile(),None);w.hold_policy=true;w.turn(&mut v,&mut s,&mut t,2,false);
    assert_eq!(p.advance(&mut v,&mut s,&mut t,2),(None,0));assert!(!s.busy()&&!t.busy());assert_eq!(w.words,0);assert_eq!(w.policy_queries,vec![1,2]);
    v.reply(w.held_policy.take().unwrap(),3);w.finish(&mut p,&mut v,&mut s,&mut t,4);
}
#[test]fn expired_public_fixture_at_valid_source_time_rejects_and_scrubs_before_clienthello(){
    let _serial=SERIAL.lock().unwrap();let mut w=World::new();w.utc=clock_source::MAX_SECONDS;
    let(mut v,mut s,mut t,mut p)=(trust_probe::Client::new(),entropy_probe::Client::new(),clock_probe::Client::new(),tls_probe::Probe::new());
    assert!(p.start(&mut v,&mut s,&mut t,0));for now in 0..1000{
        w.turn(&mut v,&mut s,&mut t,now,false);assert_eq!(p.advance(&mut v,&mut s,&mut t,now),(None,0));if !p.busy(){break;}
    }
    assert_eq!(p.view(),tls_probe::View::Rejected);assert_eq!(p.take_self_check(),None);assert!(s.private_scrubbed());assert_eq!(s.take_seed(),None);assert_eq!(t.take_snapshot(),None);assert_eq!(p.close(),0);
}
