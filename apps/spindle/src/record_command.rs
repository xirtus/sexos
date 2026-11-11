//! Bounded, allocator-free person intent. Shell owns all recording authority;
//! enqueue success never means capture is armed or that a file was saved.
use crate::Scrollback;
use sex_pdx::desktop_recording::{self as w, Intent, IntentStatus};
#[derive(Clone,Copy,PartialEq,Eq)]
enum Phase {Idle,Begin,Chunks,Commit,Stop,Await}
pub struct Command {
    phase:Phase,
    counter:u32,
    sequence:u32,
    shell_epoch:u64,
    name:[u8;w::NAME_BYTES],
    length:usize,
    offset:usize,
}
pub static mut RECORD:Command=Command::new();
impl Command {
    pub const fn new()->Self{Self{phase:Phase::Idle,counter:0,sequence:0,shell_epoch:0,name:[0;w::NAME_BYTES],length:0,offset:0}}
    fn next(&mut self)->bool{
        let Some(next)=self.counter.checked_add(1).filter(|n|*n<=0xff_ffff)else{return false};
        self.counter=next;self.sequence=next;true
    }
    pub fn start(&mut self,name:&[u8],sb:&mut Scrollback){
        if !w::ENABLED{sb.push(b"Screen recording is unavailable in this build.");return}
        if self.phase!=Phase::Idle{sb.push(b"A recording request is pending. Use record-stop.");return}
        // Use the receiver's exact canonical basename validation as well.
        let mut check=w::Name::new();
        if !check.begin(1,1,name.len()){sb.push(b"Use record NAME.mkv (one basename, up to 200 bytes).");return}
        for (n,chunk) in name.chunks(16).enumerate(){
            let mut bytes=[0u8;16];bytes[..chunk.len()].copy_from_slice(chunk);
            if !check.chunk(1,1,n*16,[u64::from_le_bytes(bytes[..8].try_into().unwrap()),u64::from_le_bytes(bytes[8..].try_into().unwrap())]){return}
        }
        if check.complete(1,1).is_none(){sb.push(b"Use record NAME.mkv without a path or control characters.");return}
        let Ok(peer)=sex_pdx::crash_log::peer_state(sex_pdx::SLOT_SHELL)else{sb.push(b"Shell state is unavailable; recording refused.");return};
        if peer.life!=sex_pdx::crash_log::PeerLife::Live{sb.push(b"Shell is restarting; recording refused.");return}
        if !self.next(){sb.push(b"Recording request IDs exhausted. Reopen Spindle.");return}
        self.shell_epoch=peer.epoch;
        self.name.fill(0);self.name[..name.len()].copy_from_slice(name);self.length=name.len();self.offset=0;self.phase=Phase::Begin;
        sb.push(b"Requested: whole desktop, 640x360, up to 5 fps, video only, maximum 60 seconds.");
        sb.push(b"Destination: Recordings. Capture starts only with the persistent Stop recording indicator.");
        sb.push(b"Use record-stop to revoke capture. Failed or cancelled output may remain as a partial file.");
    }
    pub fn stop(&mut self,args:&[u8],sb:&mut Scrollback){
        if !args.is_empty(){sb.push(b"record-stop takes no arguments.");return}
        if !w::ENABLED{sb.push(b"Screen recording is unavailable in this build.");return}
        if !self.next(){sb.push(b"Recording request IDs exhausted; use the Stop recording indicator.");return}
        // Stop preempts any unsent name/Commit. It needs no Files buffer.
        self.shell_epoch=0;self.name.fill(0);self.length=0;self.offset=0;self.phase=Phase::Stop;
        sb.push(b"Stop recording requested; waiting for trusted capture revocation.");
    }
    pub fn sending(&self)->bool{matches!(self.phase,Phase::Begin|Phase::Chunks|Phase::Commit|Phase::Stop)}
    pub fn poll(&mut self){
        if self.phase==Phase::Stop && self.shell_epoch==0 {
            let Ok(peer)=sex_pdx::crash_log::peer_state(sex_pdx::SLOT_SHELL)else{return};
            if peer.life!=sex_pdx::crash_log::PeerLife::Live{return}
            self.shell_epoch=peer.epoch;
        }
        let intent=match self.phase {
            Phase::Begin=>Intent::Begin{sequence:self.sequence,length:self.length as u16,shell_epoch:self.shell_epoch},
            Phase::Chunks=>{
                let n=(self.length-self.offset).min(16);let mut bytes=[0u8;16];bytes[..n].copy_from_slice(&self.name[self.offset..self.offset+n]);
                Intent::Chunk{sequence:self.sequence,offset:self.offset as u8,words:[u64::from_le_bytes(bytes[..8].try_into().unwrap()),u64::from_le_bytes(bytes[8..].try_into().unwrap())]}
            },
            Phase::Commit=>Intent::Commit{sequence:self.sequence},
            Phase::Stop=>Intent::Stop{sequence:self.sequence},
            _=>return,
        };
        let a=intent.args();
        if !matches!(sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL,w::OP_SPINDLE,a[0],a[1],a[2]),Ok(_)){return}
        self.phase=match self.phase {
            Phase::Begin=>Phase::Chunks,
            Phase::Chunks=>{self.offset=(self.offset+16).min(self.length);if self.offset==self.length{Phase::Commit}else{Phase::Chunks}},
            Phase::Commit|Phase::Stop=>Phase::Await,
            other=>other,
        };
    }
    pub fn receive(&mut self,caller:u32,word:u64,sb:&mut Scrollback)->bool{
        let Some((sequence,status))=w::decode_intent_reply(caller,word)else{return false};
        if sequence!=self.sequence||self.phase==Phase::Idle{return true}
        let Ok(peer)=sex_pdx::crash_log::peer_state(sex_pdx::SLOT_SHELL)else{return true};
        if peer.life!=sex_pdx::crash_log::PeerLife::Live||peer.epoch!=self.shell_epoch{return true}
        match status {
            IntentStatus::Starting=>sb.push(b"Preparing recorder and destination; capture is not active yet."),
            IntentStatus::Active=>sb.push(b"Screen recording active. Use record-stop or the Stop recording indicator."),
            IntentStatus::Stopped=>sb.push(b"Capture revoked. Previously admitted output may still be finalizing."),
            IntentStatus::Saved=>sb.push(b"Recording saved and synced in Recordings."),
            IntentStatus::Denied=>sb.push(b"Recording request denied; capture is not active."),
            IntentStatus::Unavailable=>sb.push(b"Screen recording is unavailable in this build."),
            IntentStatus::Busy=>sb.push(b"Recorder is busy. Use record-stop, then retry after it finishes."),
            IntentStatus::Failed=>sb.push(b"Recording failed. A named partial file may remain in Recordings."),
        }
        if !matches!(status,IntentStatus::Starting|IntentStatus::Active|IntentStatus::Stopped){self.phase=Phase::Idle;self.name.fill(0)}
        true
    }
}
