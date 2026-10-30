// SPDX-License-Identifier: GPL-3.0-only
//! Six fair retained immutable-policy query fences. The caller must attest live
//! exact incarnation before entry and before delivery. No transport is granted.
use sex_pdx::trust_policy::{self as a,Request};
use sex_pdx::trust_source::{self as source,Identity,Info};
const OWNERS:usize=6;
#[derive(Clone,Copy)]
struct Receipt {owner:Identity,request:Request,word:u64}
pub struct Machine {pending:[Option<Receipt>;OWNERS],cached:[Option<Receipt>;OWNERS],cursor:usize}
impl Machine {
    pub const fn new()->Self {Self {pending:[None;OWNERS],cached:[None;OWNERS],cursor:0}}
    fn selected(&self)->Option<usize>{(0..OWNERS).map(|n|(self.cursor+n)%OWNERS).find(|i|self.pending[*i].is_some())}
    pub fn pending_reply(&self)->Option<(Identity,u64)>{self.selected().and_then(|i|self.pending[i]).map(|r|(r.owner,r.word))}
    pub fn reply_accepted(&mut self){if let Some(i)=self.selected(){self.pending[i]=None;self.cursor=(i+1)%OWNERS;}}
    pub fn reply_deferred(&mut self){if let Some(i)=self.selected(){self.cursor=(i+1)%OWNERS;}}
    fn respond(&mut self,i:usize,owner:Identity,r:Request,result:Result<u32,u32>)->bool {
        let receipt=Receipt {owner,request:r,word:match result {Ok(v)=>a::reply(r.seq,v),Err(e)=>a::error(r.seq,e)}};
        self.pending[i]=Some(receipt);
        if self.cached[i].is_none_or(|c|r.seq>c.request.seq){self.cached[i]=Some(receipt);}true
    }
    /// Reserve retained capacity before querying the held source. Duplicates
    /// coalesce; older sequences never query; a full service never evicts fences.
    pub fn handle(&mut self,owner:Identity,r:Request,provider:impl FnOnce()->Result<Info,u64>)->bool {
        if !owner.valid()||!r.valid(){return false;}
        let Some(i)=self.cached.iter().position(|c|c.is_some_and(|c|c.owner==owner))
            .or_else(||self.cached.iter().position(Option::is_none))else{return false;};
        if let Some(p)=self.pending[i]{return p.request==r;}
        if let Some(c)=self.cached[i]{
            if c.request==r{self.pending[i]=Some(c);return true;}
            if r.seq<c.request.seq{return self.respond(i,owner,r,Err(a::STALE));}
        }
        let result=match provider(){
            Ok(info) if !info.valid()=>Err(a::INVALID),
            Ok(info) if info.consumer!=owner=>Err(a::DENIED),
            Ok(info)=>Ok(info.profile),
            Err(source::DENIED)=>Err(a::DENIED),Err(source::UNAVAILABLE)=>Err(a::OFFLINE),Err(_)=>Err(a::INVALID),
        };
        self.respond(i,owner,r,result)
    }
    /// Remove only this exact incarnation, before the service cleanup ACK.
    pub fn retire(&mut self,owner:Identity)->bool {
        let mut removed=false;
        for i in 0..OWNERS{if self.cached[i].is_some_and(|r|r.owner==owner){self.cached[i]=None;self.pending[i]=None;removed=true;}}
        removed
    }
}
