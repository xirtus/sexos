//! Actual allocator-free command sender, with finite syscall/peer/terminal doubles.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/desktop_recording.rs"] pub mod desktop_recording;
use std::{collections::VecDeque,sync::Mutex};
pub const SLOT_SHELL:u64=6;
pub const ERR_SERVICE_NOT_READY:u64=0x17;
#[derive(Clone,Copy)] pub struct Peer {pub epoch:u64,pub life:crash_log::PeerLife}
mod crash_log {
    #[derive(Clone,Copy,Debug,PartialEq,Eq)] pub enum PeerLife {Live,Ending}
    pub fn peer_state(slot:u64)->Result<super::Peer,u64>{assert_eq!(slot,super::SLOT_SHELL);super::HOST.lock().unwrap().peer.ok_or(1)}
}
struct Host {peer:Option<Peer>,results:VecDeque<Result<u64,u64>>,sent:Vec<[u64;5]>}
static HOST:Mutex<Host>=Mutex::new(Host{peer:None,results:VecDeque::new(),sent:Vec::new()});
static SERIAL:Mutex<()>=Mutex::new(());
pub fn pdx_call_checked(slot:u64,op:u64,a:u64,b:u64,c:u64)->Result<u64,u64>{
    let mut h=HOST.lock().unwrap();let result=h.results.pop_front().unwrap_or(Ok(0));
    if result.is_ok(){h.sent.push([slot,op,a,b,c]);}result
}
#[derive(Default)] pub struct Scrollback {lines:Vec<Vec<u8>>}
impl Scrollback {pub fn push(&mut self,line:&[u8]){self.lines.push(line.to_vec())}}
#[path="../apps/spindle/src/record_command.rs"] mod command;
fn reset(){*HOST.lock().unwrap()=Host{peer:Some(Peer{epoch:3,life:crash_log::PeerLife::Live}),results:VecDeque::new(),sent:Vec::new()};}
fn drain(c:&mut command::Command){for _ in 0..20{if !c.sending(){return}c.poll()}panic!("sender did not drain")}
#[test]
fn real_sender_retries_chunks_and_discloses_before_admission(){
    let _serial=SERIAL.lock().unwrap();reset();assert!(desktop_recording::ENABLED);
    let mut c=command::Command::new();let mut sb=Scrollback::default();c.start(b"session.mkv",&mut sb);
    assert!(HOST.lock().unwrap().sent.is_empty());assert!(sb.lines.iter().any(|line|line.starts_with(b"Requested: whole desktop")));
    HOST.lock().unwrap().results.push_back(Err(ERR_SERVICE_NOT_READY));c.poll();assert!(HOST.lock().unwrap().sent.is_empty());drain(&mut c);
    let h=HOST.lock().unwrap();let sent=&h.sent;assert_eq!(sent.len(),3);
    let intents:Vec<_>=sent.iter().map(|v|desktop_recording::Intent::decode(desktop_recording::SPINDLE,[v[2],v[3],v[4]]).unwrap()).collect();
    assert!(matches!(intents[0],desktop_recording::Intent::Begin{sequence:1,length:11,shell_epoch:3}));
    assert!(matches!(intents[1],desktop_recording::Intent::Chunk{sequence:1,offset:0,..}));
    assert!(matches!(intents[2],desktop_recording::Intent::Commit{sequence:1}));
}
#[test]
fn stop_preempts_partial_name_without_files_buffer_or_false_success(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=command::Command::new();let mut sb=Scrollback::default();
    c.start(b"first.mkv",&mut sb);c.poll();c.stop(b"",&mut sb);drain(&mut c);
    let sent=HOST.lock().unwrap().sent.clone();assert_eq!(sent.len(),2);
    assert!(matches!(desktop_recording::Intent::decode(desktop_recording::SPINDLE,[sent[1][2],sent[1][3],sent[1][4]]),Some(desktop_recording::Intent::Stop{sequence:2})));
    assert!(!sb.lines.iter().any(|line|line.starts_with(b"Recording saved")));
}
#[test]
fn old_shell_positive_and_wrong_origin_never_show_active(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=command::Command::new();let mut sb=Scrollback::default();c.start(b"first.mkv",&mut sb);drain(&mut c);
    let positive=desktop_recording::intent_reply(1,desktop_recording::IntentStatus::Active).unwrap();
    assert!(!c.receive(desktop_recording::FILES,positive,&mut sb));
    HOST.lock().unwrap().peer=Some(Peer{epoch:4,life:crash_log::PeerLife::Live});assert!(c.receive(desktop_recording::SHELL,positive,&mut sb));
    assert!(!sb.lines.iter().any(|line|line.starts_with(b"Screen recording active")));
    c.stop(b"",&mut sb);drain(&mut c);
    assert!(c.receive(desktop_recording::SHELL,desktop_recording::intent_reply(2,desktop_recording::IntentStatus::Stopped).unwrap(),&mut sb));
    assert!(sb.lines.iter().any(|line|line.starts_with(b"Capture revoked")));
}
#[test]
fn invalid_names_and_unknown_or_retiring_shell_never_enqueue(){
    let _serial=SERIAL.lock().unwrap();reset();let mut sb=Scrollback::default();
    for name in [&b"../bad.mkv"[..],&b"@private.mkv"[..],&b"line\n.mkv"[..],&b"plain"[..],&b""[..]] {let mut c=command::Command::new();c.start(name,&mut sb);drain(&mut c)}
    for peer in [None,Some(Peer{epoch:3,life:crash_log::PeerLife::Ending})] {HOST.lock().unwrap().peer=peer;let mut c=command::Command::new();c.start(b"fine.mkv",&mut sb);drain(&mut c)}
    assert!(HOST.lock().unwrap().sent.is_empty());
}
