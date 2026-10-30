// SPDX-License-Identifier: GPL-3.0-only
//! Bounded observation of one already submitted NVMe command. This helper
//! neither submits, consumes a CQE, reclaims DMA nor receives another request.
pub const POLL_QUANTUM:u32=64;
pub const MAX_TICKS:u64=1000;
pub const MAX_TURNS:u32=4096;
pub const MAX_POLLS:u32=POLL_QUANTUM*MAX_TURNS;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Failure{Deadline,ClockRollback,ClockOverflow,TurnLimit,WaitDenied(u64)}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Completion{pub dw2:u32,pub dw3:u32}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Report{pub result:Result<Completion,Failure>,pub polls:u32,pub turns:u32,pub elapsed:u64,pub last_dw3:u32}
/// The caller retains its submitted DMA guard throughout every pause. A pause
/// is only scheduling cooperation; only exact CID+phase proves completion.
pub fn wait(cid:u16,phase:u32,mut read:impl FnMut()->(u32,u32),mut ticks:impl FnMut()->u64,mut pause:impl FnMut()->Result<(),u64>)->Report{
    let start=ticks();let mut last=start;
    let mut report=Report{result:Err(Failure::TurnLimit),polls:0,turns:0,elapsed:0,last_dw3:0};
    let Some(deadline)=start.checked_add(MAX_TICKS)else{report.result=Err(Failure::ClockOverflow);return report};
    while report.turns<MAX_TURNS{
        if report.turns!=0{
            let now=ticks();
            if now<last{report.result=Err(Failure::ClockRollback);return report}
            report.elapsed=now-start;last=now;
            if now>=deadline{report.result=Err(Failure::Deadline);return report}
        }
        for _ in 0..POLL_QUANTUM{
            let(dw2,dw3)=read();report.polls+=1;report.last_dw3=dw3;
            if dw3&0xffff==cid as u32&&(dw3>>16)&1==phase{
                report.result=Ok(Completion{dw2,dw3});return report;
            }
        }
        report.turns+=1;
        let now=ticks();
        if now<last{report.result=Err(Failure::ClockRollback);return report}
        report.elapsed=now-start;last=now;
        if now>=deadline{report.result=Err(Failure::Deadline);return report}
        if report.turns==MAX_TURNS{return report}
        if let Err(error)=pause(){report.result=Err(Failure::WaitDenied(error));return report}
    }
    report
}
