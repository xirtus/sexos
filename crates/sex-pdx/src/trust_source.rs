// SPDX-License-Identifier: GPL-3.0-only
//! BOOT-held immutable trust-policy metadata. No pointer, root mutation, name
//! selector, transport permission, or ambient certificate authority is exposed.
pub const SLOT:u64=62;
pub const SYS:u64=74;
pub const INFO:u64=1;
pub const DENIED:u64=u64::MAX;
pub const UNAVAILABLE:u64=u64::MAX-1;
pub const INVALID:u64=u64::MAX-3;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Identity {pub pd:u32,pub task:u32}
impl Identity {pub const fn valid(self)->bool {self.pd!=0&&self.task!=0}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Grant {pub holder:Identity,pub consumer:Identity,pub profile:u32}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Info {pub consumer:Identity,pub profile:u32,pub root_bytes:usize,pub version:u32}
impl Info {
    pub fn valid(self)->bool {
        self.consumer.valid()&&self.profile==crate::trust_config::PROFILE
            &&self.root_bytes==crate::trust_config::ROOT_DER.len()&&self.root_bytes!=0
            &&self.version==crate::trust_config::VERSION
    }
    /// Full-width profile, root size, version and packed consumer are canonical;
    /// every error must redact every data register.
    pub fn decode(raw:[u64;5])->Result<Self,u64> {
        if raw[0]!=0 {return Err(if matches!(raw[0],DENIED|UNAVAILABLE)&&raw[1..].iter().all(|v|*v==0){raw[0]}else{INVALID});}
        if raw[1]>u32::MAX as u64||raw[3]>usize::MAX as u64||raw[4]>u32::MAX as u64{return Err(INVALID);}
        let info=Self {consumer:Identity {pd:raw[2]as u32,task:(raw[2]>>32)as u32},profile:raw[1]as u32,root_bytes:raw[3]as usize,version:raw[4]as u32};
        if info.valid(){Ok(info)}else{Err(INVALID)}
    }
}
/// All arguments and exact holder identity are checked before policy metadata.
/// A different profile cannot silently select an alternative root or hostname.
pub fn invoke(grant:Option<&Grant>,caller:Identity,kind:u64,reserved:[u64;3])->[u64;5] {
    let denied=[DENIED,0,0,0,0];
    let Some(grant)=grant else{return denied;};
    if !caller.valid()||!grant.holder.valid()||!grant.consumer.valid()||caller!=grant.holder
        ||kind!=INFO||reserved!=[0;3]{return denied;}
    if grant.profile!=crate::trust_config::PROFILE||crate::trust_config::ROOT_DER.is_empty(){return [UNAVAILABLE,0,0,0,0];}
    [0,grant.profile as u64,grant.consumer.pd as u64|((grant.consumer.task as u64)<<32),
        crate::trust_config::ROOT_DER.len()as u64,crate::trust_config::VERSION as u64]
}
