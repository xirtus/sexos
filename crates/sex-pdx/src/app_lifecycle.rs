// SPDX-License-Identifier: GPL-3.0-only
//! Immutable BOOT_DAG app lifecycle grants. Exit notifications are hints;
//! every service attests custody through its own capability before retirement.
pub const SLOTS: [usize; 6] = [38, 39, 40, 45, 50, 54];
pub const OP_APP_EXIT: u64 = 0x107;
pub const OP_APP_CLOSE: u64 = 0x108;
pub const OP_APP_PRESENT: u64 = 0x109;
pub const CLOSE_SLOTS: [usize;3] = [41,42,43];
/// An app holding unsaved work may decline an ordinary close request once;
/// the user's next Close for that incarnation is sent as confirmed.
pub const OP_APP_CLOSE_DECLINED: u64 = 0x10A;
pub const CLOSE_ASK: u64 = 0;
pub const CLOSE_CONFIRMED: u64 = 1;
pub const DECLINE_UNSAVED: u64 = 1;
/// Cooperative app finish is authorized only by the static shell coordinator.
/// Delivery is not death; services must still attest kernel custody separately.
/// Returns CLOSE_ASK or CLOSE_CONFIRMED.
pub fn close_request(opcode:u64,caller:u32,args:[u64;3],target:u32,task:u32)->Option<u64> {
    (opcode==OP_APP_CLOSE && caller==3 && target!=0 && task!=0
        && args[0]==target as u64 && args[1]==task as u64 && args[2]<=CLOSE_CONFIRMED).then_some(args[2])
}
pub fn close_requested(opcode:u64,caller:u32,args:[u64;3],target:u32,task:u32)->bool {
    close_request(opcode,caller,args,target,task).is_some()
}

pub const SYS_INFO: u64 = 63;
pub const SYS_ACK: u64 = 64;
pub const SYS_RETIRE: u64 = 65;
pub const ROLE_SHELL: u64 = 1;
pub const ROLE_DISPLAY: u64 = 2;
pub const ROLE_FILES: u64 = 4;
pub const ROLE_NET: u64 = 8;
pub const ROLE_BELL: u64 = 16;
pub const REQUIRED_SERVICES: u64 = 31;
pub const OK: u64 = 0;
pub const INVALID: u64 = 1;
pub const RETRY: u64 = 2;
pub const DENIED: u64 = 3;
pub const LIVE: u64 = 5;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogInfo { pub target_pd: u32, pub task_id: u32, pub surface_id: u64, pub coordinator_pd: u32 }
impl CatalogInfo {
    /// Compare authenticated IPC metadata with this already validated held
    /// lifecycle catalog. The numbers are correlation data, not capabilities.
    pub fn is_sender(self, caller_pd:u32, caller_task:u32)->bool {
        caller_pd!=0 && caller_task!=0 && self.target_pd==caller_pd && self.task_id==caller_task
    }
    /// A successful kernel catalog query binds the exact task. Numeric domain
    /// and task identifiers are separate namespaces; zero is never a subject.
    pub fn is_subject(self,target:u32,surface:u64,coordinator:u32)->bool {
        target!=0 && surface!=0 && coordinator!=0 && target!=coordinator
            && self.target_pd==target && self.task_id!=0
            && self.surface_id==surface && self.coordinator_pd==coordinator
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleInfo { pub target_pd: u32, pub task_id: u32, pub required_mask: u64, pub acked_mask: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetiredPool { pub base: u64, pub length: u64, pub task_id: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitNotice { pub target_pd: u32, pub task_id: u32 }
impl ExitNotice {
    pub fn decode(opcode: u64, caller: u64, args: [u64; 3]) -> Result<Self, u64> {
        if opcode != OP_APP_EXIT || caller != 0 || args[0] == 0 || args[0] > u32::MAX as u64
            || args[1] == 0 || args[1] > u32::MAX as u64 || args[2] != 0 { return Err(INVALID); }
        Ok(Self { target_pd: args[0] as u32, task_id: args[1] as u32 })
    }
}
#[cfg(target_arch = "x86_64")]
fn query(slot: usize, kind: u64) -> Result<[u64;4],u64> {
    let (status,a,b,c,d): (u64,u64,u64,u64,u64);
    unsafe { core::arch::asm!("syscall", inlateout("rax") SYS_INFO => status,
        in("rdi") slot as u64, inlateout("rsi") kind => a,
        lateout("rdx") b, lateout("r10") c, lateout("r8") d,
        out("rcx") _, out("r11") _, options(nostack)); }
    if status != OK { return Err(status); }
    Ok([a,b,c,d])
}
#[cfg(target_arch = "x86_64")]
pub fn catalog(slot: usize) -> Result<CatalogInfo,u64> {
    let r=query(slot,1)?;
    Ok(CatalogInfo { target_pd:u32::try_from(r[0]).map_err(|_|INVALID)?, task_id:u32::try_from(r[1]).map_err(|_|INVALID)?, surface_id:r[2], coordinator_pd:u32::try_from(r[3]).map_err(|_|INVALID)? })
}
#[cfg(target_arch = "x86_64")]
pub fn info(slot: usize) -> Result<LifecycleInfo,u64> {
    let r=query(slot,0)?;
    Ok(LifecycleInfo { target_pd:u32::try_from(r[0]).map_err(|_|INVALID)?, task_id:u32::try_from(r[1]).map_err(|_|INVALID)?, required_mask:r[2], acked_mask:r[3] })
}
#[cfg(target_arch = "x86_64")]
pub fn ack(slot: usize, task_id: u32) -> Result<(),u64> {
    let status:u64;
    unsafe { core::arch::asm!("syscall",inlateout("rax") SYS_ACK=>status,
        in("rdi") slot as u64,inlateout("rsi") task_id as u64=>_,
        lateout("rdx") _,lateout("r10") _,lateout("r8") _,out("rcx") _,out("r11") _,options(nostack)); }
    if status == OK { Ok(()) } else { Err(status) }
}
#[cfg(target_arch = "x86_64")]
pub fn retire(slot: usize) -> Result<RetiredPool,u64> {
    let (status,base,length,task): (u64,u64,u64,u64);
    unsafe { core::arch::asm!("syscall",inlateout("rax") SYS_RETIRE=>status,
        in("rdi") slot as u64,lateout("rsi") base,lateout("rdx") length,lateout("r10") task,
        lateout("r8") _,out("rcx") _,out("r11") _,options(nostack)); }
    if status != OK { return Err(status); }
    Ok(RetiredPool { base,length,task_id:u32::try_from(task).map_err(|_|INVALID)? })
}

/// A service applies retirement once, then retries only its own acknowledgement.
/// The terminal identity remains fenced even when ACK delivery is delayed.
pub trait Authority {
    fn catalog(&mut self, slot: usize) -> Result<CatalogInfo,u64>;
    fn info(&mut self, slot: usize) -> Result<LifecycleInfo,u64>;
    fn ack(&mut self, slot: usize, task: u32) -> Result<(),u64>;
}
#[cfg(target_arch="x86_64")]
pub struct KernelAuthority;
#[cfg(target_arch="x86_64")]
impl Authority for KernelAuthority {
    fn catalog(&mut self,s:usize)->Result<CatalogInfo,u64>{catalog(s)}
    fn info(&mut self,s:usize)->Result<LifecycleInfo,u64>{info(s)}
    fn ack(&mut self,s:usize,t:u32)->Result<(),u64>{ack(s,t)}
}
#[derive(Clone,Copy)]
pub struct ServiceCustody { retired: [Option<ExitNotice>;SLOTS.len()], acked:[bool;SLOTS.len()] }
impl ServiceCustody {
    pub const fn new()->Self { Self {retired:[None;SLOTS.len()],acked:[false;SLOTS.len()]} }
    pub fn is_retired(&self,target:u32)->bool {
        self.retired.iter().flatten().any(|n|n.target_pd==target)
    }
    /// Returns Some only for an attested terminal notice (including replay).
    /// No effects occur for a forged hint, live task, or mismatching grant.
    pub fn receive<A:Authority,F:FnMut(ExitNotice)>(&mut self,a:&mut A,
        opcode:u64,caller:u64,args:[u64;3],mut cleanup:F)->Option<(ExitNotice,bool)> {
        let notice=ExitNotice::decode(opcode,caller,args).ok()?;
        for (index,slot) in SLOTS.into_iter().enumerate() {
            let Ok(c)=a.catalog(slot) else {continue};
            if c.target_pd!=notice.target_pd || c.task_id!=notice.task_id {continue}
            let Ok(i)=a.info(slot) else {return None};
            if i.target_pd!=notice.target_pd || i.task_id!=notice.task_id
                || i.required_mask!=REQUIRED_SERVICES {return None}
            if self.retired[index].is_none() {
                self.retired[index]=Some(notice); // fence before releasing service state
                cleanup(notice);
            } else if self.retired[index]!=Some(notice) {return None}
            if self.acked[index] {return None}
            let acked=a.ack(slot,notice.task_id).is_ok();
            self.acked[index]=acked;
            return Some((notice,acked));
        }
        None
    }
}

/// Immutable BOOT grants for the finite second Quil incarnation.
pub const ACTIVATION_SLOT:usize=44;
pub const SUCCESSOR_CLOSE_SLOT:u64=46;
pub const SUCCESSOR_ROUTE_SLOT:u64=47;
pub const SELF_CATALOG_SLOT:usize=48;
/// Third Quil: separate BOOT image/layout and exact predecessor-bound grant.
pub const QUIL_THIRD_LIFECYCLE_SLOT:usize=54;
pub const QUIL_THIRD_ACTIVATION_SLOT:usize=55;
pub const QUIL_THIRD_CLOSE_SLOT:u64=56;
pub const QUIL_THIRD_ROUTE_SLOT:u64=57;
pub const SPINDLE_ACTIVATION_SLOT:usize=49;
pub const SPINDLE_LIFECYCLE_SLOT:usize=50;
pub const SPINDLE_CLOSE_SLOT:u64=51;
pub const SPINDLE_ROUTE_SLOT:u64=52;
pub const SYS_ACTIVATION:u64=66;
pub const ACTIVATION_RESERVED:u64=0;
pub const ACTIVATION_READY:u64=1;
pub const ACTIVATION_ACTIVE:u64=2;
pub const ACTIVATION_EXHAUSTED:u64=3;
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct ActivationInfo{pub successor:u32,pub task:u32,pub surface:u64,pub state:u64}
/// kind0 observes; kind1 consumes the fixed one-shot grant or replays its
/// receipt. No caller supplies a domain, image, address or replacement grant.
#[cfg(target_arch="x86_64")]
pub fn activation(slot:usize,commit:bool)->Result<ActivationInfo,u64>{
    let (status,pd,task,surface,state):(u64,u64,u64,u64,u64);
    unsafe{core::arch::asm!("syscall",inlateout("rax") SYS_ACTIVATION=>status,
        in("rdi") slot as u64,inlateout("rsi") commit as u64=>pd,
        lateout("rdx") task,lateout("r10") surface,lateout("r8") state,
        out("rcx") _,out("r11") _,options(nostack));}
    if status!=OK{return Err(status)}
    Ok(ActivationInfo{successor:u32::try_from(pd).map_err(|_|INVALID)?,
        task:u32::try_from(task).map_err(|_|INVALID)?,surface,state})
}
