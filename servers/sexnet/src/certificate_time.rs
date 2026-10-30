// SPDX-License-Identifier: GPL-3.0-only
//! Six retained read-only query receipts. Parent authenticates current BOOT
//! liveness; source metadata never supplies authority. No uptime/last-good UTC
//! fallback. Cached queries are historical snapshots, not fresh time checks.
use sex_pdx::certificate_time::{self as a,Request};
use sex_pdx::clock_source::{self as source,Identity,Info,Snapshot as SourceSnapshot};
const OWNERS:usize=6;
#[derive(Clone,Copy)]
struct Receipt {owner:Identity,request:Request,word:u64}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Snapshot {pub high_water:Option<u64>,pub rollback:bool}
pub struct Machine {
    high_water:Option<u64>,rollback:bool,
    pending:[Option<Receipt>;OWNERS],cached:[Option<Receipt>;OWNERS],cursor:usize,
}
impl Machine {
    pub const fn new()->Self {Self {high_water:None,rollback:false,pending:[None;OWNERS],cached:[None;OWNERS],cursor:0}}
    pub fn snapshot(&self)->Snapshot {Snapshot {high_water:self.high_water,rollback:self.rollback}}
    fn selected(&self)->Option<usize> {(0..OWNERS).map(|n|(self.cursor+n)%OWNERS).find(|i|self.pending[*i].is_some())}
    pub fn pending_reply(&self)->Option<(Identity,u64)> {self.selected().and_then(|i|self.pending[i]).map(|r|(r.owner,r.word))}
    pub fn reply_accepted(&mut self) {if let Some(i)=self.selected(){self.pending[i]=None;self.cursor=(i+1)%OWNERS;}}
    pub fn reply_deferred(&mut self) {if let Some(i)=self.selected(){self.cursor=(i+1)%OWNERS;}}
    fn poison(&mut self) {
        self.rollback=true;
        for slots in [&mut self.pending,&mut self.cached] {for slot in slots {
            if let Some(r)=slot.as_mut() {if a::decode_reply(r.word,r.request.seq).is_ok() {r.word=a::error(r.request.seq,a::ROLLBACK);}}
        }}
    }
    fn respond(&mut self,i:usize,owner:Identity,r:Request,result:Result<u32,u32>)->bool {
        let receipt=Receipt {owner,request:r,word:match result {Ok(v)=>a::reply(r.seq,v),Err(e)=>a::error(r.seq,e)}};
        self.pending[i]=Some(receipt);
        if self.cached[i].is_none_or(|c|r.seq>c.request.seq) {self.cached[i]=Some(receipt);}
        true
    }
    /// False means no source call or state effect: invalid correlation, six
    /// occupied owner fences, or a different request with an owner receipt pending.
    /// The same pending query coalesces without source work. Reserve the
    /// receipt slot before invoking the source; never evict an owner fence.
    pub fn handle(&mut self,owner:Identity,r:Request,info:Option<Info>,provider:impl FnOnce()->Result<SourceSnapshot,u64>)->bool {
        if !owner.valid()||!r.valid() {return false;}
        let Some(i)=self.cached.iter().position(|c|c.is_some_and(|c|c.owner==owner))
            .or_else(||self.cached.iter().position(Option::is_none)) else {return false;};
        if let Some(p)=self.pending[i] {return p.request==r;}
        if let Some(c)=self.cached[i] {
            if c.request==r {self.pending[i]=Some(c);return true;}
            if r.seq<c.request.seq {return self.respond(i,owner,r,Err(a::STALE));}
        }
        // The exclusive machine borrow and selected empty pending/fence slot
        // reserve capacity before source work; no fallible delivery occurs here.
        let result=self.sample(owner,info,provider);
        self.respond(i,owner,r,result)
    }
    fn sample(&mut self,owner:Identity,info:Option<Info>,provider:impl FnOnce()->Result<SourceSnapshot,u64>)->Result<u32,u32> {
        let info=info.ok_or(a::DENIED)?;
        if !info.consumer.valid()||info.consumer!=owner||info.source!=source::CMOS_UTC {return Err(a::DENIED);}
        if self.rollback {return Err(a::ROLLBACK);}
        let sample=provider().map_err(|error|match error {source::DENIED=>a::DENIED,source::UNAVAILABLE=>a::OFFLINE,source::BUSY=>a::BUSY,_=>a::INVALID})?;
        // Structs can be constructed without their register decoder. Reapply
        // every data invariant before touching the boot-global high-water mark.
        if sample.seconds<source::MIN_SECONDS||sample.seconds>source::MAX_SECONDS
            ||sample.attempts==0||sample.attempts>source::ATTEMPTS {return Err(a::INVALID);}
        if self.high_water.is_some_and(|high|sample.seconds<high) {self.poison();return Err(a::ROLLBACK);}
        self.high_water=Some(sample.seconds);Ok(sample.seconds as u32)
    }
    /// Exact terminal retirement removes this owner's receipts/fence only.
    /// The boot-global high-water and poison survive every owner retirement.
    pub fn retire(&mut self,owner:Identity)->bool {
        let mut removed=false;
        for i in 0..OWNERS {if self.cached[i].is_some_and(|r|r.owner==owner) {
            self.cached[i]=None;self.pending[i]=None;removed=true;
        }}
        removed
    }
}
#[cfg(test)] mod tests {
    use super::*;
    const A:Identity=Identity {pd:15,task:15};
    const B:Identity=Identity {pd:9,task:9};
    const T:u64=source::MIN_SECONDS+100;
    fn info(owner:Identity)->Option<Info> {Some(Info {consumer:owner,source:source::CMOS_UTC})}
    fn sample(seconds:u64)->Result<SourceSnapshot,u64> {Ok(SourceSnapshot {seconds,attempts:1})}
    fn forbidden()->Result<SourceSnapshot,u64> {panic!("request reached forbidden source")}
    fn call(m:&mut Machine,owner:Identity,seq:u32,value:Result<SourceSnapshot,u64>)->Result<u32,u32> {
        assert!(m.handle(owner,Request {seq},info(owner),||value));let (actual,word)=m.pending_reply().unwrap();assert_eq!(actual,owner);m.reply_accepted();a::decode_reply(word,seq)
    }
    #[test] fn authorization_and_full_slot_reservation_precede_source() {
        let mut m=Machine::new();
        for metadata in [None,info(B),Some(Info {consumer:A,source:2})] {
            let seq=m.cached.iter().flatten().map(|r|r.request.seq).max().unwrap_or(0)+1;
            assert!(m.handle(A,Request {seq},metadata,forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,seq),Err(a::DENIED));m.reply_accepted();
        }
        for owner in [Identity {pd:0,task:15},Identity {pd:15,task:0}] {assert!(!m.handle(owner,Request {seq:1},info(owner),forbidden));}
        for seq in [0,a::MAX_SEQ+1] {assert!(!m.handle(A,Request {seq},info(A),forbidden));}
        assert!(m.handle(A,Request {seq:4},info(A),||sample(T)));
        let retained=m.pending_reply();assert!(m.handle(A,Request {seq:4},info(A),forbidden));assert_eq!(m.pending_reply(),retained);assert!(!m.handle(A,Request {seq:5},info(A),forbidden));
    }
    #[test] fn cached_duplicate_and_stale_fences_never_resample_or_return_old_time_as_fresh() {
        let mut m=Machine::new();assert_eq!(call(&mut m,A,10,sample(T)),Ok(T as u32));
        assert!(m.handle(A,Request {seq:10},None,forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,10),Ok(T as u32));m.reply_accepted();
        assert!(m.handle(A,Request {seq:9},info(A),forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,9),Err(a::STALE));m.reply_accepted();
        assert_eq!(call(&mut m,A,11,Err(source::UNAVAILABLE)),Err(a::OFFLINE));
        assert!(m.handle(A,Request {seq:11},info(A),forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,11),Err(a::OFFLINE));m.reply_accepted();
        assert_eq!(call(&mut m,A,12,sample(T+1)),Ok(T as u32+1));
    }
    #[test] fn six_owner_bound_fairness_and_exact_retirement() {
        let mut m=Machine::new();for n in 1..=6 {let owner=Identity {pd:n,task:n};assert!(m.handle(owner,Request {seq:1},info(owner),||sample(T)));}
        assert!(!m.handle(A,Request {seq:1},info(A),forbidden));
        for n in 1..=6 {assert_eq!(m.pending_reply().unwrap().0,Identity {pd:n,task:n});m.reply_deferred();}
        for n in 1..=6 {assert_eq!(m.pending_reply().unwrap().0,Identity {pd:n,task:n});m.reply_accepted();}
        assert!(!m.handle(A,Request {seq:1},info(A),forbidden));
        assert!(!m.retire(Identity {pd:1,task:99}));assert!(m.retire(Identity {pd:1,task:1}));
        assert_eq!(call(&mut m,A,1,sample(T)),Ok(T as u32));assert_eq!(m.snapshot().high_water,Some(T));
    }
    #[test] fn last_finite_sequence_keeps_max_query_replayable_and_older_queries_stale() {
        let mut m=Machine::new();assert_eq!(call(&mut m,A,a::MAX_SEQ,sample(T)),Ok(T as u32));
        assert!(m.handle(A,Request {seq:1},info(A),forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,1),Err(a::STALE));m.reply_accepted();
        assert!(m.handle(A,Request {seq:a::MAX_SEQ},info(A),forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,a::MAX_SEQ),Ok(T as u32));
    }
    #[test] fn all_source_errors_and_constructed_invalid_snapshots_have_no_fallback() {
        let mut m=Machine::new();assert_eq!(call(&mut m,A,1,sample(T)),Ok(T as u32));
        for (n,(raw,expected)) in [(source::DENIED,a::DENIED),(source::UNAVAILABLE,a::OFFLINE),(source::BUSY,a::BUSY),(source::INVALID,a::INVALID),(123,a::INVALID)].into_iter().enumerate() {assert_eq!(call(&mut m,A,n as u32+2,Err(raw)),Err(expected));assert_eq!(m.snapshot(),Snapshot {high_water:Some(T),rollback:false});}
        for (n,value) in [SourceSnapshot {seconds:source::MIN_SECONDS-1,attempts:1},SourceSnapshot {seconds:source::MAX_SECONDS+1,attempts:1},SourceSnapshot {seconds:T,attempts:0},SourceSnapshot {seconds:T,attempts:5}].into_iter().enumerate() {assert_eq!(call(&mut m,A,n as u32+7,Ok(value)),Err(a::INVALID));assert_eq!(m.snapshot().high_water,Some(T));}
    }
    #[test] fn rollback_redacts_pending_and_cached_successes_globally_and_is_permanent() {
        let mut m=Machine::new();assert!(m.handle(A,Request {seq:1},info(A),||sample(T)));
        assert!(m.handle(B,Request {seq:1},info(B),||sample(T-1)));
        assert_eq!(m.snapshot(),Snapshot {high_water:Some(T),rollback:true});
        assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,1),Err(a::ROLLBACK));m.reply_accepted();
        assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,1),Err(a::ROLLBACK));m.reply_accepted();
        assert!(m.handle(A,Request {seq:1},None,forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,1),Err(a::ROLLBACK));m.reply_accepted();
        assert!(m.handle(A,Request {seq:2},info(A),forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,2),Err(a::ROLLBACK));m.reply_accepted();
        assert!(m.retire(A));assert!(m.retire(B));assert_eq!(m.snapshot(),Snapshot {high_water:Some(T),rollback:true});
        assert!(m.handle(A,Request {seq:1},info(A),forbidden));assert_eq!(a::decode_reply(m.pending_reply().unwrap().1,1),Err(a::ROLLBACK));
    }
    #[test] fn equal_forward_time_and_retire_never_reset_global_high_water() {
        let mut m=Machine::new();assert_eq!(call(&mut m,A,1,sample(T)),Ok(T as u32));assert!(m.retire(A));
        assert_eq!(call(&mut m,B,1,sample(T)),Ok(T as u32));assert_eq!(call(&mut m,B,2,sample(T+100)),Ok(T as u32+100));assert!(m.retire(B));
        assert_eq!(call(&mut m,A,1,sample(T+99)),Err(a::ROLLBACK));
    }
}
