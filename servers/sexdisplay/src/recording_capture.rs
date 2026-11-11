// SPDX-License-Identifier: GPL-3.0-only
//! Display-owned recording custody. Wire identities never replace held grants.
use sex_pdx::{desktop_recording as r, desktop_recording_capture as c};
const SHELL_ROUTE: u64 = 6;
#[derive(Clone,Copy,PartialEq,Eq)]
enum Phase { Indicator, Armed, Waiting, Held, Closed }
#[derive(Clone,Copy)]
struct Session {
    scope:c::Scope, arm:r::RoleSequence, phase:Phase, began:u64,
    last_publication:Option<u64>, acquire:Option<r::RoleSequence>, last_app:u32,
    generation:u64, delivered:bool, ack:Option<c::FrameAck>, stop:Option<r::RoleSequence>,
}
#[derive(Clone,Copy)]
struct Reply { scope:c::Scope, sequence:r::RoleSequence, status:c::FrameStatus }
#[derive(Clone,Copy)]
struct ShellReply { scope:c::Scope, sequence:r::RoleSequence, status:c::ControlStatus }
pub struct Server {
    epoch:u64, bank:Option<c::Bank>, session:Option<Session>,
    shell_reply:Option<ShellReply>, app_reply:Option<Reply>, event:Option<c::ClosedEvent>,
    dirty:bool,
}
impl Server {
    pub const fn new()->Self { Self {epoch:0,bank:None,session:None,shell_reply:None,app_reply:None,event:None,dirty:false} }
    pub fn attach(&mut self,epoch:u64) {
        self.epoch=epoch;
        if !r::ENABLED || epoch==0 || epoch>u32::MAX as u64 {return;}
        let rights=sex_pdx::memory_region::QUERY_VIEW|sex_pdx::memory_region::READ|sex_pdx::memory_region::WRITE;
        self.bank=sex_pdx::memory_region_info(c::SLOT).ok().and_then(|info|
            sex_pdx::memory_region_view(c::SLOT,0,c::BYTES as u64,rights).ok().and_then(|view|
                unsafe{c::Bank::from_resolved(info,view,true)}));
        if let Some(bank)=self.bank.as_mut() {
            // A failed epoch installation still revokes any old visible token.
            bank.scrub();if !bank.install_epoch(epoch){self.bank=None;}
        }
    }
    fn shell_live(scope:c::Scope)->bool {
        sex_pdx::crash_log::peer_state(SHELL_ROUTE).is_ok_and(|p|p.live()&&p.epoch==scope.shell_epoch)
    }
    fn task_live(target:u32,task:u32)->bool {
        let Some(row)=sex_pdx::app_catalog::by_target(target) else{return false;};
        let matches=||sex_pdx::app_lifecycle::catalog(row.lifecycle_slot).is_ok_and(|c|
            c.target_pd==target&&c.task_id==task&&c.coordinator_pd==r::SHELL);
        matches()&&sex_pdx::app_lifecycle::info(row.lifecycle_slot)==Err(sex_pdx::app_lifecycle::LIVE)&&matches()
    }
    fn live(&self,scope:c::Scope)->bool {
        scope.valid()&&scope.display_epoch==self.epoch&&Self::shell_live(scope)
            &&Self::task_live(r::TARGET,scope.record_task)&&Self::task_live(r::SPINDLE,scope.spindle_task)
            &&Self::shell_live(scope)
    }
    fn scrub(&mut self) {if let Some(bank)=self.bank.as_mut(){bank.scrub();}}
    fn close(&mut self,reason:c::CloseReason) {
        // Fence publication and erase the bank before any IPC/encoder/Files wait.
        let Some(mut s)=self.session else{return;};
        if s.phase==Phase::Closed{return;}
        s.phase=Phase::Closed;s.ack=None;self.session=Some(s);self.scrub();self.dirty=true;
        sex_pdx::serial_record!("[display.recording.revoked] session={} task={} epoch={} reason={} scrubbed=1",s.scope.session.word(),s.scope.record_task,s.scope.display_epoch,reason as u8);
        self.shell_reply=None;self.app_reply=None;
        if let Some(sequence)=s.acquire {self.app_reply=Some(Reply{scope:s.scope,sequence,status:match reason{
            c::CloseReason::Expired=>c::FrameStatus::Expired,_=>c::FrameStatus::Cancelled}});}
        self.event=Some(c::ClosedEvent{reason,sequence:s.arm,scope:s.scope});
    }
    /// Poll before cleanup acknowledgments, each intake, and before publication.
    pub fn reconcile(&mut self,now:u64) {
        if let Some(s)=self.session.filter(|s|s.phase!=Phase::Closed) {
            if !self.live(s.scope){self.close(c::CloseReason::Cancelled);}
            else if now<s.began||now-s.began>=c::DURATION_MS {self.close(c::CloseReason::Expired);}
        }
    }
    pub fn mark(&self)->Option<[u64;4]> {
        self.session.filter(|s|s.phase!=Phase::Closed).map(|s|Self::scope_mark(s.scope))
    }
    fn scope_mark(s:c::Scope)->[u64;4] {
        [s.session.word()as u64,s.display_epoch,s.shell_epoch,(s.record_task as u64)<<32|s.spindle_task as u64]
    }
    pub fn placement(&mut self,valid:bool) {
        if !valid{self.close(c::CloseReason::Cancelled);}
    }
    pub fn needs_publication(&mut self,now:u64)->bool {
        self.reconcile(now);
        self.dirty||self.session.is_some_and(|s|s.phase==Phase::Indicator||s.phase==Phase::Waiting
            &&s.last_publication.is_none_or(|last|now>=last&&now-last>=c::INTERVAL_MS))
    }
    fn control_reply(&mut self,scope:c::Scope,sequence:r::RoleSequence,status:c::ControlStatus) {
        self.shell_reply=Some(ShellReply{scope,sequence,status});
    }
    fn deny(msg:sex_pdx::PdxMessage,frame:bool) {
        let nonce=(msg.arg0>>32)&0xff_ffff;
        let task=if !frame&&msg.caller_pd==r::SHELL{msg.arg1 as u32}else{msg.caller_task};
        let value=if frame{c::FrameReply::denied(nonce,msg.caller_task).and_then(|r|r.word())}
            else{c::ControlReply::denied(nonce,task).and_then(|r|r.word())};
        if let Some(value)=value{let _=sex_pdx::pdx_reply(msg.caller_pd,value);}
    }
    pub fn control(&mut self,msg:sex_pdx::PdxMessage,now:u64,placement:bool) {
        self.reconcile(now);
        let Some(req)=c::ControlRequest::decode(msg.caller_pd,[msg.arg0,msg.arg1,msg.arg2])else{Self::deny(msg,msg.caller_pd==r::TARGET);return;};
        let caller_ok=match req.kind{c::ControlKind::Arm|c::ControlKind::Stop=>msg.caller_task==r::SHELL,
            c::ControlKind::Acquire=>msg.caller_task==req.scope.record_task};
        let scope_live=if req.kind==c::ControlKind::Stop {
            req.scope.valid()&&req.scope.display_epoch==self.epoch&&Self::shell_live(req.scope)
        }else{self.live(req.scope)};
        if !caller_ok||!scope_live{Self::deny(msg,req.kind==c::ControlKind::Acquire);return;}
        match req.kind {
            c::ControlKind::Arm=>{
                if !r::ENABLED||self.bank.is_none()||!placement {
                    self.control_reply(req.scope,req.sequence,c::ControlStatus::Unavailable);return;
                }
                if let Some(s)=self.session {
                    if s.scope==req.scope&&s.arm==req.sequence&&s.phase!=Phase::Closed {
                        if s.phase!=Phase::Indicator {self.control_reply(req.scope,req.sequence,c::ControlStatus::Armed);}return;
                    }
                    // Closed sessions are tombstones: never resurrect a queued Arm.
                    if s.phase!=Phase::Closed||s.scope.shell_epoch==req.scope.shell_epoch
                        &&req.scope.session.word()<=s.scope.session.word() {Self::deny(msg,false);return;}
                }
                if self.shell_reply.is_some()||self.app_reply.is_some()||self.event.is_some(){Self::deny(msg,false);return;}
                self.scrub();self.session=Some(Session{scope:req.scope,arm:req.sequence,phase:Phase::Indicator,
                    began:now,last_publication:None,acquire:None,last_app:0,generation:0,delivered:false,ack:None,stop:None});
                self.dirty=true;
            }
            c::ControlKind::Stop=>{
                if self.session.is_none_or(|s|s.scope!=req.scope) {
                    // Trusted Stop may overtake an unprocessed Arm. Retain a
                    // closed high-water mark, never create capture authority.
                    if !self.live(req.scope)||self.session.is_some_and(|s|s.phase!=Phase::Closed
                        ||s.scope.shell_epoch==req.scope.shell_epoch&&s.scope.session.word()>=req.scope.session.word()) {
                        Self::deny(msg,false);return;
                    }
                    self.scrub();self.session=Some(Session{scope:req.scope,arm:req.sequence,phase:Phase::Closed,
                        began:now,last_publication:None,acquire:None,last_app:0,generation:0,delivered:false,ack:None,stop:None});
                }
                let s=self.session.unwrap();
                if s.stop.is_some_and(|seq|seq!=req.sequence){Self::deny(msg,false);return;}
                self.close(c::CloseReason::Cancelled);
                if let Some(s)=self.session.as_mut(){s.stop=Some(req.sequence);}
                self.control_reply(req.scope,req.sequence,c::ControlStatus::Stopped);
            }
            c::ControlKind::Acquire=>{
                let Some(mut s)=self.session.filter(|s|s.scope==req.scope&&s.phase!=Phase::Closed)else{Self::deny(msg,true);return;};
                if req.sequence.word()<=s.last_app {Self::deny(msg,true);return;}
                if s.phase!=Phase::Armed||self.app_reply.is_some(){
                    if let Some(word)=c::FrameReply::new(req.sequence,msg.caller_task,c::FrameStatus::Busy).and_then(|r|r.word()){
                        let _=sex_pdx::pdx_reply(msg.caller_pd,word);
                    }return;
                }
                s.acquire=Some(req.sequence);s.last_app=req.sequence.word();s.phase=Phase::Waiting;s.delivered=false;s.ack=None;
                self.session=Some(s);
            }
        }
    }
    pub fn frame(&mut self,msg:sex_pdx::PdxMessage,now:u64) {
        self.reconcile(now);
        let Some(req)=c::FrameAck::decode(msg.caller_pd,[msg.arg0,msg.arg1,msg.arg2])else{Self::deny(msg,true);return;};
        let Some(mut s)=self.session.filter(|s|s.scope.record_task==msg.caller_task
            &&s.scope.session==req.session&&s.scope.display_epoch==req.display_epoch)else{Self::deny(msg,true);return;};
        if !self.live(s.scope){Self::deny(msg,true);return;}
        if req.kind==c::AckKind::Frame&&s.phase==Phase::Closed{Self::deny(msg,true);return;}
        if s.ack==Some(req)&&s.last_app==req.sequence.word()
            &&self.app_reply.is_none_or(|r|r.scope==s.scope&&r.sequence==req.sequence) {
            self.app_reply=Some(Reply{scope:s.scope,sequence:req.sequence,status:if req.kind==c::AckKind::Release{c::FrameStatus::Released}else{c::FrameStatus::Acked}});return;
        }
        if req.sequence.word()<=s.last_app{Self::deny(msg,true);return;}
        match req.kind {
            c::AckKind::Release=>{
                self.close(c::CloseReason::Released);
                s=self.session.unwrap();s.ack=Some(req);s.last_app=req.sequence.word();self.session=Some(s);
                // Cleanup response supersedes any old Ready/Cancelled response.
                self.app_reply=Some(Reply{scope:s.scope,sequence:req.sequence,status:c::FrameStatus::Released});
            }
            c::AckKind::Frame=>{
                if s.phase!=Phase::Held||!s.delivered||s.generation!=req.generation||self.app_reply.is_some(){Self::deny(msg,true);return;}
                self.scrub();s.phase=Phase::Armed;s.acquire=None;s.ack=Some(req);s.last_app=req.sequence.word();self.session=Some(s);
                self.app_reply=Some(Reply{scope:s.scope,sequence:req.sequence,status:c::FrameStatus::Acked});
            }
        }
    }
    /// Exact immutable Scene mark plus inspected final front pixels, after CPU
    /// publication. No reference into the front survives this call.
    pub fn published(&mut self,mark:Option<[u64;4]>,indicator_pixels:bool,generation:u64,
        w:usize,h:usize,pixels:&[u32],night:u8,now:u64) {
        self.reconcile(now);
        if mark.is_none(){
            if self.session.is_some_and(|s|s.phase!=Phase::Closed){self.close(c::CloseReason::Cancelled);}else{self.dirty=false;}
            return;
        }
        let Some(mut s)=self.session.filter(|s|s.phase!=Phase::Closed)else{return;};
        if mark!=Some(Self::scope_mark(s.scope)){return;}
        if !indicator_pixels{self.close(c::CloseReason::Cancelled);return;}
        self.dirty=false;
        if s.phase==Phase::Indicator {
            s.phase=Phase::Armed;self.session=Some(s);
            sex_pdx::serial_record!("[display.recording.armed] session={} task={} epoch={} generation={} indicator=cpu_pixels",s.scope.session.word(),s.scope.record_task,s.scope.display_epoch,generation);
            self.control_reply(s.scope,s.arm,c::ControlStatus::Armed);return;
        }
        if s.phase!=Phase::Waiting||s.last_publication.is_some_and(|last|now<last||now-last<c::INTERVAL_MS){return;}
        let Some(sequence)=s.acquire else{return;};
        let Some(header)=c::Header::new(s.scope,sequence,generation,now,w,h,night)else{self.close(c::CloseReason::Cancelled);return;};
        if !self.bank.as_mut().is_some_and(|b|b.publish(header,pixels)){self.close(c::CloseReason::Cancelled);return;}
        // Held epochs/lifecycle can change while copying. Revalidate before Ready.
        if !self.live(s.scope){self.close(c::CloseReason::Cancelled);return;}
        s.phase=Phase::Held;s.generation=generation;s.last_publication=Some(now);s.delivered=false;self.session=Some(s);
        self.app_reply=Some(Reply{scope:s.scope,sequence,status:c::FrameStatus::Ready});
        sex_pdx::serial_record!("[display.recording.frame] session={} task={} generation={} publication_ms={} native_w={} native_h={} width=640 height=360 night={}",s.scope.session.word(),s.scope.record_task,generation,now,w,h,night);
    }
    pub fn poll(&mut self,now:u64) {
        self.reconcile(now);
        if let Some(reply)=self.shell_reply {
            if !Self::shell_live(reply.scope){self.shell_reply=None;}
            else if reply.status==c::ControlStatus::Armed&&!self.live(reply.scope){self.close(c::CloseReason::Cancelled);}
            else if let Some(word)=c::ControlReply::new(reply.sequence,reply.scope.record_task,reply.status).and_then(|r|r.word()){
                if sex_pdx::pdx_reply(r::SHELL,word)==0{self.shell_reply=None;}
            }
        }
        if let Some(reply)=self.app_reply {
            if !self.live(reply.scope){self.app_reply=None;}
            else if let Some(word)=c::FrameReply::new(reply.sequence,reply.scope.record_task,reply.status).and_then(|r|r.word()){
                if sex_pdx::pdx_reply(r::TARGET,word)==0{
                    self.app_reply=None;
                    if reply.status==c::FrameStatus::Ready {if let Some(s)=self.session.as_mut(){s.delivered=true;}}
                }
            }
        }
        if let Some(event)=self.event {
            if !Self::shell_live(event.scope){self.event=None;}
            else if let Some(a)=event.args(){if sex_pdx::pdx_call(SHELL_ROUTE,c::OP_EVENT,a[0],a[1],a[2]).0==0{self.event=None;}}
        }
    }
}
