// SPDX-License-Identifier: GPL-3.0-only
//! Explicit local TLS preparation over existing held policy/seed/time clients.
//! A fresh BOOT-sanctioned profile precedes seed/time and public fixture checks.
//! Reuses monotonic fences; no transport or authenticated peer is produced.
use crate::{clock_probe as clock, entropy_probe as entropy, trust_probe as trust};
use sex_tls::{Session, Stats};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View { Idle,Preparing,HelloPending,Unavailable,Cancelled,Rejected,TimedOut }
pub struct Probe {
    session:Option<Session>, view:View, closed:bool, deadline:Option<u64>, last:u64, profile:Option<u32>, self_check:Option<u32>,
}
impl Probe {
    pub const fn new()->Self { Self {session:None,view:View::Idle,closed:false,deadline:None,last:0,profile:None,self_check:None} }
    pub fn view(&self)->View {self.view}
    /// Public controlled-fixture success, consumed once; never a peer receipt.
    pub fn take_self_check(&mut self)->Option<u32>{self.self_check.take()}
    pub fn busy(&self)->bool {matches!(self.view,View::Preparing|View::HelloPending)&&!self.closed}
    pub fn start(&mut self,policy:&mut trust::Client,seed:&mut entropy::Client,time:&mut clock::Client,now:u64)->bool {
        if self.closed||self.busy()||policy.busy()||seed.busy()||time.busy(){return false;}
        let Some(deadline)=now.checked_add(30_000)else{self.view=View::Unavailable;return false;};
        self.profile=None;self.self_check=None;
        if !policy.start(){self.view=View::Unavailable;return false;}
        self.deadline=Some(deadline);self.last=now;self.view=View::Preparing;true
    }
    fn erase(&mut self)->usize {
        let bytes=self.session.as_ref().map_or(0,|s|s.stats().task_bytes+2*sex_tls::QUEUE_BYTES);
        self.session=None;self.deadline=None;self.profile=None;self.self_check=None;bytes
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
            let Some(mut bytes)=seed.take_seed()else{let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);};
            let Some(seconds)=time.take_snapshot()else{
                for byte in &mut bytes{unsafe{core::ptr::write_volatile(byte,0)}}
                let n=self.cancel(policy,seed,time);self.view=View::Unavailable;return(None,n);
            };
            let profile=self.profile.unwrap();
            if sex_tls::controlled_self_check(profile,seconds).is_err(){
                for byte in &mut bytes{unsafe{core::ptr::write_volatile(byte,0)}}
                let n=self.cancel(policy,seed,time);self.view=View::Rejected;return(None,n);
            }
            self.self_check=Some(profile);
            match Session::prepare(&mut bytes,seconds,profile) {
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
        }
    }
}
pub fn label(view:View)->Option<&'static[u8]> { match view {
    View::Idle=>None,View::Preparing=>Some(b"preparing"),View::HelloPending=>Some(b"hello pending; no peer"),
    View::Unavailable=>Some(b"unavailable"),View::Cancelled=>Some(b"cancelled"),View::Rejected=>Some(b"rejected"),View::TimedOut=>Some(b"timed out"),
} }
