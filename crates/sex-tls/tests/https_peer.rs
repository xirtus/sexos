//! Host-only actual TLS/HTTP interoperability through the owned Session, Browser
//! client and bounded job machine. Host sockets implement only this test peer;
//! this is not guest capability, native desktop or public-Internet evidence.
extern crate self as sex_pdx;
#[path="../../sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../../../servers/sexnet/src/jobs.rs"]mod jobs;
#[path="../../../apps/kaleidoscope/src/tls_exchange.rs"]mod exchange_client;
use sex_tls::{Session,State,Failure,HttpError,MAX_DOCUMENT};
use std::{io::{BufRead,BufReader,Read,Write},net::{TcpStream,Shutdown},process::{Child,ChildStdout,Command,Stdio},sync::{Mutex,atomic::{AtomicU64,Ordering}},time::{Duration,Instant}};
pub const SLOT_NET:u64=18;pub const ERR_SERVICE_NOT_READY:u64=9;
static SERIAL:Mutex<()>=Mutex::new(());
static CALLS:Mutex<Vec<network_job::Request>>=Mutex::new(Vec::new());
// Explicit release-admission faults only; actual TLS/body success always comes
// from Session + the independent SSL endpoint, never this injection.
static RELEASE_ADMISSION:AtomicU64=AtomicU64::new(0);
static RELEASE_ATTEMPTS:Mutex<Vec<(network_job::Request,u64)>>=Mutex::new(Vec::new());
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
    assert_eq!((slot,op),(SLOT_NET,network_job::OP));let r=network_job::Request::decode([a,b,c]).unwrap();assert!(r.valid());
    if r.kind==network_job::RELEASE{let status=RELEASE_ADMISSION.load(Ordering::SeqCst);RELEASE_ATTEMPTS.lock().unwrap().push((r,status));if status!=0{return(status,0);}}
    CALLS.lock().unwrap().push(r);(0,0)
}
const EXPECTED_REQUEST:&[u8]=b"GET /welcome HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: text/plain\r\n\r\n";
const TAIL:&[u8]=b"FINAL AUTHENTICATED HTTPS DOCUMENT BYTE Z";
fn body()->Vec<u8>{let mut b=Vec::new();for n in 0..300{b.extend_from_slice(format!("{n:03} SEXOS AUTHENTICATED HTTPS DOCUMENT / EXACT SOURCE BYTES").as_bytes());while b.len()%64!=63{b.push(b'.');}b.push(b'\n');}b.truncate(MAX_DOCUMENT-TAIL.len());b.extend_from_slice(TAIL);assert_eq!(b.len(),MAX_DOCUMENT);b}
const PEER:&str=r#"
import socket,ssl,sys,json,re,tempfile,hashlib
from pathlib import Path
base=Path(sys.argv[1]);mode=sys.argv[2]
want=b'GET /welcome HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: text/plain\r\n\r\n'
tail=b'FINAL AUTHENTICATED HTTPS DOCUMENT BYTE Z'
body=b''.join((f'{n:03} SEXOS AUTHENTICATED HTTPS DOCUMENT / EXACT SOURCE BYTES').encode().ljust(63,b'.')+b'\n' for n in range(300))[:12288-len(tail)]+tail
assert len(body)==12288
with tempfile.TemporaryDirectory(prefix='sexos-https-public-peer-') as d:
 cert=base/'server.pem'
 if mode=='wrong-name':
  source=(base.parents[2]/'crates/sex-tls/src/probe_fixture.rs').read_text();match=re.search(r'pub const WRONG_NAME_DER:[^=]+=\s*&\s*\[(.*?)\];',source,re.S);assert match is not None
  raw=bytes(int(v,16) for v in re.findall(r'0x([0-9a-fA-F]{2})',match[1]));cert=Path(d)/'wrong-name-public.pem';cert.write_text(ssl.DER_cert_to_PEM_cert(raw))
 c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);c.keylog_filename=None;c.minimum_version=c.maximum_version=ssl.TLSVersion.TLSv1_3
 c.set_ecdh_curve('prime256v1');c.num_tickets=0;c.load_cert_chain(cert,base/'server-PUBLIC-TEST-key.pem')
 names=[];c.set_servername_callback(lambda s,n,x:names.append(n))
 l=socket.socket();l.bind(('127.0.0.1',0));l.listen(1);l.settimeout(12);print(l.getsockname()[1],flush=True)
 s,_=l.accept();s.settimeout(12);request=b'';close_notify=False;response_bytes=0;failure=None;version=None;cipher=None;extra=b''
 try:
  t=c.wrap_socket(s,server_side=True);version=t.version();cipher=t.cipher()[0]
  assert version=='TLSv1.3' and cipher=='TLS_AES_128_GCM_SHA256' and names==['probe.sexos.test']
  while not request.endswith(b'\r\n\r\n'):
   part=t.recv(512)
   if not part:raise ValueError('guest closed before fixed HTTPS request')
   request+=part
   assert len(request)<=134
  assert request==want
  if mode=='good':response=b'HTTP/1.1 200 OK\r\nContent-Length: 12288\r\nContent-Type: text/plain\r\n\r\n'+body
  elif mode=='dup-length':response=b'HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Length: 4\r\nContent-Type: text/plain\r\n\r\nBODY'
  elif mode=='oversize':response=b'HTTP/1.1 200 OK\r\nContent-Length: 12289\r\nContent-Type: text/plain\r\n\r\n'
  elif mode=='mime':response=b'HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: text/html\r\n\r\nBODY'
  elif mode=='trailing':response=b'HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: text/plain\r\n\r\nBODYEXTRA'
  elif mode=='truncated':response=b'HTTP/1.1 200 OK\r\nContent-Length: 12288\r\nContent-Type: text/plain\r\n\r\n'+body[:256]
  else:raise ValueError('unexpected accepted endpoint mode')
  t.sendall(response);response_bytes=len(response)
  if mode=='good':t.unwrap().close();close_notify=True
  else:
   try:extra=t.recv(512)
   except (ssl.SSLError,OSError) as e:failure=type(e).__name__
   assert extra==b''
 except (ssl.SSLError,OSError) as e:failure=type(e).__name__
 finally:
  try:s.close()
  except OSError:pass
 if mode=='good':assert close_notify and request==want
 elif mode=='wrong-name':assert request==b'' and failure is not None
 else:assert request==want and not close_notify
 print(json.dumps(dict(mode=mode,request_hex=request.hex(),request_bytes=len(request),response_bytes=response_bytes,body_sha256=hashlib.sha256(body).hexdigest(),body_bytes=len(body),tls_version=version,cipher=cipher,sni=names,close_notify=close_notify,extra_application_bytes=len(extra),failure=failure,keylog_enabled=False)),flush=True)
"#;
struct Peer{child:Child,stdout:BufReader<ChildStdout>}
impl Peer{
    fn start(mode:&str)->(Self,TcpStream){
        let fixtures=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tls-controlled").canonicalize().unwrap();
        let mut child=Command::new("python3").args(["-u","-c",PEER]).arg(fixtures).arg(mode).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
        let mut stdout=BufReader::new(child.stdout.take().unwrap());let mut port=String::new();stdout.read_line(&mut port).unwrap();
        let stream=TcpStream::connect(("127.0.0.1",port.trim().parse::<u16>().expect("actual HTTPS endpoint setup"))).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(1))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();(Self{child,stdout},stream)
    }
    fn finish(&mut self,good:bool,wrong_name:bool){
        assert!(self.child.wait().unwrap().success(),"actual independent HTTPS endpoint");let mut output=String::new();self.stdout.read_to_string(&mut output).unwrap();
        assert!(output.contains(if good{"\"close_notify\": true"}else{"\"close_notify\": false"}),"{output}");assert!(output.contains("\"extra_application_bytes\": 0")&&output.contains("\"keylog_enabled\": false"),"{output}");
        assert!(output.contains(&format!("\"request_bytes\": {}",if wrong_name{0}else{EXPECTED_REQUEST.len()})),"{output}");
    }
}
impl Drop for Peer{fn drop(&mut self){if self.child.try_wait().ok().flatten().is_none(){self.child.kill().ok();self.child.wait().ok();}}}
struct SocketWire{stream:TcpStream,peer_fin:bool,closed:bool,finishes:usize}
impl jobs::Wire for SocketWire{
    fn begin(&mut self,_:u32)->Result<(),u32>{Ok(())}
    fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{match self.stream.read(out){Ok(0)=>{self.peer_fin=true;Ok(0)},Ok(n)=>Ok(n),Err(e)if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>Ok(0),Err(_)=>Err(network_job::IO)}}
    fn send(&mut self,data:&[u8])->Result<usize,u32>{self.stream.write(data).map_err(|_|network_job::IO)}
    fn queued(&self)->usize{0}fn link(&self)->jobs::Link{if self.closed{jobs::Link::Closed}else if self.peer_fin{jobs::Link::PeerFin}else{jobs::Link::Open}}
    fn finish_send(&mut self){self.finishes+=1;self.stream.shutdown(Shutdown::Write).unwrap();}fn abort(&mut self){self.closed=true;self.stream.shutdown(Shutdown::Both).ok();}
}
struct Harness{client:exchange_client::Client,machine:jobs::Machine,wire:SocketWire,requests:Vec<network_job::Request>,now:u64,began:Instant,auth:usize,verified:usize}
impl Harness{
    fn new(stream:TcpStream)->Self{
        assert!(CALLS.lock().unwrap().is_empty());RELEASE_ADMISSION.store(0,Ordering::SeqCst);RELEASE_ATTEMPTS.lock().unwrap().clear();let mut seed=[29;32];let mut session=Session::document(&mut seed,1_793_367_907,1).expect("bounded actual document future");assert_eq!(seed,[0;32]);assert!(session.is_document());assert!(session.take_document().is_none());
        let stats=session.stats();assert!(stats.task_bytes<=sex_tls::MAX_TASK_BYTES);assert!(stats.private_bytes<65536);let pipe=stats.private_bytes-stats.task_bytes-MAX_DOCUMENT;assert!((8192..9216).contains(&pipe));println!("actual HTTPS task={} private={} pipe={} document={}",stats.task_bytes,stats.private_bytes,pipe,MAX_DOCUMENT);
        for _ in 0..32{if session.poll()==State::HelloPending{break;}}assert_eq!(session.state(),State::HelloPending);assert!(session.take_document().is_none());
        let mut client=exchange_client::Client::new();assert!(client.start(session,1,3));assert!(client.is_document());assert!(client.document().is_empty());
        Self{client,machine:jobs::Machine::new(),wire:SocketWire{stream,peer_fin:false,closed:false,finishes:0},requests:vec![],now:0,began:Instant::now(),auth:0,verified:0}
    }
    fn turn(&mut self,hold_release:bool)->bool{
        assert!(self.began.elapsed()<Duration::from_secs(12),"actual HTTPS/client deadline view={:?}",self.client.view());assert!(self.now<30_000);self.now+=2;
        if self.client.view()!=exchange_client::View::Verified{assert!(self.client.document().is_empty());assert!(!self.client.has_document());}
        self.client.advance(self.now);
        let owner=jobs::Identity{pd:15,task:15};let resolution=jobs::Resolution{owner,generation:3,address:[10,0,2,2],observed_tick:0,expires_tick:60_000};
        for request in CALLS.lock().unwrap().drain(..){assert!(self.requests.last().is_none_or(|last|request.seq>last.seq));assert!(self.machine.handle_resolved(owner,request,self.now,false,Ok(resolution),&mut self.wire));self.requests.push(request);}
        self.machine.advance(self.now,&mut self.wire);self.auth+=self.client.take_authenticated()as usize;self.verified+=self.client.take_response_verified()as usize;
        let Some((actual,word))=self.machine.pending_reply()else{return false;};assert_eq!(actual,owner);
        if hold_release&&self.requests.last().is_some_and(|r|r.kind==network_job::RELEASE){assert_eq!(network_job::decode_reply(word,self.requests.last().unwrap().seq),Ok(0));assert!(self.machine.owner().is_none());return true;}
        self.machine.reply_accepted();self.client.reply(word,self.now);false
    }
    fn released(&mut self){for _ in 0..15_000{self.turn(false);if !self.client.busy(){return;}}panic!("HTTPS cleanup not complete");}
}
#[test]fn authenticated_full_document_stays_private_until_delayed_positive_release_then_q_erases(){
    let _serial=SERIAL.lock().unwrap();let(mut peer,stream)=Peer::start("good");let mut h=Harness::new(stream);
    loop{if h.turn(true){break;}}assert_eq!((h.auth,h.verified),(1,1));assert_eq!(h.client.retained_document_len(),MAX_DOCUMENT);assert_eq!(h.client.document_received(),MAX_DOCUMENT);assert!(h.client.document().is_empty());assert!(!h.client.has_document());assert_ne!(h.client.view(),exchange_client::View::Verified);
    let calls=h.requests.len();for _ in 0..10{assert!(h.turn(true));assert_eq!(h.requests.len(),calls);assert!(h.client.document().is_empty());}
    let(_,word)=h.machine.pending_reply().unwrap();h.machine.reply_accepted();h.client.reply(word,h.now);assert_eq!(h.client.view(),exchange_client::View::Verified);assert!(h.client.has_document());assert_eq!(h.client.document(),body());assert!(h.client.document().ends_with(TAIL));assert_eq!(h.wire.finishes,1);assert_eq!(h.client.failure(),None);assert_eq!(h.client.http_failure(),None);
    assert_eq!(h.requests.last().unwrap().kind,network_job::RELEASE);let finish=h.requests.iter().position(|r|r.kind==network_job::FINISH).unwrap();assert!(!h.requests[finish+1..].iter().any(|r|r.kind==network_job::WRITE));
    let seq=h.client.next_sequence();h.client.cancel();assert_eq!(h.client.next_sequence(),seq);assert!(h.client.document().is_empty());assert_eq!(h.client.retained_document_len(),0);assert!(h.client.private_scrubbed_for_test());h.client.advance(h.now+2);assert!(CALLS.lock().unwrap().is_empty());peer.finish(true,false);
}
#[test]fn malformed_authenticated_http_never_exposes_a_body_and_positively_releases(){
    let _serial=SERIAL.lock().unwrap();for(mode,want)in[("dup-length",HttpError::DuplicateContentLength),("oversize",HttpError::BodyLimit),("mime",HttpError::UnsupportedContentType),("trailing",HttpError::TrailingData)]{
        let(mut peer,stream)=Peer::start(mode);let mut h=Harness::new(stream);h.released();assert_eq!(h.client.view(),exchange_client::View::Unavailable,"{mode}");assert_eq!(h.client.failure(),Some(Failure::Application),"{mode}");assert_eq!(h.client.http_failure(),Some(want),"{mode}");assert_eq!((h.auth,h.verified),(1,0));assert!(h.client.peer_authenticated());assert!(h.client.document().is_empty());assert_eq!(h.client.retained_document_len(),0);assert!(h.client.private_scrubbed_for_test());assert!(h.machine.owner().is_none());assert!(h.wire.closed);assert_eq!(h.wire.finishes,0);assert!(h.requests.iter().any(|r|r.kind==network_job::CANCEL));assert_eq!(h.requests.last().unwrap().kind,network_job::RELEASE);peer.finish(false,false);println!("actual HTTPS {mode}: Application/{want:?}, no body, positive RELEASE");
    }
}
#[test]fn truncated_authenticated_body_stays_private_and_explicit_cancel_releases_without_eof_claim(){
    let _serial=SERIAL.lock().unwrap();let(mut peer,stream)=Peer::start("truncated");let mut h=Harness::new(stream);while h.client.document_received()==0{h.turn(false);}assert!(h.client.document_received()<=256);assert!(h.client.document().is_empty());assert_eq!(h.client.retained_document_len(),0);assert_eq!(h.client.failure(),None);assert_eq!(h.client.http_failure(),None);
    h.client.cancel();h.released();assert_eq!(h.client.view(),exchange_client::View::Cancelled);assert!(h.client.private_scrubbed_for_test());assert_eq!(h.client.failure(),None);assert_eq!(h.client.http_failure(),None);assert!(h.machine.owner().is_none());assert_eq!(h.wire.finishes,0);assert_eq!(h.verified,0);peer.finish(false,false);
}
#[test]fn wrong_name_certificate_cannot_send_http_or_publish_a_document(){
    let _serial=SERIAL.lock().unwrap();let(mut peer,stream)=Peer::start("wrong-name");let mut h=Harness::new(stream);h.released();assert_eq!(h.client.view(),exchange_client::View::Unavailable);assert_eq!(h.client.failure(),Some(Failure::Certificate));assert_eq!(h.client.http_failure(),None);assert_eq!((h.auth,h.verified),(0,0));assert!(!h.client.peer_authenticated());assert!(h.client.document().is_empty());assert!(h.client.private_scrubbed_for_test());assert!(h.machine.owner().is_none());peer.finish(false,true);
}
#[test]fn terminal_close_scrubs_a_published_full_document_and_cannot_restart_or_issue_pdx(){
    let _serial=SERIAL.lock().unwrap();let(mut peer,stream)=Peer::start("good");let mut h=Harness::new(stream);h.released();assert_eq!(h.client.view(),exchange_client::View::Verified);assert_eq!(h.client.document(),body());let seq=h.client.next_sequence();h.client.close();assert_eq!(h.client.next_sequence(),seq);assert!(h.client.document().is_empty());assert_eq!(h.client.retained_document_len(),0);assert!(h.client.private_scrubbed_for_test());h.client.advance(h.now+2);assert!(CALLS.lock().unwrap().is_empty());assert_eq!(h.client.view(),exchange_client::View::Cancelled);peer.finish(true,false);
}
#[test]fn pending_terminal_close_erases_partial_document_without_pdx_then_exact_parent_retirement(){
    let _serial=SERIAL.lock().unwrap();let(mut peer,stream)=Peer::start("truncated");let mut h=Harness::new(stream);while h.client.document_received()==0{h.turn(false);}assert!(h.machine.owner().is_some());let calls=h.requests.len();let seq=h.client.next_sequence();
    h.client.close();assert_eq!(h.client.next_sequence(),seq);assert_eq!(h.client.view(),exchange_client::View::Cancelled);assert!(h.client.document().is_empty());assert_eq!(h.client.retained_document_len(),0);assert!(h.client.private_scrubbed_for_test());h.client.advance(h.now+2);assert!(CALLS.lock().unwrap().is_empty());assert_eq!(h.requests.len(),calls);
    assert!(!h.machine.retire(jobs::Identity{pd:15,task:16},&mut h.wire));assert!(h.machine.owner().is_some());assert!(h.machine.retire(jobs::Identity{pd:15,task:15},&mut h.wire));assert!(h.machine.owner().is_none());assert!(h.machine.pending_reply().is_none());assert!(h.wire.closed);peer.finish(false,false);
}

// These failures occur after actual peer authentication, exact full HTTP body
// parsing and encrypted close_notify. They prove erasure on cleanup uncertainty,
// not a successful RELEASE or publication from an injected response.
fn assert_private_cleanup_failure(h:&mut Harness){
    assert_eq!(h.client.view(),exchange_client::View::Uncertain);assert!(h.client.busy());
    assert!(h.client.document().is_empty());assert!(!h.client.has_document());
    assert_eq!(h.client.retained_document_len(),0);assert_eq!(h.client.retained_document_hash(),0);
    assert!(h.client.private_scrubbed_for_test());assert_eq!(h.client.failure(),None);assert_eq!(h.client.http_failure(),None);
    let calls=RELEASE_ATTEMPTS.lock().unwrap().len();h.client.advance(h.now+2001);assert_eq!(RELEASE_ATTEMPTS.lock().unwrap().len(),calls);assert!(CALLS.lock().unwrap().is_empty());
}
#[test]fn actual_document_erased_on_pre_effect_release_full_timeout_or_denied_admission(){
    let _serial=SERIAL.lock().unwrap();for status in [ERR_SERVICE_NOT_READY,network_job::DENIED as u64]{
        let(mut peer,stream)=Peer::start("good");let mut h=Harness::new(stream);RELEASE_ADMISSION.store(status,Ordering::SeqCst);
        while RELEASE_ATTEMPTS.lock().unwrap().is_empty(){h.turn(false);}
        assert_eq!((h.auth,h.verified),(1,1));assert!(h.machine.owner().is_some());assert!(h.client.document().is_empty());
        if status==ERR_SERVICE_NOT_READY{
            assert_eq!(h.client.retained_document_len(),MAX_DOCUMENT);let request=RELEASE_ATTEMPTS.lock().unwrap()[0].0;
            for _ in 0..3{h.turn(false);}assert!(RELEASE_ATTEMPTS.lock().unwrap().iter().all(|(r,s)|*r==request&&*s==status));
            h.client.advance(h.now+2000);
        }
        assert_private_cleanup_failure(&mut h);assert!(h.machine.retire(jobs::Identity{pd:15,task:15},&mut h.wire));
        h.client.close();RELEASE_ADMISSION.store(0,Ordering::SeqCst);peer.finish(true,false);
    }
}
#[test]fn actual_document_erased_on_denied_or_invalid_release_ack(){
    let _serial=SERIAL.lock().unwrap();for invalid_positive in [false,true]{
        let(mut peer,stream)=Peer::start("good");let mut h=Harness::new(stream);while !h.turn(true){}
        assert_eq!((h.auth,h.verified),(1,1));assert_eq!(h.client.retained_document_len(),MAX_DOCUMENT);
        let r=*h.requests.last().unwrap();assert_eq!(r.kind,network_job::RELEASE);
        let injected=if invalid_positive{network_job::reply(r.seq,1)}else{network_job::error(r.seq,network_job::DENIED)};
        h.machine.reply_accepted();h.client.reply(injected,h.now);assert_private_cleanup_failure(&mut h);
        assert!(h.machine.owner().is_none());h.client.close();peer.finish(true,false);
    }
}
#[test]fn actual_document_release_timeout_and_late_exact_ack_cannot_publish(){
    let _serial=SERIAL.lock().unwrap();for reply_without_advance in [false,true]{
        let(mut peer,stream)=Peer::start("good");let mut h=Harness::new(stream);while !h.turn(true){}
        let(_,positive)=h.machine.pending_reply().unwrap();let request=*h.requests.last().unwrap();let calls=RELEASE_ATTEMPTS.lock().unwrap().len();
        assert_eq!(h.client.retained_document_len(),MAX_DOCUMENT);
        // Wrong correlation cannot finish the accepted operation or expose its body.
        h.client.reply(network_job::reply(request.seq+1,0),h.now);assert_eq!(h.client.retained_document_len(),MAX_DOCUMENT);assert!(h.client.document().is_empty());
        if !reply_without_advance{h.client.advance(h.now+2000);assert_eq!(h.client.view(),exchange_client::View::Uncertain);assert_eq!(h.client.retained_document_len(),0);assert!(h.client.private_scrubbed_for_test());}
        h.machine.reply_accepted();h.client.reply(positive,h.now+2000);
        assert!(!h.client.busy());assert_eq!(h.client.view(),exchange_client::View::Unavailable);assert_eq!(h.client.retained_document_len(),0);assert_eq!(h.client.retained_document_hash(),0);assert!(h.client.document().is_empty());assert!(h.client.private_scrubbed_for_test());
        h.client.advance(h.now+2001);assert_eq!(RELEASE_ATTEMPTS.lock().unwrap().len(),calls);assert!(CALLS.lock().unwrap().is_empty());peer.finish(true,false);
    }
}
