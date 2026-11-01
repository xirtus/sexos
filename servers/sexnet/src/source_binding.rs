// SPDX-License-Identifier: GPL-3.0-only
//! Immutable metadata association assembled at Net startup from its actual
//! held lifecycle and typed source INFO calls. This helper neither queries nor
//! grants capabilities. The caller must supply results of those exact slot
//! queries in the current holder context, never app-provided descriptors.
use sex_pdx::{app_catalog::{self,App},app_lifecycle::CatalogInfo,browser_sources::{self,Plan},entropy_source,clock_source,trust_source};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Binding {
    plan:Plan,
    task:u32,
    entropy:entropy_source::Info,
    clock:clock_source::Info,
    trust:trust_source::Info,
}
impl Binding {
    /// Slot labels catch accidental crossed query results, but are metadata;
    /// they do not establish that a syscall occurred or create source authority.
    /// Attempt/precision/RDSEED-profile validation belongs to actual typed INFO
    /// decoders: those fields are intentionally absent from their Info structs.
    pub fn from_held(
        plan:Plan,
        catalog:(usize,Result<CatalogInfo,u64>),
        entropy:(u64,Result<entropy_source::Info,u64>),
        clock:(u64,Result<clock_source::Info,u64>),
        trust:(u64,Result<trust_source::Info,u64>),
    )->Option<Self>{
        if browser_sources::by_target(plan.target())!=Some(&plan){return None}
        let row=app_catalog::by_surface(plan.target(),plan.surface())?;
        if row.app!=App::Browser || row.lifecycle_slot!=plan.lifecycle_slot(){return None}
        if catalog.0!=plan.lifecycle_slot() || entropy.0!=plan.entropy_slot()
            || clock.0!=plan.clock_slot() || trust.0!=plan.trust_slot(){return None}
        let c=catalog.1.ok()?;
        if !c.is_subject(row.target,row.surface,3){return None}
        let entropy=entropy.1.ok()?;let clock=clock.1.ok()?;let trust=trust.1.ok()?;
        if entropy.consumer!=(entropy_source::Identity{pd:c.target_pd,task:c.task_id})
            || clock.consumer!=(clock_source::Identity{pd:c.target_pd,task:c.task_id})
            || trust.consumer!=(trust_source::Identity{pd:c.target_pd,task:c.task_id})
            || clock.source!=clock_source::CMOS_UTC || !trust.valid(){return None}
        // available=false is still a valid held RDSEED grant. The existing
        // source machine reports Offline; never substitute a different slot.
        Some(Self{plan,task:c.task_id,entropy,clock,trust})
    }
    pub const fn plan(self)->Plan{self.plan}
    pub const fn target(self)->u32{self.plan.target()}
    pub const fn task(self)->u32{self.task}
    pub const fn entropy_info(self)->entropy_source::Info{self.entropy}
    pub const fn clock_info(self)->clock_source::Info{self.clock}
    pub const fn trust_info(self)->trust_source::Info{self.trust}
    pub const fn entropy_slot(self)->u64{self.plan.entropy_slot()}
    pub const fn clock_slot(self)->u64{self.plan.clock_slot()}
    pub const fn trust_slot(self)->u64{self.plan.trust_slot()}
    pub const fn matches(self,pd:u32,task:u32)->bool{pd!=0&&task!=0&&pd==self.plan.target()&&task==self.task}
}
/// Selection consumes an already authenticated/captured owner, never a mutable
/// current Browser. Ambiguous duplicate entries fail closed. Lifecycle checks
/// remain required before provider work/reply; this lookup proves no liveness.
pub fn lookup(bindings:&[Option<Binding>],pd:u32,task:u32)->Option<&Binding>{
    let mut found=bindings.iter().flatten().filter(|b|b.matches(pd,task));
    let binding=found.next()?;
    if found.next().is_some(){return None}
    Some(binding)
}
