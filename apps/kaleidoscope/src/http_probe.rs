// SPDX-License-Identifier: GPL-3.0-only
//! Explicit controlled HTTP intent over the existing owned network-job route.
//! Sequence custody is handed off by the app after the preceding TCP job's
//! positive RELEASE. An accepted timeout never authorizes mutation replay.
use sex_pdx::network_job as w;
use crate::http_wire;
const BLANK:[u8;64]=[b' ';64];
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum View { Idle, Pending, Ready, Unavailable, Cancelled, Uncertain }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum Step { Done, Begin, Write, Start, Poll, Read, Cancel, Release, Held }
pub struct Client {
    step:Step,seq:u32,ticket:u32,pending:Option<w::Request>,accepted:bool,
    deadline:u64,next_poll:u64,cancel:bool,view:View,terminal:View,
    request:[u8;http_wire::MAX_REQUEST],request_len:usize,written:usize,
    header:[u8;http_wire::MAX_HEADERS],header_len:usize,header_done:bool,
    response_len:usize,read:usize,transport_complete:bool,body_hash:u64,
    preview:[u8;64],body_len:usize,
    document:[u8;http_wire::MAX_BODY],document_len:usize,
    generation:u32,
    navigation:bool,path:http_wire::Path,redirect:Option<http_wire::Path>,released_redirect:Option<http_wire::Path>,response_status:u16,
}
impl Client {
    pub const fn new()->Self {
        Self {step:Step::Done,seq:1,ticket:0,pending:None,accepted:false,
            deadline:0,next_poll:0,cancel:false,view:View::Idle,terminal:View::Unavailable,
            request:[0;http_wire::MAX_REQUEST],request_len:0,written:0,
            header:[0;http_wire::MAX_HEADERS],header_len:0,header_done:false,
            response_len:0,read:0,transport_complete:false,body_hash:0xcbf29ce484222325,
            preview:BLANK,body_len:0,document:[0;http_wire::MAX_BODY],document_len:0,generation:0,
            navigation:false,path:http_wire::Path::initial(),redirect:None,released_redirect:None,response_status:200}
    }
    pub fn next_sequence(&self)->u32 { self.seq }
    pub fn busy(&self)->bool { self.step!=Step::Done || self.ticket!=0 }
    pub fn view(&self)->View { self.view }
    pub fn preview(&self)->&[u8;64] { if self.view==View::Ready {&self.preview}else{&BLANK} }
    pub fn body_length(&self)->usize { if self.view==View::Ready {self.body_len}else{0} }
    /// Exact received body bytes become visible only after positive RELEASE.
    pub fn document(&self)->&[u8] { if self.view==View::Ready {&self.document[..self.document_len]}else{&[]} }
    pub fn document_path(&self)->&[u8] {if self.view==View::Ready{self.path.as_bytes()}else{&[]}}
    /// A continuation is data, available only after the preceding job's
    /// positive RELEASE. Main must apply its hop policy and fresh DNS proof.
    pub fn take_redirect(&mut self)->Option<http_wire::Path>{if self.busy(){None}else{self.released_redirect.take()}}
    pub fn reject_redirect(&mut self)->bool{
        if self.busy(){return false;}
        self.scrub_raw();self.clear_preview();self.view=View::Unavailable;self.terminal=View::Unavailable;true
    }
    pub fn start(&mut self,next_seq:u32)->bool {
        self.start_with_generation(next_seq,0)
    }
    pub fn wait_for_resolution(&mut self) {
        if !self.busy(){self.scrub_raw();self.clear_preview();self.view=View::Pending;self.terminal=View::Unavailable;self.generation=0;}
    }
    pub fn start_resolved(&mut self,next_seq:u32,generation:u32)->bool {
        if self.busy(){return false;}
        if generation==0 || generation>w::MAX_TICKET {
            self.scrub_raw();self.clear_preview();self.view=View::Unavailable;self.terminal=View::Unavailable;self.generation=0;return false;
        }
        self.start_with_generation(next_seq,generation)
    }
    pub fn start_resolved_path(&mut self,next_seq:u32,generation:u32,path:http_wire::Path)->bool {
        if !self.start_resolved(next_seq,generation){return false;}
        self.navigation=true;self.path=path;self.request_len=http_wire::request_path(&mut self.request,&path);true
    }
    fn start_with_generation(&mut self,next_seq:u32,generation:u32)->bool {
        if self.busy() { return false; }
        if next_seq<self.seq || next_seq>w::MAX_SEQ-8 { self.scrub_raw();self.clear_preview();self.view=View::Unavailable;self.terminal=View::Unavailable;return false; }
        self.scrub_raw();self.clear_preview();self.seq=next_seq;
        self.generation=generation;self.navigation=false;self.path=http_wire::Path::initial();self.response_status=200;
        self.request_len=http_wire::request(&mut self.request);self.written=0;self.read=0;self.response_len=0;
        self.header_len=0;self.header_done=false;self.transport_complete=false;self.body_hash=0xcbf29ce484222325;
        self.pending=None;self.accepted=false;self.cancel=false;
        self.terminal=View::Unavailable;self.view=View::Pending;self.step=Step::Begin;true
    }
    fn scrub_raw(&mut self) { self.request.fill(0);self.header.fill(0); }
    fn clear_preview(&mut self) { self.preview=BLANK;self.body_len=0;self.document.fill(0);self.document_len=0;self.redirect=None;self.released_redirect=None; }
    fn finished(&mut self) {
        self.scrub_raw();self.pending=None;self.accepted=false;self.step=Step::Done;
        self.request_len=0;self.written=0;self.response_len=0;self.read=0;
        self.header_len=0;self.header_done=false;self.transport_complete=false;self.body_hash=0xcbf29ce484222325;
        self.generation=0;
        self.view=self.terminal;
        if self.view!=View::Ready { self.clear_preview(); }
    }
    pub fn cancel(&mut self) {
        self.cancel=true;self.terminal=View::Cancelled;self.clear_preview();self.scrub_raw();
        if !self.busy() { self.finished();return; }
        if self.step==Step::Begin && !self.accepted {
            if self.pending.take().is_some() { self.seq+=1; }
            self.finished();return;
        }
        if !self.accepted && self.pending.is_some_and(|r|r.kind!=w::RELEASE) {
            let r=self.pending.take().unwrap();self.seq=r.seq+1;
        }
        if self.step==Step::Held && self.ticket!=0 { self.step=Step::Release; }
    }
    fn rejected(&mut self,kind:u8) {
        self.scrub_raw();self.clear_preview();
        self.terminal=if self.cancel {View::Cancelled}else{View::Unavailable};
        self.view=if kind==w::RELEASE && self.ticket!=0 {View::Uncertain}else{View::Unavailable};
        if self.ticket==0 { self.finished(); }
        else { self.step=if kind==w::RELEASE {Step::Held}else{Step::Release}; }
    }
    /// Consume only an exact newly acknowledged absolute-cursor READ turn.
    /// Exact body bytes feed both the receipt hash and retained document.
    /// Display replacement never rewrites this source storage.
    fn consume(&mut self,bytes:&[u8])->Result<(),http_wire::Error> {
        for &byte in bytes {
            if !self.header_done {
                if self.header_len==self.header.len() {return Err(http_wire::Error::HeaderLimit);}
                self.header[self.header_len]=byte;self.header_len+=1;
                if self.header[..self.header_len].ends_with(b"\r\n\r\n") {
                    if self.navigation {
                        let parsed=http_wire::parse_navigation_headers(&self.header[..self.header_len])?;
                        self.body_len=parsed.content_length;self.redirect=parsed.redirect;self.response_status=parsed.status;
                    }else{
                        let parsed=http_wire::parse_headers(&self.header[..self.header_len])?;
                        self.body_len=parsed.content_length;
                    }
                    self.header_done=true;
                }
            }else{
                if self.document_len>=self.body_len {return Err(http_wire::Error::TrailingData);}
                self.body_hash=(self.body_hash^byte as u64).wrapping_mul(0x100000001b3);
                self.document[self.document_len]=byte;
                self.document_len+=1;
            }
        }
        Ok(())
    }
    fn validate_complete(&mut self) {
        if !self.transport_complete || self.read!=self.response_len || !self.header_done
            || self.document_len!=self.body_len || self.read!=self.header_len+self.body_len {
            self.rejected(w::READ);return;
        }
        if let Some(path)=self.redirect{
            // Redirect bodies are finite framing data, never document content.
            self.clear_preview();self.redirect=Some(path);self.terminal=View::Pending;self.step=Step::Release;return;
        }
        self.preview=normalize(&self.document[..self.document_len]);
        sex_pdx::serial_record!("[browser.http.body] ticket={} status=200 length={} hash={:#x} controlled_peer=1 tls=0",self.ticket,self.body_len,self.body_hash);
        self.terminal=View::Ready;self.step=Step::Release;
    }
    pub fn reply(&mut self,word:u64,now:u64) {
        let Some(r)=self.pending else{return;};
        if !self.accepted || !matches!(word>>56,0x78|0xE8)
            || ((word>>32)&w::MAX_SEQ as u64)!=r.seq as u64 { return; }
        self.pending=None;self.accepted=false;self.seq=r.seq+1;
        let value=match w::decode_reply(word,r.seq) {
            Ok(value)=>value,
            Err(w::BUSY) if r.kind==w::READ && self.generation!=0 && !self.cancel=>{
                self.step=Step::Poll;self.next_poll=now.saturating_add(2);return;
            }
            Err(_)=>{self.rejected(r.kind);return;},
        };
        // Late data receipts are reconciled by cleanup, never used to republish
        // cancelled bytes or restart a request whose authority is uncertain.
        if self.cancel && !matches!(r.kind,w::BEGIN|w::BEGIN_RESOLVED|w::BEGIN_STREAM_RESOLVED|w::CANCEL|w::RELEASE) {
            self.step=Step::Cancel;return;
        }
        match r.kind {
            w::BEGIN|w::BEGIN_RESOLVED|w::BEGIN_STREAM_RESOLVED=>{
                if value==0 || value>w::MAX_TICKET {
                    self.scrub_raw();self.clear_preview();self.view=View::Uncertain;self.step=Step::Held;return;
                }
                self.ticket=value;self.step=Step::Write;
            }
            w::WRITE=>{
                let end=r.offset as usize+r.length as usize;
                if value as usize!=end || end>self.request_len {self.rejected(r.kind);return;}
                self.written=end;self.step=if end==self.request_len {Step::Start}else{Step::Write};
            }
            w::START=>{
                if value!=0 {self.rejected(r.kind);return;}
                self.step=Step::Poll;self.next_poll=now;
            }
            w::STATUS=>match w::decode_status(value) {
                Some((state @ (w::State::Receiving|w::State::Complete),len)) if self.generation!=0 || state==w::State::Complete=>{
                    let limit=if self.generation!=0 {http_wire::MAX_RESPONSE}else{w::MAX_BYTES};
                    if len>limit || len<self.response_len || len<self.read
                        || (self.transport_complete && state!=w::State::Complete) {
                        self.rejected(r.kind);return;
                    }
                    self.response_len=len;self.transport_complete=state==w::State::Complete;
                    if self.read<len {self.step=Step::Read;}
                    else if self.transport_complete {self.validate_complete();}
                    else {self.next_poll=now.saturating_add(25);self.step=Step::Poll;}
                }
                Some((w::State::Staging|w::State::Connecting|w::State::Sending|w::State::Receiving,_)) if !self.transport_complete && self.read==0=>{
                    self.next_poll=now.saturating_add(25);self.step=Step::Poll;
                }
                _=>{self.rejected(r.kind);return;}
            },
            w::READ=>{
                let end=self.read+r.length as usize;
                if r.offset as usize!=self.read || end>self.response_len {self.rejected(r.kind);return;}
                if self.consume(&value.to_le_bytes()[..r.length as usize]).is_err(){self.rejected(r.kind);return;}
                self.read=end;
                if end==self.response_len {
                    if self.transport_complete {self.validate_complete();}
                    else {self.next_poll=now;self.step=Step::Poll;}
                }
            }
            w::CANCEL=>{
                if value!=0 {self.rejected(r.kind);return;}
                self.scrub_raw();self.clear_preview();self.step=Step::Release;
            }
            w::RELEASE=>{
                if value!=0 {self.rejected(r.kind);return;}
                let follow=if !self.cancel&&self.terminal==View::Pending{self.redirect}else{None};
                let status=self.response_status;
                self.ticket=0;self.finished();
                self.released_redirect=follow;
                sex_pdx::serial_record!("[browser.http.released] ticket={} view={:?} private_job=0",r.ticket,self.view);
                if self.view==View::Ready{sex_pdx::serial_record!("[browser.http.document] ticket={} path={} status=200 private_bytes={} positive_release=1",r.ticket,core::str::from_utf8(self.path.as_bytes()).unwrap(),self.document_len);}
                if let Some(path)=follow{sex_pdx::serial_record!("[browser.http.redirect.released] ticket={} status={} path={} positive_release=1 document_bytes=0",r.ticket,status,core::str::from_utf8(path.as_bytes()).unwrap());}
                return;
            }
            _=>{self.rejected(r.kind);return;}
        }
        if self.cancel && self.ticket!=0 && !matches!(self.step,Step::Release|Step::Held) {self.step=Step::Cancel;}
    }
    pub fn advance(&mut self,now:u64) {
        if !self.busy() || self.step==Step::Held {return;}
        if self.pending.is_some() && now>=self.deadline {
            if self.accepted {
                self.cancel=true;self.terminal=View::Cancelled;self.view=View::Uncertain;
                self.scrub_raw();self.clear_preview();self.deadline=u64::MAX;return;
            }
            let kind=self.pending.take().unwrap().kind;self.seq+=1;self.accepted=false;
            self.rejected(kind);
            if !self.busy() || self.step==Step::Held {return;}
        }
        if self.pending.is_none() {
            if self.cancel && self.ticket!=0 && self.step!=Step::Release {self.step=Step::Cancel;}
            if self.step==Step::Poll && now<self.next_poll {return;}
            if self.seq>=w::MAX_SEQ-1 && !matches!(self.step,Step::Cancel|Step::Release) {
                self.scrub_raw();self.clear_preview();self.terminal=View::Unavailable;self.view=View::Unavailable;self.step=Step::Cancel;
            }
            if self.seq>w::MAX_SEQ {self.scrub_raw();self.clear_preview();self.view=View::Uncertain;self.step=Step::Held;return;}
            let(kind,offset,length,a)=match self.step {
                Step::Begin=>if self.generation!=0{(w::BEGIN_STREAM_RESOLVED,0,self.request_len,self.generation as u64)}else{(w::BEGIN,0,self.request_len,0)},
                Step::Write=>{
                    let length=(self.request_len-self.written).min(8);let mut bytes=[0;8];
                    bytes[..length].copy_from_slice(&self.request[self.written..self.written+length]);
                    (w::WRITE,self.written,length,u64::from_le_bytes(bytes))
                }
                Step::Start=>(w::START,0,0,0),Step::Poll=>(w::STATUS,0,0,0),
                Step::Read=>(w::READ,self.read,(self.response_len-self.read).min(4),0),
                Step::Cancel=>(w::CANCEL,0,0,0),Step::Release=>(w::RELEASE,0,0,0),Step::Done|Step::Held=>return,
            };
            self.pending=Some(w::Request {seq:self.seq,kind,offset:offset as u16,length:length as u16,
                ticket:if matches!(kind,w::BEGIN|w::BEGIN_RESOLVED|w::BEGIN_STREAM_RESOLVED) {0}else{self.ticket},a});
            self.deadline=now.saturating_add(2000);self.accepted=false;
        }
        if !self.accepted {
            let r=self.pending.unwrap();let args=r.encode();
            let(status,_)=sex_pdx::pdx_call(sex_pdx::SLOT_NET,w::OP,args[0],args[1],args[2]);
            match status {
                0=>self.accepted=true,sex_pdx::ERR_SERVICE_NOT_READY=>{},
                _=>{self.pending=None;self.seq=r.seq+1;self.rejected(r.kind);}
            }
        }
    }
    #[cfg(test)] pub fn raw_scrubbed(&self)->bool {
        self.request.iter().all(|b|*b==0) && self.header.iter().all(|b|*b==0)
    }
    #[cfg(test)] pub fn document_scrubbed(&self)->bool {
        self.document_len==0 && self.document.iter().all(|b|*b==0)
    }
}

fn normalize(body:&[u8])->[u8;64] {
    let mut out=BLANK;let mut length=0;let mut space=false;let mut truncated=false;
    for &byte in body {
        let byte=match byte {0..=32|127=>b' ',33..=126=>byte,_=>b'?'};
        if byte==b' ' {space=length!=0;continue;}
        if space {
            if length<out.len() {out[length]=b' ';length+=1;}else{truncated=true;}
            space=false;
        }
        if length<out.len() {out[length]=byte;length+=1;}else{truncated=true;}
    }
    if truncated {out[61..].copy_from_slice(b"...");}
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{vec,vec::Vec,format};
    fn message(body:&[u8])->Vec<u8> {
        let mut bytes=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",body.len()).into_bytes();bytes.extend_from_slice(body);bytes
    }
    fn client()->Client {
        let mut client=Client::new();client.ticket=1;client.generation=1;client.view=View::Pending;client.step=Step::Poll;client
    }
    fn receipt(p:&mut Client,kind:u8,offset:usize,length:usize,value:u32) {
        let r=w::Request {seq:p.seq,kind,offset:offset as u16,length:length as u16,ticket:p.ticket,a:0};
        p.pending=Some(r);p.accepted=true;p.reply(w::reply(r.seq,value),10);
    }
    fn drain(p:&mut Client,bytes:&[u8],complete:bool) {
        receipt(p,w::STATUS,0,0,w::status(if complete{w::State::Complete}else{w::State::Receiving},bytes.len()));
        while p.read<bytes.len() {
            let at=p.read;let length=(bytes.len()-at).min(4);let mut turn=[0;4];turn[..length].copy_from_slice(&bytes[at..at+length]);
            receipt(p,w::READ,at,length,u32::from_le_bytes(turn));assert!(p.document().is_empty());assert_eq!(p.view(),View::Pending);
        }
    }
    #[test]fn full_stream_is_incremental_hidden_until_complete_then_exact_positive_release(){
        let body:Vec<u8>=(0..http_wire::MAX_BODY).map(|n|(n%256)as u8).collect();let bytes=message(&body);let mut p=client();
        drain(&mut p,&bytes,false);assert_eq!(p.step,Step::Poll);assert_eq!(p.document_len,http_wire::MAX_BODY);assert!(p.document().is_empty());
        let hash=body.iter().fold(0xcbf29ce484222325u64,|hash,&b|(hash^b as u64).wrapping_mul(0x100000001b3));assert_eq!(p.body_hash,hash);
        receipt(&mut p,w::STATUS,0,0,w::status(w::State::Complete,bytes.len()));assert_eq!(p.step,Step::Release);assert!(p.document().is_empty());
        receipt(&mut p,w::RELEASE,0,0,0);assert_eq!(p.view(),View::Ready);assert!(p.raw_scrubbed());assert_eq!(p.body_length(),http_wire::MAX_BODY);
        assert_eq!(p.document(),body);
        assert!(core::mem::size_of::<Client>()<15*1024,"bounded header plus exact body, no second response");
    }
    #[test]fn every_header_body_turn_alignment_uses_one_parse_and_retains_exact_tail(){
        for pad in 0..4 {
            let body=b"one\nfull tail Z";let mut bytes=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nX-Pad: {}\r\n\r\n",body.len(),"x".repeat(pad)).into_bytes();bytes.extend_from_slice(body);
            let mut p=client();drain(&mut p,&bytes,true);assert_eq!(p.step,Step::Release);receipt(&mut p,w::RELEASE,0,0,0);assert_eq!(p.document(),body);
        }
    }
    #[test]fn complete_short_or_trailing_wire_bytes_never_publish_and_scrub_every_private_byte(){
        let exact=message(b"body");
        for bytes in [&exact[..exact.len()-1],&[exact.as_slice(),b"extra"].concat()] {
            let mut p=client();receipt(&mut p,w::STATUS,0,0,w::status(w::State::Complete,bytes.len()));
            while p.step==Step::Read {
                let at=p.read;let length=(bytes.len()-at).min(4);let mut turn=[0;4];turn[..length].copy_from_slice(&bytes[at..at+length]);receipt(&mut p,w::READ,at,length,u32::from_le_bytes(turn));
            }
            assert_eq!(p.view(),View::Unavailable);assert!(p.document().is_empty());assert!(p.document_scrubbed());assert!(p.raw_scrubbed());assert_eq!(p.step,Step::Release);
        }
    }
    #[test]fn stream_failure_or_regressed_cumulative_count_scrubs_partially_consumed_data(){
        let bytes=message(b"partial body");
        for value in [w::status(w::State::Failed,0),w::status(w::State::Receiving,bytes.len()-1)] {
            let mut p=client();drain(&mut p,&bytes,false);receipt(&mut p,w::STATUS,0,0,value);
            assert_eq!(p.view(),View::Unavailable);assert!(p.document_scrubbed());assert!(p.raw_scrubbed());assert_eq!(p.step,Step::Release);
        }
    }
    #[test]fn busy_read_consumes_no_private_bytes_and_resumes_by_fresh_status(){
        let bytes=message(b"body");let mut p=client();receipt(&mut p,w::STATUS,0,0,w::status(w::State::Receiving,bytes.len()));
        let r=w::Request {seq:p.seq,kind:w::READ,offset:0,length:4,ticket:1,a:0};p.pending=Some(r);p.accepted=true;p.reply(w::error(r.seq,w::BUSY),10);
        assert_eq!(p.read,0);assert_eq!(p.header_len,0);assert_eq!(p.step,Step::Poll);assert_eq!(p.seq,r.seq+1);assert!(p.document_scrubbed());
        drain(&mut p,&bytes,true);receipt(&mut p,w::RELEASE,0,0,0);assert_eq!(p.document(),b"body");
    }
    #[test]fn cancel_and_late_accepted_stream_read_cannot_retain_or_republish_partial_body(){
        let bytes=message(&vec![b'S';8193]);let mut p=client();drain(&mut p,&bytes,false);assert_eq!(p.document_len,8193);
        let r=w::Request {seq:p.seq,kind:w::STATUS,offset:0,length:0,ticket:1,a:0};p.pending=Some(r);p.accepted=true;p.cancel();assert!(p.document_scrubbed());assert!(p.raw_scrubbed());
        p.reply(w::reply(r.seq,w::status(w::State::Complete,bytes.len())),11);assert_eq!(p.step,Step::Cancel);assert!(p.document_scrubbed());
        receipt(&mut p,w::CANCEL,0,0,0);receipt(&mut p,w::RELEASE,0,0,0);assert_eq!(p.view(),View::Cancelled);assert!(p.document().is_empty());
    }
}
