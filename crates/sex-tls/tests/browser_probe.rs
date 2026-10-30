//! Actual Browser coordinator, held clients, ABI and brokers. Source providers
//! are public fixtures; this is not native entropy, time or network evidence.
extern crate self as sex_pdx;
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
const CLOCK_OWNER:clock_source::Identity=clock_source::Identity{pd:15,task:15};
static SERIAL:Mutex<()>=Mutex::new(());
static CALLS:Mutex<Vec<(u64,[u64;3])>>=Mutex::new(Vec::new());
pub fn pdx_call(slot:u64,op:u64,x:u64,y:u64,z:u64)->(u64,u64) {
    assert_eq!(slot,SLOT_NET);assert!(matches!(op,entropy_seed::OP|certificate_time::OP));
    CALLS.lock().unwrap().push((op,[x,y,z]));(0,0)
}
struct World { seed:seeds::Machine,clock:clocks::Machine,words:u64,fail_seed:bool,fail_clock:bool,hold_release:Option<u64> }
impl World {
    fn new()->Self { CALLS.lock().unwrap().clear();Self{seed:seeds::Machine::new(),clock:clocks::Machine::new(),words:0,fail_seed:false,fail_clock:false,hold_release:None} }
    fn turn(&mut self,seed:&mut entropy_probe::Client,time:&mut clock_probe::Client,now:u64,hold:bool) {
        seed.advance(now);time.advance(now);
        for(op,args)in CALLS.lock().unwrap().drain(..) {
            if op==entropy_seed::OP {
                let r=entropy_seed::Request::decode(args).unwrap();
                assert!(self.seed.handle(OWNER,r,now,Some(entropy_source::Info{consumer:OWNER,available:!self.fail_seed})));
                self.seed.advance(now,||{self.words+=1;Ok(self.words)});
                if let Some((_,word))=self.seed.pending_reply(){self.seed.reply_accepted();
                    if hold&&r.kind==entropy_seed::RELEASE{self.hold_release=Some(word);}else{seed.reply(word,now);}}
            }else{
                let r=certificate_time::Request::decode(args).unwrap();
                assert!(self.clock.handle(CLOCK_OWNER,r,Some(clock_source::Info{consumer:CLOCK_OWNER,source:clock_source::CMOS_UTC}),||{
                    if self.fail_clock{Err(clock_source::UNAVAILABLE)}else{Ok(clock_source::Snapshot{seconds:UTC,attempts:1})}
                }));
                if let Some((_,word))=self.clock.pending_reply(){self.clock.reply_accepted();time.reply(word,now);}
            }
        }
        self.seed.advance(now,||{self.words+=1;Ok(self.words)});
    }
    fn finish(&mut self,p:&mut tls_probe::Probe,seed:&mut entropy_probe::Client,time:&mut clock_probe::Client,start:u64)->sex_tls::Stats {
        for now in start..start+1000 {
            self.turn(seed,time,now,false);
            if let(Some(stats),0)=p.advance(seed,time,now){return stats;}
        }panic!("no private TLS hello");
    }
}
#[test] fn only_positive_release_and_fresh_time_allow_hello_then_cancel_and_fresh_retry() {
    let _serial=SERIAL.lock().unwrap();let mut w=World::new();let mut s=entropy_probe::Client::new();let mut t=clock_probe::Client::new();let mut p=tls_probe::Probe::new();
    assert!(p.start(&mut s,&mut t,0));assert!(!p.start(&mut s,&mut t,0));
    for now in 0..1000 {
        w.turn(&mut s,&mut t,now,true);assert_eq!(p.advance(&mut s,&mut t,now),(None,0));
        if w.hold_release.is_some(){break;}
    }
    assert!(s.busy());assert_eq!(p.view(),tls_probe::View::Preparing);
    s.reply(w.hold_release.take().unwrap(),1000);
    let stats=w.finish(&mut p,&mut s,&mut t,1001);assert_eq!(stats.utc_snapshot,UTC as u32);
    assert!(s.take_seed().is_none()&&s.private_scrubbed());assert_eq!(t.take_snapshot(),None);
    let old=s.next_sequence();assert_eq!(p.cancel(&mut s,&mut t),stats.task_bytes+8192);
    assert_eq!(p.view(),tls_probe::View::Cancelled);assert!(p.start(&mut s,&mut t,1100));
    assert!(s.next_sequence()>=old);assert_eq!(w.finish(&mut p,&mut s,&mut t,1101).utc_snapshot,UTC as u32);
    assert_eq!(p.close(),stats.task_bytes+8192);assert!(!p.start(&mut s,&mut t,1200));
    assert_eq!(p.advance(&mut s,&mut t,1200),(None,0));
}
#[test] fn missing_either_prerequisite_never_constructs_protocol_state() {
    let _serial=SERIAL.lock().unwrap();for fail_clock in [false,true] {
        let mut w=World::new();w.fail_clock=fail_clock;w.fail_seed=!fail_clock;
        let mut s=entropy_probe::Client::new();let mut t=clock_probe::Client::new();let mut p=tls_probe::Probe::new();assert!(p.start(&mut s,&mut t,0));
        for now in 0..1000 {w.turn(&mut s,&mut t,now,false);assert_eq!(p.advance(&mut s,&mut t,now),(None,0));if !p.busy(){break;}}
        assert_eq!(p.view(),tls_probe::View::Unavailable);assert!(s.private_scrubbed());assert_eq!(p.close(),0);
    }
}
#[test] fn cancel_before_effect_late_reply_timeout_and_backwards_turn_never_restore_state() {
    let _serial=SERIAL.lock().unwrap();for (cancel,now) in [(true,2),(false,30_000),(false,0)] {
        let mut w=World::new();let mut s=entropy_probe::Client::new();let mut t=clock_probe::Client::new();let mut p=tls_probe::Probe::new();assert!(p.start(&mut s,&mut t,1));
        if cancel {assert_eq!(p.cancel(&mut s,&mut t),0);}else {assert_eq!(p.advance(&mut s,&mut t,now),(None,0));}
        for at in 30_002..30_010 {w.turn(&mut s,&mut t,at,false);assert_eq!(p.advance(&mut s,&mut t,at),(None,0));}
        assert!(matches!(p.view(),tls_probe::View::Cancelled|tls_probe::View::TimedOut));assert!(s.private_scrubbed());
    }
}
