// SPDX-License-Identifier: GPL-3.0-only
//! Explicit local TLS preparation over existing held policy/seed/time clients.
//! A fresh BOOT-sanctioned profile precedes seed/time and public fixture checks.
//! Reuses monotonic fences; no transport or authenticated peer is produced.
use crate::{clock_probe as clock, entropy_probe as entropy, trust_probe as trust};
use sex_tls::{Session, Stats};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View { Idle,Preparing,HelloPending,Unavailable,Cancelled,Rejected,TimedOut,Uncertain }
/// Work receipts describe local computation only, never peer/clock authority.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum PrepWork { FixtureCheck,ClientHello }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Work { pub kind:PrepWork,pub sequence:u64 }
pub struct Probe {
    session:Option<Session>, view:View, closed:bool, deadline:Option<u64>, last:u64, profile:Option<u32>, self_check:Option<u32>, exchange:bool,document:bool,
    checked_seconds:Option<u32>,turn:u64,work:Option<Work>,
}
impl Probe {
    pub const fn new()->Self { Self {session:None,view:View::Idle,closed:false,deadline:None,last:0,profile:None,self_check:None,exchange:false,document:false,checked_seconds:None,turn:0,work:None} }
    pub fn view(&self)->View {self.view}
    pub fn turn(&self)->u64{self.turn}
    pub fn take_work(&mut self)->Option<Work>{self.work.take()}
    pub fn allocated_payload_bytes(&self)->usize{self.session.as_ref().map_or(0,Session::allocated_payload_bytes)}
    /// Public controlled-fixture success, consumed once; never a peer receipt.
    pub fn take_self_check(&mut self)->Option<u32>{self.self_check.take()}
    pub fn busy(&self)->bool {matches!(self.view,View::Preparing|View::HelloPending)&&!self.closed}
    pub fn start(&mut self,policy:&mut trust::Client,seed:&mut entropy::Client,time:&mut clock::Client,now:u64)->bool {
        self.start_mode(policy,seed,time,now,false,false)
    }
    pub fn start_exchange(&mut self,policy:&mut trust::Client,seed:&mut entropy::Client,time:&mut clock::Client,now:u64)->bool {
        self.start_mode(policy,seed,time,now,true,false)
    }
    pub fn start_document(&mut self,policy:&mut trust::Client,seed:&mut entropy::Client,time:&mut clock::Client,now:u64)->bool{
        self.start_mode(policy,seed,time,now,true,true)
    }
    fn start_mode(&mut self,policy:&mut trust::Client,seed:&mut entropy::Client,time:&mut clock::Client,now:u64,exchange:bool,document:bool)->bool {
        if self.closed||self.busy()||policy.busy()||seed.busy()||time.busy(){return false;}
        let Some(deadline)=now.checked_add(30_000)else{self.view=View::Unavailable;return false;};
        self.profile=None;self.self_check=None;self.checked_seconds=None;self.work=None;self.exchange=exchange;self.document=document;
        if !policy.start(){self.view=View::Unavailable;return false;}
        self.deadline=Some(deadline);self.last=now;self.view=View::Preparing;true
    }
    /// Transfer the finite owned protocol state only after this fresh start's
    /// positive BOOT policy/seed/time joins and actual private ClientHello.
    pub fn take_exchange(&mut self)->Option<Session>{
        if !self.exchange||self.view!=View::HelloPending||self.closed{return None;}
        let session=self.session.take()?;self.deadline=None;self.profile=None;self.self_check=None;self.checked_seconds=None;self.work=None;self.exchange=false;self.view=View::Idle;Some(session)
    }
    fn erase(&mut self)->usize {
        let bytes=self.session.as_ref().map_or(0,|s|s.owned_payload_bytes());
        self.session=None;self.deadline=None;self.profile=None;self.self_check=None;self.checked_seconds=None;self.work=None;bytes
    }
    pub fn cancel(&mut self,policy:&mut trust::Client,seed:&mut entropy::Client,time:&mut clock::Client)->usize {
        if self.closed{return 0;}
        let bytes=self.erase();policy.cancel();seed.cancel();time.cancel();self.view=View::Cancelled;bytes
    }
    pub fn close(&mut self)->usize {
        let bytes=self.erase();self.closed=true;self.view=View::Cancelled;bytes
    }
    /// Parent advances each existing held client at most once per event turn.
    /// This method issues no PDX calls and polls protocol state at most once.
    pub fn advance(&mut self,policy:&mut trust::Client,seed:&mut entropy::Client,time:&mut clock::Client,now:u64)->(Option<Stats>,usize) {
        if !self.busy(){return(None,0);}
        self.work=None;
        let Some(turn)=self.turn.checked_add(1)else{let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);};self.turn=turn;
        if now<self.last||self.deadline.is_none_or(|end|now>=end) {
            let bytes=self.cancel(policy,seed,time);self.view=View::TimedOut;return(None,bytes);
        }
        self.last=now;
        if self.session.is_none() {
            if self.profile.is_none() {
                if policy.busy(){return(None,0);}
                let Some(profile)=policy.take_profile()else{let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);};
                if profile!=sex_pdx::trust_config::PROFILE||policy.view()!=trust::View::Ready(profile){
                    let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);
                }
                self.profile=Some(profile);
                if !time.start(){let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);}
                if !seed.start(){let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);}
            }
            let failed=(!seed.busy()&&seed.view()!=entropy::View::Ready)
                ||(!time.busy()&&!matches!(time.view(),clock::View::Ready(_)));
            if failed {let bytes=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,bytes);}
            if seed.busy()||time.busy(){return(None,0);}
            let profile=self.profile.unwrap();
            if self.checked_seconds.is_none(){
                let Some(seconds)=time.take_snapshot()else{let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);};
                // The public fixture performs two synchronous ECDSA checks.
                // Do not add ClientHello's scalar multiplication to this turn.
                // The released seed stays in its existing, cancellable client.
                self.work=Some(Work{kind:PrepWork::FixtureCheck,sequence:turn});
                if sex_tls::controlled_self_check(profile,seconds).is_err(){
                    let n=self.cancel(policy,seed,time);self.view=View::Rejected;
                    self.work=Some(Work{kind:PrepWork::FixtureCheck,sequence:turn});return(None,n);
                }
                self.self_check=Some(profile);self.checked_seconds=Some(seconds);
                return(None,0);
            }
            let seconds=self.checked_seconds.take().unwrap();
            let Some(mut bytes)=seed.take_seed()else{let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);};
            self.work=Some(Work{kind:PrepWork::ClientHello,sequence:turn});
            let prepared=if self.document {Session::document(&mut bytes,seconds,profile)}else if self.exchange {Session::exchange(&mut bytes,seconds,profile)}else{Session::prepare(&mut bytes,seconds,profile)};
            match prepared {
                Ok(session)=>self.session=Some(session),
                Err(_)=>{let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);},
            }
        }
        let session=self.session.as_mut().unwrap();
        match session.poll() {
            sex_tls::State::Preparing=>(None,0),
            sex_tls::State::HelloPending=>{
                let newly=self.view!=View::HelloPending;self.view=View::HelloPending;
                (if newly{Some(session.stats())}else{None},0)
            },
            sex_tls::State::Rejected=>{let n=self.erase();self.view=View::Rejected;(None,n)},
            sex_tls::State::PeerAuthenticated|sex_tls::State::ExchangeVerified=>{
                // Ingress is exclusively the transferred duplex client's job.
                // A retained preparation has no source of peer bytes.
                let n=self.erase();self.view=View::Rejected;(None,n)
            },
        }
    }
}
pub fn label(view:View)->Option<&'static[u8]> { match view {
    View::Idle=>None,View::Preparing=>Some(b"preparing"),View::HelloPending=>Some(b"hello pending; no peer"),
    View::Unavailable=>Some(b"unavailable"),View::Cancelled=>Some(b"cancelled"),View::Rejected=>Some(b"rejected"),View::TimedOut=>Some(b"timed out"),View::Uncertain=>Some(b"waiting receipt"),
} }
