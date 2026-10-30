//! Actual resource Session against an independent public-fixture TLS endpoint.
//! Host sockets exercise private TLS/framing custody. Coordinator tests below
//! also use the actual bounded job machine and its RELEASE receipts; none of
//! this constitutes native BOOT or guest endpoint authority evidence.
extern crate self as sex_pdx;
#[path="../../sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../../../servers/sexnet/src/jobs.rs"]mod jobs;
#[path="../../../apps/kaleidoscope/src/tls_exchange.rs"]mod exchange;
use sex_tls::{Asset,Failure,HttpError,Resource,Session,State,MAX_DOCUMENT};
use std::{io::{BufRead,BufReader,Read,Write},net::{Shutdown,TcpStream},process::{Child,ChildStdout,Command,Stdio},time::{Duration,Instant}};
use std::sync::{Mutex,atomic::{AtomicU64,Ordering}};
pub const SLOT_NET:u64=18;pub const ERR_SERVICE_NOT_READY:u64=9;
static SERIAL:Mutex<()>=Mutex::new(());
static CALLS:Mutex<Vec<network_job::Request>>=Mutex::new(Vec::new());
static RELEASE_ADMISSION:AtomicU64=AtomicU64::new(0);
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
    assert_eq!((slot,op),(SLOT_NET,network_job::OP));let r=network_job::Request::decode([a,b,c]).unwrap();assert!(r.valid());
    if r.kind==network_job::RELEASE{let status=RELEASE_ADMISSION.load(Ordering::SeqCst);if status!=0{return(status,0);}}
    CALLS.lock().unwrap().push(r);(0,0)
}

const PEER:&str=r#"
import socket,ssl,sys,json
from pathlib import Path
base=Path(sys.argv[1]);mode=sys.argv[2]
want=b'GET /art.qoi HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: image/qoi\r\n\r\n'
# Valid solid 48x48 RGBA QOI: first pixel, 37 runs of62, final run9.
body=b'qoif'+(48).to_bytes(4,'big')*2+bytes([4,0,255,0,255,255,255])+bytes([253])*37+bytes([200])+bytes(7)+bytes([1])
assert len(body)==65
if mode=='full':body=bytes(range(256))*48
mime=b'text/plain' if mode=='mime' else b'image/qoi'
wire=b'HTTP/1.1 200 OK\r\nContent-Length: '+str(len(body)).encode()+b'\r\nContent-Type: '+mime+b'\r\n\r\n'+body
if mode=='trailing':wire+=b'EXTRA'
if mode=='partial':wire=wire[:-len(body)]+body[:17]
c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);c.minimum_version=c.maximum_version=ssl.TLSVersion.TLSv1_3
c.set_ecdh_curve('prime256v1');c.num_tickets=0;c.keylog_filename=None;c.load_cert_chain(base/'server.pem',base/'server-PUBLIC-TEST-key.pem')
names=[];c.set_servername_callback(lambda s,n,x:names.append(n))
l=socket.socket();l.bind(('127.0.0.1',0));l.listen(1);l.settimeout(12);print(l.getsockname()[1],flush=True)
s,_=l.accept();s.settimeout(12);t=c.wrap_socket(s,server_side=True);request=b'';closed=False
assert t.version()=='TLSv1.3' and t.cipher()[0]=='TLS_AES_128_GCM_SHA256' and names==['probe.sexos.test']
while not request.endswith(b'\r\n\r\n'):
 part=t.recv(512)
 assert part
 request+=part
 assert len(request)<=134
assert request==want
t.sendall(wire)
try:
 if mode in ('good','full'):t.unwrap().close();closed=True
 else:assert t.recv(512)==b''
except (ssl.SSLError,OSError):
 if mode in ('good','full'):raise
s.close();l.close()
print(json.dumps(dict(mode=mode,request_bytes=len(request),close_notify=closed,keylog_enabled=False)),flush=True)
"#;
struct Peer { child:Child,stdout:BufReader<ChildStdout>,stream:TcpStream }
impl Peer {
    fn start(mode:&str)->Self{
        let fixtures=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tls-controlled");
        let mut child=Command::new("python3").args(["-u","-c",PEER]).arg(fixtures).arg(mode).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
        let mut stdout=BufReader::new(child.stdout.take().unwrap());let mut port=String::new();stdout.read_line(&mut port).unwrap();let stream=TcpStream::connect(("127.0.0.1",port.trim().parse::<u16>().unwrap())).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(1))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();Self{child,stdout,stream}
    }
    fn turn(&mut self,session:&mut Session)->State{
        let state=session.poll();let mut out=[0;4096];let len=session.take_outgoing(&mut out);if len!=0{self.stream.write_all(&out[..len]).unwrap();}
        if !matches!(state,State::Rejected|State::ExchangeVerified){
            let mut input=[0;512];let len=input.len().min(session.incoming_capacity());
            if len!=0{match self.stream.read(&mut input[..len]){Ok(0)=>panic!("premature peer EOF"),Ok(n)=>session.offer_incoming(&input[..n]).unwrap(),Err(e)if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>{},Err(e)=>panic!("peer receive: {e}")}}
        }state
    }
    fn finish(&mut self,good:bool){
        self.stream.shutdown(Shutdown::Both).ok();let status=self.child.wait().unwrap();let mut output=String::new();self.stdout.read_to_string(&mut output).unwrap();assert!(status.success(),"{output}");
        assert!(output.contains(if good{"\"close_notify\": true"}else{"\"close_notify\": false"}),"{output}");assert!(output.contains("\"keylog_enabled\": false"));
    }
}
impl Drop for Peer{fn drop(&mut self){if self.child.try_wait().ok().flatten().is_none(){self.child.kill().ok();self.child.wait().ok();}}}
fn started()->Session{
    let mut seed=[31;32];let mut session=Session::resource(&mut seed,1_793_367_907,1,Resource::ArtworkQoi).unwrap();assert_eq!(seed,[0;32]);assert_eq!(session.resource_kind(),Some(Resource::ArtworkQoi));assert!(!session.is_document());assert!(session.take_asset().is_none());assert!(session.take_document().is_none());
    let stats=session.stats();assert!(stats.task_bytes<=sex_tls::MAX_TASK_BYTES);assert!(stats.private_bytes<65536);session
}
fn solid_qoi()->Vec<u8>{let mut b=b"qoif".to_vec();b.extend_from_slice(&48u32.to_be_bytes());b.extend_from_slice(&48u32.to_be_bytes());b.extend_from_slice(&[4,0,255,0,255,255,255]);b.extend_from_slice(&[253;37]);b.push(200);b.extend_from_slice(&[0;7]);b.push(1);assert_eq!(b.len(),65);b}
fn completed(mode:&str)->Asset{
    let mut peer=Peer::start(mode);let mut session=started();let start=Instant::now();let mut authenticated=false;
    loop{
        assert!(session.take_document().is_none());let state=peer.turn(&mut session);authenticated|=state==State::PeerAuthenticated;
        if state==State::ExchangeVerified{break;}assert_ne!(state,State::Rejected);assert!(session.take_asset().is_none());assert!(start.elapsed()<Duration::from_secs(12));
    }
    assert!(authenticated);assert_eq!(session.failure(),None);assert_eq!(session.http_failure(),None);assert!(session.take_document().is_none());let asset=session.take_asset().unwrap();assert_eq!(asset.resource_kind(),Resource::ArtworkQoi);assert!(session.take_asset().is_none());assert!(session.take_document().is_none());assert_eq!(session.allocated_payload_bytes(),2*sex_tls::QUEUE_BYTES);drop(session);peer.finish(true);asset
}
#[test] fn actual_qoi_session_transfers_only_asset_after_tls_framing_and_local_close(){let asset=completed("good");assert_eq!(asset.as_bytes(),solid_qoi());}
#[test] fn actual_binary_session_preserves_full_twelve_kib_without_text_conversion(){let asset=completed("full");let expected:Vec<_>=(0..MAX_DOCUMENT).map(|n|(n%256)as u8).collect();assert_eq!(asset.as_bytes(),expected);}
#[test] fn actual_resource_mime_and_same_feed_trailing_bytes_reject_without_asset_transfer(){
    for(mode,error)in[("mime",HttpError::UnsupportedContentType),("trailing",HttpError::TrailingData)]{
        let mut peer=Peer::start(mode);let mut session=started();let start=Instant::now();
        loop{let state=peer.turn(&mut session);assert!(session.take_asset().is_none());assert!(session.take_document().is_none());if state==State::Rejected{break;}assert_ne!(state,State::ExchangeVerified);assert!(start.elapsed()<Duration::from_secs(12));}
        assert_eq!(session.failure(),Some(Failure::Application));assert_eq!(session.http_failure(),Some(error));drop(session);peer.finish(false);
    }
}
#[test] fn actual_partial_binary_session_cannot_transfer_and_drop_closes_private_custody(){
    let mut peer=Peer::start("partial");let mut session=started();let start=Instant::now();
    while session.document_received()==0{let state=peer.turn(&mut session);assert!(!matches!(state,State::Rejected|State::ExchangeVerified));assert!(session.take_asset().is_none());assert!(session.take_document().is_none());assert!(start.elapsed()<Duration::from_secs(12));}
    assert_eq!(session.document_received(),17);assert!(session.take_asset().is_none());assert!(session.take_document().is_none());assert_eq!(session.http_failure(),None);drop(session);peer.finish(false);
}

struct Wire { stream:TcpStream,peer_fin:bool,closed:bool,finishes:usize }
impl jobs::Wire for Wire {
    fn begin(&mut self,_:u32)->Result<(),u32>{Ok(())}
    fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{match self.stream.read(out){Ok(0)=>{self.peer_fin=true;Ok(0)},Ok(n)=>Ok(n),Err(e)if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>Ok(0),Err(_)=>Err(network_job::IO)}}
    fn send(&mut self,bytes:&[u8])->Result<usize,u32>{self.stream.write(bytes).map_err(|_|network_job::IO)}
    fn queued(&self)->usize{0}
    fn link(&self)->jobs::Link{if self.closed{jobs::Link::Closed}else if self.peer_fin{jobs::Link::PeerFin}else{jobs::Link::Open}}
    fn finish_send(&mut self){self.finishes+=1;self.stream.shutdown(Shutdown::Write).unwrap();}
    fn abort(&mut self){self.closed=true;self.stream.shutdown(Shutdown::Both).ok();}
}
const OWNER:jobs::Identity=jobs::Identity{pd:15,task:15};
struct Exchange { peer:Peer,wire:Wire,client:exchange::Client,machine:jobs::Machine,requests:Vec<network_job::Request>,now:u64,began:Instant,auth:usize,verified:usize }
impl Exchange {
    fn new(mode:&str)->Self{
        assert!(CALLS.lock().unwrap().is_empty());RELEASE_ADMISSION.store(0,Ordering::SeqCst);let peer=Peer::start(mode);let stream=peer.stream.try_clone().unwrap();let mut session=started();
        for _ in 0..32{if session.poll()==State::HelloPending{break;}}assert_eq!(session.state(),State::HelloPending);
        let mut client=exchange::Client::new();assert!(client.start(session,1,3));assert_eq!(client.resource_kind(),Some(Resource::ArtworkQoi));
        Self{peer,wire:Wire{stream,peer_fin:false,closed:false,finishes:0},client,machine:jobs::Machine::new(),requests:vec![],now:0,began:Instant::now(),auth:0,verified:0}
    }
    fn turn(&mut self,hold_release:bool)->bool{
        self.now+=2;assert!(self.now<30_000);assert!(self.began.elapsed()<Duration::from_secs(12));self.client.advance(self.now);
        let resolution=jobs::Resolution{owner:OWNER,generation:3,address:[10,0,2,2],observed_tick:0,expires_tick:60_000};
        for r in CALLS.lock().unwrap().drain(..){assert!(self.requests.last().is_none_or(|old|r.seq>old.seq));assert!(self.machine.handle_resolved(OWNER,r,self.now,false,Ok(resolution),&mut self.wire));self.requests.push(r);}
        self.machine.advance(self.now,&mut self.wire);self.auth+=self.client.take_authenticated()as usize;self.verified+=self.client.take_response_verified()as usize;
        if let Some((owner,word))=self.machine.pending_reply(){assert_eq!(owner,OWNER);if hold_release&&self.requests.last().unwrap().kind==network_job::RELEASE{return true;}self.machine.reply_accepted();self.client.reply(word,self.now);}false
    }
    fn release_pending(&mut self){for _ in 0..15_000{if self.turn(true){return;}assert!(!self.client.has_asset());assert!(self.client.asset().is_none());assert!(self.client.take_asset().is_none());}panic!("no actual release");}
    fn complete(&mut self){for _ in 0..15_000{self.turn(false);if !self.client.busy(){return;}}panic!("cleanup pending");}
    fn assert_erased(&mut self){
        assert!(!self.client.has_asset());assert!(self.client.asset().is_none());assert!(self.client.take_asset().is_none());assert_eq!(self.client.retained_asset_len(),0);assert_eq!(self.client.retained_asset_hash(),0);assert_eq!(self.client.resource_kind(),None);assert_eq!(self.client.allocated_payload_bytes(),0);assert!(self.client.private_scrubbed_for_test());assert!(self.client.document().is_empty());assert!(self.client.take_document_for_import().is_none());
    }
}
#[test]fn actual_qoi_exchange_retains_private_asset_until_exact_release_and_moves_once(){
    let _serial=SERIAL.lock().unwrap();let mut h=Exchange::new("good");h.release_pending();assert_eq!((h.auth,h.verified),(1,1));assert_eq!(h.client.retained_asset_len(),65);assert_eq!(h.client.allocated_payload_bytes(),8192+MAX_DOCUMENT);assert!(!h.client.has_document());assert!(h.client.take_document_for_import().is_none());assert!(h.client.asset().is_none());assert!(h.client.take_asset().is_none());
    let r=*h.requests.last().unwrap();let(_,word)=h.machine.pending_reply().unwrap();let calls=h.requests.len();
    for foreign in [network_job::reply(r.seq+1,0),(0x55u64<<56)|((r.seq as u64)<<32)]{h.client.reply(foreign,h.now);assert!(h.client.take_asset().is_none());assert_eq!(h.client.retained_asset_len(),65);}
    assert!(!h.machine.retire(jobs::Identity{pd:15,task:16},&mut h.wire));for _ in 0..10{assert!(h.turn(true));assert_eq!(h.requests.len(),calls);assert!(h.client.asset().is_none());}
    h.machine.reply_accepted();h.client.reply(word,h.now);assert_eq!(h.client.view(),exchange::View::Verified);assert!(h.client.has_asset());assert_eq!(h.client.asset().unwrap().as_bytes(),solid_qoi());let released=h.client.take_released().unwrap();assert_eq!(released.resource,Some(Resource::ArtworkQoi));assert!(!released.document);assert_eq!(released.payload_bytes,MAX_DOCUMENT);assert_eq!(h.client.allocated_payload_bytes(),MAX_DOCUMENT);
    let asset=h.client.take_asset().unwrap();assert_eq!(asset.as_bytes(),solid_qoi());assert!(!h.client.has_asset());assert!(h.client.take_asset().is_none());assert_eq!(h.client.allocated_payload_bytes(),0);h.client.cancel();h.assert_erased();assert_eq!(asset.as_bytes(),solid_qoi());drop(asset);h.peer.finish(true);
}
#[test]fn actual_qoi_negative_or_invalid_release_erases_asset_and_cannot_replay(){
    let _serial=SERIAL.lock().unwrap();for invalid in [false,true]{let mut h=Exchange::new("good");h.release_pending();let r=*h.requests.last().unwrap();let word=if invalid{network_job::reply(r.seq,1)}else{network_job::error(r.seq,network_job::DENIED)};h.machine.reply_accepted();h.client.reply(word,h.now);assert_eq!(h.client.view(),exchange::View::Uncertain);h.assert_erased();let calls=h.requests.len();h.client.advance(h.now+2001);assert_eq!(h.requests.len(),calls);assert!(CALLS.lock().unwrap().is_empty());h.client.close();h.peer.finish(true);}
}
#[test]fn actual_qoi_release_timeout_late_ack_and_pre_effect_denial_never_publish(){
    let _serial=SERIAL.lock().unwrap();for denied in [false,true]{let mut h=Exchange::new("good");
        if denied{RELEASE_ADMISSION.store(network_job::DENIED as u64,Ordering::SeqCst);while h.client.view()!=exchange::View::Uncertain{h.turn(false);}h.assert_erased();assert!(h.machine.retire(OWNER,&mut h.wire));}
        else{h.release_pending();let(_,word)=h.machine.pending_reply().unwrap();h.client.advance(h.now+2000);h.assert_erased();h.machine.reply_accepted();h.client.reply(word,h.now+2000);assert_eq!(h.client.view(),exchange::View::Unavailable);h.assert_erased();assert_eq!(h.client.take_released().unwrap().resource,None);}
        h.client.close();RELEASE_ADMISSION.store(0,Ordering::SeqCst);h.peer.finish(true);
    }
}
#[test]fn actual_qoi_partial_cancel_and_terminal_close_drop_asset_without_replay(){
    let _serial=SERIAL.lock().unwrap();let mut h=Exchange::new("partial");while h.client.document_received()==0{h.turn(false);}assert_eq!(h.client.retained_asset_len(),0);h.client.cancel();h.complete();h.assert_erased();assert_eq!(h.client.view(),exchange::View::Cancelled);h.peer.finish(false);
    let mut h=Exchange::new("good");h.release_pending();let calls=h.requests.len();h.client.close();h.assert_erased();h.client.advance(h.now+2);assert_eq!(h.requests.len(),calls);assert!(CALLS.lock().unwrap().is_empty());h.peer.finish(true);
}
