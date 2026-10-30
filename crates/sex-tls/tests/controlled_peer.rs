//! Host-only interoperability of the actual owned TLS driver against an
//! independent Python/OpenSSL endpoint. No guest transport/authority evidence.
use sex_tls::{Session,State,EXCHANGE_REQUEST,EXCHANGE_RESPONSE};
use std::{io::{BufRead,BufReader,Read,Write},net::{TcpStream,Shutdown},process::{Command,Stdio},time::{Duration,Instant}};
extern crate self as sex_pdx;
#[path="../../sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../../../servers/sexnet/src/jobs.rs"]mod jobs;
#[path="../../../apps/kaleidoscope/src/tls_exchange.rs"]mod exchange_client;
pub const SLOT_NET:u64=18;pub const ERR_SERVICE_NOT_READY:u64=9;
static CLIENT_LOCK:std::sync::Mutex<()>=std::sync::Mutex::new(());
static CALLS:std::sync::Mutex<Vec<network_job::Request>>=std::sync::Mutex::new(Vec::new());
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
    assert_eq!((slot,op),(SLOT_NET,network_job::OP));CALLS.lock().unwrap().push(network_job::Request::decode([a,b,c]).unwrap());(0,0)
}
const PEER:&str=r#"
import socket,ssl,sys,json
from pathlib import Path
base=Path(sys.argv[1]);bad=sys.argv[2]=='bad'
c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
c.minimum_version=c.maximum_version=ssl.TLSVersion.TLSv1_3
c.set_ecdh_curve('prime256v1');c.num_tickets=0
c.load_cert_chain(base/'server.pem',base/'server-PUBLIC-TEST-key.pem')
names=[];c.set_servername_callback(lambda s,n,x:names.append(n))
l=socket.socket();l.bind(('127.0.0.1',0));l.listen(1);l.settimeout(10)
print(l.getsockname()[1],flush=True)
s,_=l.accept();s.settimeout(10);t=c.wrap_socket(s,server_side=True)
assert t.version()=='TLSv1.3' and t.cipher()[0]=='TLS_AES_128_GCM_SHA256'
want=b'SexOS guest encrypted request round93\n';data=b''
while len(data)<len(want):data+=t.recv(len(want)-len(data))
assert data==want and names==['probe.sexos.test']
reply=b'SexOS controlled encrypted reply round93\n'
t.sendall((b'X'+reply[1:]) if bad else reply)
try:t.unwrap().close();closed=True
except ssl.SSLError:
 if not bad:raise
 closed=False
print(json.dumps(dict(request=data.decode(),response=reply.decode(),close_notify=closed)),flush=True)
"#;
fn exchange(bad:bool){
    let fixtures=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tls-controlled");
    let mut child=Command::new("python3").args(["-u","-c",PEER]).arg(fixtures).arg(if bad{"bad"}else{"good"})
        .stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
    let mut stdout=BufReader::new(child.stdout.take().unwrap());let mut port=String::new();stdout.read_line(&mut port).unwrap();
    let mut stream=TcpStream::connect(("127.0.0.1",port.trim().parse::<u16>().unwrap())).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(1))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut seed=[19;32];let mut session=Session::exchange(&mut seed,1_793_367_907,1).unwrap();assert_eq!(seed,[0;32]);
    let start=Instant::now();let mut authenticated=false;
    loop{
        let state=session.poll();authenticated|=state==State::PeerAuthenticated;
        let mut out=[0;4096];let n=session.take_outgoing(&mut out);if n>0{stream.write_all(&out[..n]).unwrap();}
        if matches!(state,State::Rejected|State::ExchangeVerified){
            if bad {assert_eq!(state,State::Rejected);}else{assert_eq!(state,State::ExchangeVerified);assert!(authenticated);}
            break;
        }
        let mut incoming=[0;512];let available=incoming.len().min(session.incoming_capacity());
        if available>0{match stream.read(&mut incoming[..available]){
            Ok(0)=>panic!("peer closed before verified exchange"),
            Ok(n)=>session.offer_incoming(&incoming[..n]).unwrap(),
            Err(e)if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>{},
            Err(e)=>panic!("peer read: {e}"),
        }}
        assert!(start.elapsed()<Duration::from_secs(12),"finite TLS exchange deadline");
    }
    stream.shutdown(Shutdown::Both).ok();let status=child.wait().unwrap();let mut result=String::new();stdout.read_to_string(&mut result).unwrap();
    assert!(status.success(),"independent peer failed: {result}");
    assert!(result.contains(if bad{"\"close_notify\": false"}else{"\"close_notify\": true"}));
    assert_eq!(EXCHANGE_REQUEST.len(),38);assert_eq!(EXCHANGE_RESPONSE.len(),41);
}
#[test]fn actual_tls13_p256_peer_decrypts_request_and_accepts_encrypted_close(){exchange(false)}
#[test]fn authenticated_wrong_application_bytes_never_verify(){exchange(true)}

struct SocketWire{stream:TcpStream,peer_fin:bool,closed:bool,finishes:usize}
impl jobs::Wire for SocketWire{
    fn begin(&mut self,_:u32)->Result<(),u32>{Ok(())}
    fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{match self.stream.read(out){
        Ok(0)=>{self.peer_fin=true;Ok(0)},Ok(n)=>Ok(n),
        Err(e)if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>Ok(0),Err(_)=>Err(network_job::IO),
    }}
    fn send(&mut self,data:&[u8])->Result<usize,u32>{self.stream.write(data).map_err(|_|network_job::IO)}
    fn queued(&self)->usize{0}
    fn link(&self)->jobs::Link{if self.closed{jobs::Link::Closed}else if self.peer_fin{jobs::Link::PeerFin}else{jobs::Link::Open}}
    fn finish_send(&mut self){self.finishes+=1;self.stream.shutdown(Shutdown::Write).unwrap();}
    fn abort(&mut self){self.closed=true;self.stream.shutdown(Shutdown::Both).ok();}
}
#[test]fn actual_duplex_client_and_job_release_only_after_independent_encrypted_exchange(){
    let _guard=CLIENT_LOCK.lock().unwrap();
    let fixtures=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tls-controlled");
    let mut child=Command::new("python3").args(["-u","-c",PEER]).arg(fixtures).arg("good")
        .stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
    let mut stdout=BufReader::new(child.stdout.take().unwrap());let mut port=String::new();stdout.read_line(&mut port).unwrap();
    let stream=TcpStream::connect(("127.0.0.1",port.trim().parse::<u16>().unwrap())).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(1))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut wire=SocketWire{stream,peer_fin:false,closed:false,finishes:0};let mut machine=jobs::Machine::new();
    let owner=jobs::Identity{pd:15,task:15};let resolution=jobs::Resolution{owner,generation:3,address:[10,0,2,2],observed_tick:0,expires_tick:60_000};
    let mut seed=[19;32];let mut session=Session::exchange(&mut seed,1_793_367_907,1).unwrap();
    for _ in 0..16{if session.poll()==State::HelloPending{break;}}
    let mut client=exchange_client::Client::new();assert!(client.start(session,1,3));
    let start=Instant::now();let mut accepted=Vec::new();let(mut auth,mut verified)=(0,0);
    for now in (0..30_000).step_by(2){
        client.advance(now);
        for request in CALLS.lock().unwrap().drain(..){
            assert!(machine.handle_resolved(owner,request,now,false,Ok(resolution),&mut wire));accepted.push(request);
        }
        machine.advance(now,&mut wire);
        if let Some((actual,word))=machine.pending_reply(){assert_eq!(actual,owner);machine.reply_accepted();client.reply(word,now);}
        auth+=client.take_authenticated() as usize;verified+=client.take_response_verified() as usize;
        if !client.busy(){break;}
        assert!(start.elapsed()<Duration::from_secs(12),"actual duplex deadline; view={:?}",client.view());
    }
    assert_eq!(client.view(),exchange_client::View::Verified);assert_eq!((auth,verified),(1,1));
    assert!(machine.owner().is_none());assert_eq!(wire.finishes,1);
    assert!(accepted.windows(2).all(|w|w[0].seq<w[1].seq));
    assert_eq!(accepted.first().unwrap().kind,network_job::BEGIN_DUPLEX_RESOLVED);
    assert_eq!(accepted.last().unwrap().kind,network_job::RELEASE);
    let finish=accepted.iter().position(|r|r.kind==network_job::FINISH).unwrap();
    assert!(!accepted[finish+1..].iter().any(|r|r.kind==network_job::WRITE));
    let sequence=client.next_sequence();assert!(client.peer_authenticated());client.cancel();
    assert_eq!(client.next_sequence(),sequence);assert_eq!(client.ticket(),0);
    assert!(!client.peer_authenticated());assert!(client.stats().is_none());assert_eq!((client.tx_bytes(),client.rx_bytes()),(0,0));
    client.advance(29_999);assert!(CALLS.lock().unwrap().is_empty());
    assert!(child.wait().unwrap().success());let mut result=String::new();stdout.read_to_string(&mut result).unwrap();assert!(result.contains("\"close_notify\": true"));
}

// This endpoint uses the already source-bound public certificate bytes and
// server test key. It performs its own TLS handshake, never acts as a proxy.
const REJECTED_PEER:&str=r#"
import socket,ssl,sys,json,re,tempfile
from pathlib import Path
base=Path(sys.argv[1]);mode=sys.argv[2]
source=(base.parents[2]/'crates/sex-tls/src/probe_fixture.rs').read_text()
symbol={'wrong-name':'WRONG_NAME_DER','expired':'EXPIRED_DER','future':'FUTURE_DER','client-only':'CLIENT_ONLY_DER','foreign-root':'FOREIGN_DER'}.get(mode,'LEAF_DER')
match=re.search(r'pub const '+symbol+r':[^=]+=\s*&\s*\[(.*?)\];',source,re.S)
assert match is not None
cert=bytearray(int(v,16) for v in re.findall(r'0x([0-9a-fA-F]{2})',match[1]))
if mode=='certificate-signature':cert[-1]^=1
with tempfile.TemporaryDirectory(prefix='sexos-rejected-peer-') as d:
 pem=Path(d)/'public.pem';pem.write_text(ssl.DER_cert_to_PEM_cert(bytes(cert)))
 c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);c.keylog_filename=None
 c.minimum_version=c.maximum_version=ssl.TLSVersion.TLSv1_2 if mode=='unsupported-version' else ssl.TLSVersion.TLSv1_3
 c.set_ecdh_curve('prime256v1');c.num_tickets=0
 c.load_cert_chain(pem,base/'server-PUBLIC-TEST-key.pem')
 names=[];c.set_servername_callback(lambda s,n,x:names.append(n))
 l=socket.socket();l.bind(('127.0.0.1',0));l.listen(1);l.settimeout(10)
 print(l.getsockname()[1],flush=True)
 s,_=l.accept();s.settimeout(10);data=b'';failure=None
 try:
  with c.wrap_socket(s,server_side=True) as t:data=t.recv(128)
 except (ssl.SSLError,OSError) as e:failure=type(e).__name__
 assert data==b'' and failure is not None,(mode,data,failure)
 print(json.dumps(dict(mode=mode,application_bytes=len(data),handshake_failed=True,sni=names)),flush=True)
"#;

#[test]fn actual_rejected_peers_never_authenticate_and_release_the_exact_job(){
    let _guard=CLIENT_LOCK.lock().unwrap();
    for mode in ["wrong-name","expired","future","client-only","foreign-root","certificate-signature","unsupported-version"] {
        let fixtures=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tls-controlled").canonicalize().unwrap();
        let mut child=Command::new("python3").args(["-u","-c",REJECTED_PEER]).arg(fixtures).arg(mode)
            .stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
        let mut stdout=BufReader::new(child.stdout.take().unwrap());let mut port=String::new();stdout.read_line(&mut port).unwrap();
        let stream=TcpStream::connect(("127.0.0.1",port.trim().parse::<u16>().expect("endpoint setup"))).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(1))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut wire=SocketWire{stream,peer_fin:false,closed:false,finishes:0};let mut machine=jobs::Machine::new();
        let owner=jobs::Identity{pd:15,task:15};let resolution=jobs::Resolution{owner,generation:3,address:[10,0,2,2],observed_tick:0,expires_tick:60_000};
        let mut seed=[19;32];let mut session=Session::exchange(&mut seed,1_793_367_907,1).unwrap();assert_eq!(seed,[0;32]);
        for _ in 0..16{if session.poll()==State::HelloPending{break;}}
        let mut client=exchange_client::Client::new();assert!(client.start(session,1,3));
        let start=Instant::now();let mut accepted=Vec::new();
        for now in (0..30_000).step_by(2){
            client.advance(now);
            for request in CALLS.lock().unwrap().drain(..){
                assert!(machine.handle_resolved(owner,request,now,false,Ok(resolution),&mut wire));accepted.push(request);
            }
            machine.advance(now,&mut wire);
            if let Some((actual,word))=machine.pending_reply(){assert_eq!(actual,owner);machine.reply_accepted();client.reply(word,now);}
            assert!(!client.peer_authenticated(),"rejected {mode} authenticated");
            assert!(!client.take_authenticated());assert!(!client.take_response_verified());
            if !client.busy(){break;}
            assert!(start.elapsed()<Duration::from_secs(12),"rejection deadline {mode}; {:?}",client.view());
        }
        assert_eq!(client.view(),exchange_client::View::Unavailable,"{mode}");
        let want=if mode=="unsupported-version"{sex_tls::Failure::Handshake}else{sex_tls::Failure::Certificate};
        assert_eq!(client.failure(),Some(want),"explicit TLS rejection, not generic timeout: {mode}");
        assert!(client.private_scrubbed_for_test());assert!(machine.owner().is_none());assert!(wire.closed);
        assert_eq!(wire.finishes,0);assert!(client.ticket()>0);
        assert!(accepted.iter().any(|r|r.kind==network_job::CANCEL));
        assert_eq!(accepted.last().unwrap().kind,network_job::RELEASE);
        assert!(accepted.windows(2).all(|w|w[0].seq<w[1].seq));
        assert!(child.wait().unwrap().success(),"independent rejected endpoint {mode}");
        let mut result=String::new();stdout.read_to_string(&mut result).unwrap();
        assert!(result.contains("\"application_bytes\": 0")&&result.contains("\"handshake_failed\": true"),"{result}");
        println!("controlled rejection {mode}: {want:?}, exact positive RELEASE, no authenticated data");
    }
}
