#!/usr/bin/env python3
"""Actual fixed controlled HTTPS endpoint, leased ciphertext and document pixels.

The host is the endpoint itself, with deliberately public laboratory fixtures.
No proxy, key log, public-Internet trust or transport-EOF inference is used.
"""
import argparse,ast,hashlib,json,math,os,re,shutil,socket,ssl,subprocess,threading,time
from pathlib import Path
import tls_exchange_gate as tls
import http_content_gate as http
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from certificate_time_gate import RTC_NORMAL
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from check_dns_capture import inspect as dns_capture
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_nonunix_target import inspect as target_check
from check_disk_persistence import ppm_rgb
from check_desktop_pixels import glyph

MODES=('ready','duplicate-length','oversize','unsupported-mime','partial-cancel','partial-close','wrong-name')
REQUEST=b'GET /welcome HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: text/plain\r\n\r\n'
TAIL=b'FINAL CONTROLLED HTTPS DOCUMENT BYTE 95 Z'
GUIDE=(b'SexOS / controlled HTTPS field guide',b'One shared address space supplies addresses; held capabilities supply authority.',b'BOOT reserves identities and grants finite routes. Applications cannot mint them.',b'This document came from the named laboratory endpoint through guest TLS.',b'The laboratory root is deliberately public. This does not establish public web trust.',b'Read with Up Down PgUp PgDn Home and End. V returns to the overview.',b'Q clears the retained document. Close retires the exact app incarnation.',b'Use the launcher to open Quil, Linen and Spindle. Save work before closing.',b'Quil edits documents. Linen lists workspace records. Spindle keeps bounded history.',b'Plain HTTP and controlled HTTPS are labelled separately in their readers.',b'Network bytes and document bytes are data; neither supplies new capability authority.',b'SexOS is a single address space microkernel operating system. It is not Unix.')
_corpus=b''.join(f'{n:03d} / '.encode()+GUIDE[n%len(GUIDE)]+b'\n'for n in range(300))
BODY=_corpus[:12288-len(TAIL)-1]+b'\n'+TAIL
assert len(BODY)==12288 and all(b==10 or 32<=b<=126 for b in BODY)
HEADER=b'Controlled HTTPS / lab root'
LABELS={'Pending':b'loading','Verified':b'ready','Unavailable':b'unavailable','Cancelled':b'cancelled','Uncertain':b'waiting receipt'}
SCOPE='Actual fixed BOOT-controlled HTTPS document through fresh held policy/seed/UTC/DNS/duplex authority; real endpoint OpenSSL and independent ciphertext capture plus complete bounded reader glyphs. Public laboratory root only; no host proxy, publicInternet/publicCA, arbitrary endpoint, HTML, physical/SMP/hostile-containment or daily-driver acceptance claim.'

def response(mode):
 body=BODY;headers=[b'HTTP/1.1 200 OK',b'Content-Type: text/plain',b'Content-Length: '+str(len(body)).encode(),b'Connection: close']
 if mode=='duplicate-length':headers.insert(3,b'Content-Length: '+str(len(body)+1).encode());body=b''
 elif mode=='oversize':headers[2]=b'Content-Length: 12289';body=b''
 elif mode=='unsupported-mime':headers[1]=b'Content-Type: image/png';body=b''
 elif mode in('partial-cancel','partial-close'):body=body[:1024]
 return b'\r\n'.join(headers)+b'\r\n\r\n'+body

class ControlledHTTPSPeer(tls.ControlledPeer):
 def __init__(self,root,mode):
  super().__init__(root,mode);self.response_sent=threading.Event();self.partial_sent=threading.Event()
 def serve_tls(self,listener,ctx):
  if self.mode=='wrong-name':return super().serve_tls(listener,ctx)
  sock=self.accept(listener)
  if sock is None:return
  row=dict(mode=self.mode,sni=None,handshake_complete=False,tls_version=None,cipher=None,request_hex=None,response_hex=None,application_bytes_received=0,extra_application_bytes=0,unwrap_completed=False);self.rows.append(row)
  wrapped=ctx.wrap_socket(sock,server_side=True,do_handshake_on_connect=False);self.live.append(wrapped);wrapped.do_handshake();row.update(handshake_complete=True,sni=getattr(wrapped,'controlled_sni',None),tls_version=wrapped.version(),cipher=wrapped.cipher()[0])
  if(row['tls_version'],row['cipher'],row['sni'])!=('TLSv1.3',tls.CIPHER,tls.HOST):raise ValueError('actual HTTPS endpoint negotiated a foreign TLS profile')
  request=bytearray()
  while len(request)<len(REQUEST):
   chunk=wrapped.recv(4096)
   if not chunk:raise ValueError('guest closed before exact HTTPS request')
   request.extend(chunk)
   if len(request)>len(REQUEST):raise ValueError('extra guest HTTPS application bytes')
  if bytes(request)!=REQUEST:raise ValueError('actual decrypted HTTPS request differs from fixed independent request')
  row.update(request_hex=bytes(request).hex(),application_bytes_received=len(request));wire=response(self.mode);
  for offset in range(0,len(wire),1024):wrapped.sendall(wire[offset:offset+1024])
  row['response_hex']=wire.hex();self.response_sent.set()
  if self.mode=='ready':
   plain=wrapped.unwrap();row['unwrap_completed']=True;plain.close();self.completed.set()
  else:
   if self.mode.startswith('partial-'):self.partial_sent.set()
   wrapped.settimeout(.1)
   while not self.stop.is_set():
    try:chunk=wrapped.recv(4096)
    except socket.timeout:continue
    except(ssl.SSLZeroReturnError,ssl.SSLEOFError,ConnectionResetError):break
    if not chunk:break
    row['extra_application_bytes']+=len(chunk);raise ValueError('rejected/partial HTTPS endpoint received extra application bytes')
   wrapped.close()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('extra HTTPS endpoint connection')
   except socket.timeout:pass

def inspect_wire(raw,peer_rows,mode):
 errors=[];joins=[]
 try:
  assert mode in MODES,'unknown HTTPS mode'
  flows,packets=tls.reassemble(raw);baseline=[f for f in flows if f['peer_port']==46078];actual=[f for f in flows if f['peer_port']==46080]
  assert len(flows)==2 and len(baseline)==len(actual)==1,'expected sole startup Echo and one fixed HTTPS connection'
  echo=baseline[0];assert echo['outgoing']==echo['incoming']==tls.ECHO and echo['fin_out']and echo['fin_in'],'startup Echo differs'
  assert len(peer_rows)==1,'missing/extra actual HTTPS endpoint observation'
  row=peer_rows[0];flow=actual[0];assert row['mode']==mode,'foreign endpoint mode'
  for clear in(REQUEST,b'GET /welcome HTTP/1.1',b'HTTP/1.1 200 OK',TAIL,BODY[:64]):
   assert clear not in flow['incoming']and clear not in flow['outgoing'],'HTTP application plaintext appeared on TLS wire'
  outgoing=tls.tls_records(flow['outgoing']);incoming=tls.tls_records(flow['incoming']);client=tls.hello(outgoing[0],True);server=tls.hello(incoming[0],False)
  assert any(r['type']==23 for r in incoming),'actual peer certificate flight absent'
  if mode=='wrong-name':
   assert row.get('fixture')==tls.fixture_descriptor(Path(__file__).resolve().parents[1],mode),'wrong source-bound public bad certificate'
   assert row.get('handshake_started')is True and row.get('server_flight_complete')is True and row.get('handshake_complete')is False,'wrong-name fixture lacks actual incomplete handshake flight'
   assert row.get('sni')==tls.HOST and row.get('application_bytes_received')==row.get('guest_bytes_after_flight')==0,'wrong-name peer received application/extra bytes'
   assert not row.get('request_hex')and not row.get('response_hex')and not row.get('unwrap_completed')and not any(r['type']==23 for r in outgoing),'rejected certificate received HTTP or fabricated TLS success'
   assert row.get('sent_wire_bytes')==len(flow['incoming'])and row.get('sent_wire_sha256')==hashlib.sha256(flow['incoming']).hexdigest(),'bad certificate flight differs from independently captured ciphertext'
  else:
   assert(row.get('tls_version'),row.get('cipher'),row.get('sni'))==('TLSv1.3',tls.CIPHER,tls.HOST)and row.get('handshake_complete')is True,'actual endpoint TLS handshake absent'
   assert row.get('request_hex')==REQUEST.hex()and row.get('application_bytes_received')==len(REQUEST)and row.get('extra_application_bytes')==0,'actual decrypted request differs'
   assert row.get('response_hex')==response(mode).hex(),'actual sent encrypted HTTP response differs from independent fixture'
   assert sum(r['length']for r in outgoing if r['type']==23)>=len(REQUEST)+17,'guest encrypted request extent absent'
   assert sum(r['length']for r in incoming if r['type']==23)>=len(response(mode))+17,'actual encrypted HTTP response extent absent'
   if mode=='ready':assert row.get('unwrap_completed')is True and flow['fin_out']and flow['fin_in']and not flow['rst_out']and not flow['rst_in'],'positive HTTPS lacks actual guest close_notify and clean TCP finish'
   else:assert not row.get('unwrap_completed'),'malformed/partial HTTP fixture fabricated positive completion'
  joins.append(dict(ticket=flow['guest_port']-48000,guest_port=flow['guest_port'],peer_port=46080,syn=flow['syn'],client_hello=client,server_hello=server,outgoing_bytes=len(flow['outgoing']),incoming_bytes=len(flow['incoming']),initial_tls_bytes=outgoing[0]['length']+5,outgoing_records=[{k:r[k]for k in('type','version','length')}for r in outgoing],incoming_records=[{k:r[k]for k in('type','version','length')}for r in incoming]))
 except(AssertionError,ValueError,KeyError,TypeError,IndexError)as exc:errors.append(str(exc));packets=locals().get('packets',0)
 return dict(ok=not errors,errors=errors,joins=joins,tcp_packets=packets,scope='Independent checksum/TCP/Hello/outer-record framing and absence of public HTTP plaintext. Actual endpoint SSL establishes decrypted HTTP bytes; serial labels alone never establish authentication. Malformed/partial/certificate-negative modes make no TLS close_notify or TCP FIN claim.')

def load_contract(root):
 root=Path(root);contract=http.load_pixel_contract(root);contract['source_sha256'].update(tls.load_contract(root)['source_sha256']);contract.update(request=REQUEST,reader_header=HEADER,reader_path=b' /welcome',dynamic_reader_path=False);contract['reader_footers']=contract['https_reader_footers']
 # The independent body is public checker data, not a guest startup fixture.
 contract['expected_body_sha256']=hashlib.sha256(BODY).hexdigest();contract['expected_request_sha256']=hashlib.sha256(REQUEST).hexdigest()
 for name in('scripts/https_document_gate.py','scripts/tls_exchange_gate.py','crates/sex-tls/src/lib.rs','crates/sex-tls/src/verifier.rs','crates/sex-tls/src/probe_fixture.rs','crates/sex-trust-config/src/lib.rs','crates/sex-http/src/lib.rs','crates/sex-http/src/codec.rs','apps/kaleidoscope/src/tls_exchange.rs','tests/fixtures/tls-controlled/server.pem','tests/fixtures/tls-controlled/server-PUBLIC-TEST-key.pem'):
  contract['source_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 reader=(root/'apps/kaleidoscope/src/document_reader.rs').read_text();app=(root/'apps/kaleidoscope/src/main.rs').read_text();view=(root/'apps/kaleidoscope/src/browser_view.rs').read_text()
 if 'self.render_source(body,path,b"Controlled HTTPS / lab root",b"probe.sexos.test:46080")'not in reader or 'self.reader.render_https(body,sex_tls::HTTPS_PATH)'not in app or 'selected:2|256|512'not in app:raise ValueError('source-bound authenticated HTTPS reader selection changed')
 if 'b"\\n\\nHTTPS / lab root: "'not in view or 'b"\\nB fetch V read Q clear"'not in view:raise ValueError('source-bound HTTPS status chrome changed')
 source=(root/'crates/sex-tls/src/lib.rs').read_text();meta=re.search(r'pub const PIPE_METADATA_BYTES\s*:\s*usize\s*=\s*(\d+)',source)
 if not meta or int(meta[1])!=56 or 'core::mem::size_of::<Pipe>()==PIPE_BYTES'not in source or 'PIPE_BYTES:usize=2*QUEUE_BYTES+PIPE_METADATA_BYTES'not in source:raise ValueError('source-bound x86_64 Pipe metadata/compile assertion differs')
 path=re.search(r'pub const HTTPS_PATH[^=]*=\s*(b"[^"\n]*");',source)
 if not path or ast.literal_eval(path[1])!=b'/welcome':raise ValueError('source-bound HTTPS document path differs')
 contract['pipe_bytes']=8192+int(meta[1]);return contract

def authority_joins(trace,wire,dns,raw,host_start,host_end,contract):
 assert wire['ok']and dns['ok'],'independent actual HTTPS/DNS wire proof absent'
 for marker in('[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1'):
  assert trace.count(marker)==1 and trace.count(marker.split(']')[0]+']')==1,'missing/foreign/duplicate exact BOOT grant or raw-source denial'
 private_lines=[line for line in trace.splitlines()if re.search(r'\[(?:browser\.(?:tls|https)|entropy\.)',line)]
 assert not any(re.search(r'\b(?:key|keylog|traffic_secret|master_secret|seed_bytes|seed_value|secret)=',line)for line in private_lines),'private TLS/seed material in metadata'
 intent=re.search(r'\[browser.https.intent\] accepted=1 explicit=1 public_internet=0',trace);assert intent and trace.count('[browser.https.intent]')==1,'missing exact actual B intent'
 policy=re.search(r'\[trust.policy.snapshot\] owner=15 task=15 seq=1 profile=1 root_bytes=376 origin=held_boot_grant',trace);assert policy and trace.count('[trust.policy.snapshot]')==1 and intent.start()<policy.start(),'missing fresh exact policy query'
 source=re.search(r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=(\d+) attempts=([1-4]) precision=1 profile=q35_cmos_utc ordinal=1',trace);service=re.search(r'\[certificate.time.snapshot\] owner=15 task=15 seq=1 seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace)
 assert source and service and trace.count('[clock.source.snapshot]')==trace.count('[certificate.time.snapshot]')==1 and source[1]==service[1]and policy.start()<source.start()<service.start(),'missing fresh exact UTC receipt'
 assert math.floor(host_start)-1<=int(source[1])<=math.ceil(host_end)+1,'UTC outside actual trusted host bracket'
 seed=re.search(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=1 bytes=32 source=held_boot_grant',trace);produced=re.search(r'\[entropy.seed.progress\] owner=Some\(Identity \{ pd: 15, task: 15 \}\) ticket=1 state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace);released=re.search(r'\[entropy.seed.released\] owner=15 task=15 ticket=1 service_payload=erased receipt=retained',trace)
 assert seed and produced and released and trace.count('[entropy.seed.admitted]')==trace.count('[entropy.seed.released]')==1 and policy.start()<seed.start()<produced.start()<released.start(),'missing fresh produced32/positive seed cleanup'
 check=re.search(r'\[browser.trust.self_check\] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',trace);prep=re.search(r'\[browser.https.prepared\] task_bytes=(\d+) private_bytes=(\d+) tx_bytes=(\d+) utc_snapshot=(\d+) request_bytes=94 document_limit=12288 network=0',trace)
 assert check and prep and trace.count('[browser.trust.self_check]')==trace.count('[browser.https.prepared]')==1 and max(released.start(),service.start())<check.start()<prep.start(),'local prerequisite preparation proof missing'
 task,private,initial,utc=map(int,prep.groups());assert 0<task<=32768 and private==task+contract['pipe_bytes']+12288 and private<49152 and 0<initial<=4096 and utc==int(source[1]),'private task/buffer/UTC bound differs'
 shared=re.search(r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0',trace);assert shared and trace.count('[browser.tls.prepared]')==1 and tuple(map(int,shared.groups()))==(task,private,initial,utc),'shared/document preparation metadata disagree'
 assert initial==wire['joins'][0]['initial_tls_bytes'],'prepared ClientHello extent differs from first actual TLS record'
 peer_admit=re.search(r'\[browser.https.admitted\] ticket=(\d+) dns_generation=(\d+) seq=(\d+) task_bytes=(\d+) private_bytes=(\d+) profile=1 authority=held_route',trace);assert peer_admit and trace.count('[browser.https.admitted]')==1,'missing correlated nonzero duplex lease receipt'
 ticket,generation,seq,atask,aprivate=map(int,peer_admit.groups());assert ticket==wire['joins'][0]['ticket']and ticket==2 and generation==1 and 0<seq<=0xffffff and(atask,aprivate)==(task,private),'wrong peer/DNS/sequence/private join'
 resolution=http.resolution_joins(trace,raw,[(ticket,generation)],dns);assert resolution['ok'],resolution['errors']
 assert re.search(rf'\[net.job.state\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={ticket} ',trace),'duplex lease lacks actual authenticated owner/task state'
 admission=re.search(rf'\[net.job.duplex.admitted\] owner=15 task=15 ticket={ticket} kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route',trace)
 assert admission and trace.count('[net.job.duplex.admitted]')==1 and prep.start()<admission.start()<peer_admit.start(),'missing exact service-backed duplex admission'
 return dict(ticket=ticket,dns_generation=generation,sequence=seq,task_bytes=task,private_bytes=private,initial_tls_bytes=initial,utc_snapshot=utc,prepared_offset=prep.start(),admitted_offset=peer_admit.start(),resolution=resolution)

def viewport(trace):
 notices=list(re.finditer(r'\[browser.viewport\] target=15 task=15 sequence=(\d+) w=(\d+) h=(\d+) origin=held_shell',trace));assert notices,'missing actual Browser viewport'
 sequence,w,h=map(int,notices[-1].groups());queued=re.findall(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={sequence} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace)
 assert len(queued)==1,'viewport lacks one exact queued Shell geometry'
 x,y,qw,qh=map(int,queued[0]);assert(w,h)==(qw,qh),'held-shell viewport dimensions differ'
 return dict(x=x,y=y,w=w,h=h),sequence

def frame_pixels(ppm,expected,contract,resolution=None):
 """Pure complete text-band oracle; separate caller binds authenticated receipt."""
 width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=expected['tile'];factor=expected['scale'];color=contract['color'].to_bytes(3,'big');checked=ink=mismatches=0
 if resolution and(width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
 if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('reader tile outside framebuffer')
 columns=min(80,max(0,(tile['w']-2*contract['inset'])//(contract['advance']*factor)));band_rows=min(contract['max_rows'],max(0,(tile['h']-contract['top']+contract['line']*factor-1)//(contract['line']*factor)))
 for row in range(band_rows):
  line=expected['lines'][row]if row<expected['rows']else b''
  for column in range(columns):
   bitmap=glyph(contract['font'],chr(line[column]))if column<len(line)else(0,)*7
   for gy in range(contract['line']):
    for gx in range(contract['advance']):
     on=gy<contract['glyph_height']and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
     for dy in range(factor):
      py=tile['y']+contract['top']+(row*contract['line']+gy)*factor+dy
      if py>=tile['y']+tile['h']:continue
      for dx in range(factor):
       px=tile['x']+contract['inset']+(column*contract['advance']+gx)*factor+dx;actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;checked+=1;ink+=on;mismatches+=actual!=on
 if mismatches or not ink:raise ValueError('whole HTTPS reader glyph/padding/stale-erasure mismatches='+str(mismatches))
 return dict(ok=True,errors=[],checked_pixels=checked,expected_ink_pixels=ink,geometry={k:v for k,v in expected.items()if k not in('starts','lines')},expected_rows=[line.decode('ascii')for line in expected['lines']])

HTTP_FAILURE={'duplicate-length':'DuplicateContentLength','oversize':'BodyLimit','unsupported-mime':'UnsupportedContentType'}
def inspect_document(trace,mode,wire,dns,raw,host_start,host_end,contract):
 errors=[];joins=[]
 try:
  assert mode in MODES,'unknown HTTPS mode';joined=authority_joins(trace,wire,dns,raw,host_start,host_end,contract);ticket=joined['ticket'];admitted=joined['admitted_offset']
  auth=list(re.finditer(rf'\[browser.https.authenticated\] ticket={ticket} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1',trace));assert len(auth)==trace.count('[browser.https.authenticated]'),'foreign/malformed HTTPS authentication'
  body=list(re.finditer(rf'\[browser.https.body\] ticket={ticket} bytes=(\d+) hash=(0x[0-9a-f]+) parsed=1 authenticated=1 visible=0',trace));positive=list(re.finditer(rf'\[browser.https.released\] ticket={ticket} tx=(\d+) rx=(\d+) request_bytes=94 body_bytes=(\d+) body_hash=(0x[0-9a-f]+) positive_release=1 authenticated=1 public_test_root=1',trace));assert len(body)==trace.count('[browser.https.body]')and len(positive)==trace.count('[browser.https.released]'),'foreign/malformed HTTPS body/release'
  views=list(re.finditer(r'\[browser.https.view\] state=(Pending|Verified|Unavailable|Cancelled|Uncertain) controlled_profile=1 public_internet=0',trace));assert len(views)==trace.count('[browser.https.view]')and any(v[1]=='Pending'for v in views),'missing/scoped HTTPS status'
  progress=list(re.finditer(rf'\[browser.https.progress\] ticket={ticket} body_received=(\d+) authenticated=1 body_visible=0',trace));assert len(progress)==trace.count('[browser.https.progress]'),'foreign/visible uncommitted HTTPS progress'
  assert all(0<int(v[1])<=len(BODY)and auth and auth[0].start()<v.start()for v in progress),'invalid/unauthed HTTPS progress'
  if mode!='wrong-name':assert len(auth)==1 and admitted<auth[0].start(),'actual HTTP endpoint missing guest TLS authentication'
  else:assert not auth and not progress,'wrong-name peer became authenticated'
  if mode=='ready':
   assert len(body)==len(positive)==1 and int(body[0][1])==len(BODY)and body[0][2]==hex(tls.fingerprint(BODY)),'complete authenticated expected body absent'
   released=positive[0];assert(int(released[1]),int(released[2]),int(released[3]),released[4])==(wire['joins'][0]['outgoing_bytes'],wire['joins'][0]['incoming_bytes'],len(BODY),hex(tls.fingerprint(BODY))),'released encrypted cursors/body/hash differ from actual independent document'
   verified=[v for v in views if v[1]=='Verified'];assert len(verified)==1 and auth[0].start()<body[0].start()<released.start()<verified[0].start(),'reader readiness published before positive RELEASE'
   assert '[browser.https.rejected]'not in trace,'successful HTTPS also claimed rejection'
   joined.update(authenticated=True,body_bytes=len(BODY),body_hash=hex(tls.fingerprint(BODY)),released_offset=released.start())
  else:
   assert not body and not positive and not any(v[1]=='Verified'for v in views),'malformed/partial/wrong-name HTTPS published a document'
   assert not re.search(r'\[browser.reader.layout\][^\r\n]*tls=1',trace),'unreleased HTTPS opened reader'
   if mode in HTTP_FAILURE or mode=='wrong-name':
    expected_auth=0 if mode=='wrong-name'else 1;failure='Certificate'if mode=='wrong-name'else'Application';http_failure='None'if mode=='wrong-name'else'Some('+HTTP_FAILURE[mode]+')'
    rejected=list(re.finditer(rf'\[browser.https.rejected\] ticket={ticket} positive_release=1 authenticated={expected_auth} failure=Some\({failure}\) http={re.escape(http_failure)}(?=\r?$)',trace,re.M));unavailable=[v for v in views if v[1]=='Unavailable'];assert len(rejected)==trace.count('[browser.https.rejected]')==len(unavailable)==1 and admitted<rejected[0].start()<unavailable[0].start(),'wrong/missing rejection reason or positive cleanup before Unavailable'
   else:
    assert progress and max(int(p[1])for p in progress)==1024,'partial HTTP did not reach actual withheld body prefix'
    if mode=='partial-cancel':
     cancel=re.search(rf'\[browser.https.cancelled\] ticket={ticket} positive_release=1 authenticated=1',trace);assert trace.count('[browser.https.cancelled]')==1,'foreign/duplicate partial cancellation receipt';cancelled=[v for v in views if v[1]=='Cancelled'];assert cancel and len(cancelled)==1 and progress[-1].start()<cancel.start()<cancelled[0].start(),'Q missing authenticated partial lease cleanup before Cancelled'
    else:
     close=re.search(r'\[browser.https.closed\] pending=1 terminal=1 owned_storage=erased body_visible=0',trace);assert trace.count('[browser.https.closed]')==1,'foreign/duplicate Close receipt';ack=re.search(r'\[net.app.cleanup.ack\] target=15 task=15 client_leases=0 shared_cache=retained ok=1',trace);assert close and ack and progress[-1].start()<close.start()<ack.start(),'Close missing partial future/body cleanup before ACK';cleanup_receipts(trace,15,15,close.start())
     for marker in('[trust.policy.retired] target=15 task=15 before_cleanup_ack=1','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1'):assert close.start()<trace.index(marker)<ack.start(),'prerequisite retired after ACK'
     assert not re.search(r'\[net\.(?:job|dns)\.egress\.submit\]',trace[ack.end():]),'retired app emitted network work after cleanupACK'
   joined.update(authenticated=mode!='wrong-name',body_bytes=0)
  joins.append(joined)
 except(AssertionError,ValueError,KeyError,IndexError,TypeError,AttributeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,scope=SCOPE)

def reader_geometry(trace,contract,anchor=0):
 releases=list(re.finditer(r'\[browser.https.released\] ticket=(\d+) tx=\d+ rx=\d+ request_bytes=94 body_bytes=(\d+) body_hash=(0x[0-9a-f]+) positive_release=1 authenticated=1 public_test_root=1',trace));assert releases,'missing positive authenticated HTTPS document release';release=releases[-1];assert int(release[1])==2 and int(release[2])==len(BODY)and release[3]==hex(tls.fingerprint(BODY)),'reader belongs to foreign/stale document'
 tail=trace[release.end():];assert not re.search(r'\[browser.https.cleared\]|\[browser.https.view\] state=(Pending|Cancelled|Unavailable|Uncertain)',tail),'released HTTPS document was superseded/cleared'
 tile,sequence=viewport(trace);frame=http.reader_frame(BODY,tile['w'],tile['h'],contract,anchor,b'/welcome');frame.update(tile=tile,viewport=sequence,document_ticket=2)
 layouts=list(re.finditer(r'\[browser.reader.layout\] first=(\d+) end=(\d+) body_rows=(\d+) rows=(\d+) cols=(\d+) total_lines=(\d+) source_bytes=(\d+) viewport=(\d+) origin=authenticated_body tls=1 public_test_root=1 pixels=unconfirmed',trace));wanted=tuple(frame[k]for k in('first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'))
 assert layouts and layouts[-1].start()>release.end()and tuple(map(int,layouts[-1].groups()))==wanted,'HTTPS reader layout differs from independent authenticated document/navigation'
 http.reader_position(trace,layouts[-1].end(),frame,1,contract.get('reader_require_position',False))
 assert re.search(rf'\[browser.view.applied\] bytes={frame["bytes"]} http=770 body_bytes=12288 pixels=unconfirmed',trace[layouts[-1].end():]),'missing positive atomic HTTPS reader Applied'
 return frame

def verify_reader_pixels(trace,ppm,contract,anchor=0,resolution=None):
 try:return frame_pixels(ppm,reader_geometry(trace,contract,anchor),contract,resolution)
 except(AssertionError,ValueError,KeyError,IndexError,TypeError)as exc:return dict(ok=False,errors=[str(exc)])

def status_geometry(trace,expected,mode,contract):
 # Independently compose the actual preceding public statuses and panel.
 layouts=list(re.finditer(r'\[browser.https.layout\] base=(\d+) bytes=64 viewport=(\d+) state=(\w+) pixels=unconfirmed',trace))
 if not layouts or layouts[-1][3]!=expected:raise ValueError('wrong latest controlled peer layout')
 marker=layouts[-1];base,sequence=int(marker[1]),int(marker[2]);previews=re.findall(r'\[browser.preview.layout\] bytes=(\d+) newline=1 cached_or_sample=1',trace)
 if not previews:raise ValueError('missing actual preview extent')
 length=int(previews[-1]);prefix=contract['prefix']
 if base!=length+258 or not len(prefix)<length<=192:raise ValueError('peer extent differs from complete overview')
 queued=re.findall(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={sequence} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace);accepted=re.findall(rf'\[browser.viewport\] target=15 task=15 sequence={sequence} w=(\d+) h=(\d+) origin=held_shell',trace)
 if len(queued)!=1 or not accepted:raise ValueError('missing exact held-shell peer viewport')
 x,y,w,h=map(int,queued[0])
 if(w,h)!=tuple(map(int,accepted[-1])):raise ValueError('peer viewport geometry differs')
 preceding_trace=trace[:marker.end()]
 echo_views=re.findall(r'\[browser.net.job.view\] state=(\w+) web=0',preceding_trace);echo_state=echo_views[-1]if echo_views else'Verified'
 http_views=re.findall(r'\[browser.http.view\] state=(\w+) tls=0',preceding_trace);http_state=http_views[-1]if http_views else'Idle'
 dns_views=re.findall(r'\[browser.dns.view\] state=(Answer\(\[10, 0, 2, 2\]\)|\w+) web=0',preceding_trace);dns_state=dns_views[-1]if dns_views else'Idle'
 http_codes={'Idle':0,'Pending':1,'Ready':2,'Unavailable':3,'Cancelled':4,'Uncertain':5}
 if echo_state not in contract['echo']or http_state not in contract['http']or http_state=='Ready':raise ValueError('unqualified preceding peer overview status')
 dns_label=b'10.0.2.2'if dns_state=='Answer([10, 0, 2, 2])'else contract['dns'].get(dns_state)
 if dns_label is None:raise ValueError('unqualified preceding DNS state')
 if not re.search(rf'\[browser.view.applied\] bytes={base+64} http={http_codes[http_state]} body_bytes=0 pixels=unconfirmed',trace[marker.end():]):raise ValueError('peer layout lacks positive atomic Applied')
 preceding=prefix+b'?'*(length-len(prefix))+contract['echo'][echo_state].ljust(64,b' ')+(contract['dns_prefix']+dns_label+contract['dns_footer']).ljust(64,b' ')+(contract['http_prefix']+contract['http'][http_state]+contract['http_footer']).ljust(64,b' ')+b'\n\n'+b' '*64
 # DNS is public actual state after the peer's positively released query.
 if len(preceding)!=base:raise ValueError('source overview changed')
 label=(b'certificate rejected'if mode=='wrong-name'else b'invalid document')if expected=='Unavailable'else LABELS[expected]
 status=(b'\n\nHTTPS / lab root: '+label+b'\nB fetch V read Q clear').ljust(64,b' ')
 if len(status)!=64:raise ValueError('controlled TLS panel exceeds64bytes')
 data=preceding+status;scale=2 if w>=contract['scale_threshold']else 1;cols=min(80,max(1,(w-2*contract['inset'])//(contract['advance']*scale)));starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:starts.append(index+1);column=0
  else:
   column+=1
   if column>=cols and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('peer status exceeds renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=base:continue
  end=starts[row+1]if row+1<len(starts)else len(data);py=y+contract['top']+row*contract['line']*scale
  if py+contract['line']*scale>y+h:raise ValueError('peer status clipped')
  rows.append(dict(row=row,text=data[start:end].rstrip(b'\n').decode(),x=x+contract['inset'],y=py))
 if not any('HTTPS / lab root:'in row['text']for row in rows):raise ValueError('controlled HTTPS label missing')
 return dict(tile=dict(x=x,y=y,w=w,h=h),base=base,viewport=sequence,scale=scale,columns=cols,rows=rows,status_text=label.decode())

def verify_status_pixels(trace,ppm,expected,mode,contract,resolution=None):
 from check_disk_persistence import ppm_rgb
 from check_desktop_pixels import glyph
 try:
  g=status_geometry(trace,expected,mode,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=g['tile'];scale=g['scale'];color=contract['color'].to_bytes(3,'big');checked=ink=mismatch=0
  if resolution and(width,height)!=tuple(resolution):raise ValueError('wrong framebuffer resolution')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('peer tile outside framebuffer')
  for row in g['rows']:
   for col in range(g['columns']):
    bits=glyph(contract['font'],row['text'][col])if col<len(row['text'])else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<7 and gx<5 and bool(bits[gy]&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        px=row['x']+(col*contract['advance']+gx)*scale+dx;py=row['y']+gy*scale+dy;actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;mismatch+=actual!=on;checked+=1;ink+=on
  if mismatch or not ink:raise ValueError('whole peer glyph/padding/trailing-erasure mismatches='+str(mismatch))
  return dict(ok=True,errors=[],expected=expected,geometry=g,checked_pixels=checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,IndexError,UnicodeError)as exc:return dict(ok=False,errors=[str(exc)],expected=expected)

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--iso',required=True,type=Path);parser.add_argument('--source-disk',required=True,type=Path);parser.add_argument('--binding',required=True,type=Path);parser.add_argument('--out',required=True,type=Path);parser.add_argument('--mode',choices=MODES,default='ready');parser.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');parser.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=parser.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh HTTPS evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));held=json.loads(a.binding.read_text());contract=load_contract(root);target=target_check(root)
 if held['runner_sha256'].get('scripts/https_document_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()or not target['ok']:raise RuntimeError('HTTPS checker/target absent from exact frozen binding')
 peer=ControlledHTTPSPeer(root,a.mode);peer.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-https-{os.getpid()}.sock'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',RTC_NORMAL,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}'];profile=tls.command_profile(command)
 if not profile['ok']:raise RuntimeError(str(profile))
 host_start=time.time();manifest=dict(binding=binding,command=command,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),openssl_version=ssl.OPENSSL_VERSION,mode=a.mode,model=a.model,resolution=a.resolution,platform_profile=profile,target=target,host_utc_start=host_start,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),pixel_source_sha256=contract['source_sha256'],request_hex=REQUEST.hex(),body_sha256=hashlib.sha256(BODY).hexdigest(),body_bytes=len(BODY),public_test_fixture=True,keylog_enabled=False)
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;pixels=[];actions=[];cleanup=None;resolution=tuple(map(int,a.resolution.split('x')))
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>300:raise TimeoutError('bounded300second HTTPS document gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise RuntimeError('raw HTTPS evidence exceeded bound')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.01)
  raise TimeoutError(pattern)
 def event(event,name,timeout=30):
  deadline=time.monotonic()+timeout
  while not event.wait(.01):
   health()
   if time.monotonic()>deadline:raise TimeoutError(name)
 def key(code,pattern):
  boundary=len(text());q.key(code);return wait(pattern,boundary)
 def picture(name,kind,state=None,anchor=0):
  path=out/(name+'.ppm');deadline=time.monotonic()+25;result=dict(ok=False,errors=['no frame']);snapshot=''
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=text();result=verify_reader_pixels(snapshot,path.read_bytes(),contract,anchor,resolution)if kind=='reader'else verify_status_pixels(snapshot,path.read_bytes(),state,a.mode,contract,resolution)
   if result['ok']:break
   time.sleep(.1)
  result.update(screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256'],kind=kind,expected=state,anchor=anchor,serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest(),requested_anchor=anchor if kind=='reader'else None);pixels.append(result);(out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n')
  if not result['ok']:raise RuntimeError(str(result['errors']))
  return result
 try:
  deadline=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,text,key);key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
  boundary=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',boundary);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',boundary)
  boundary=len(text());q.key('b');wait(r'\[shell.browser.input\] code=48 down=1 slot=43 accepted=1',boundary);wait(r'\[browser.https.intent\] accepted=1 explicit=1 public_internet=0',boundary)
  if a.mode=='ready':
   contract['reader_require_position']=True
   wait(r'\[browser.https.view\] state=Verified controlled_profile=1 public_internet=0',boundary);event(peer.completed,'actual SSL HTTP response and guest close_notify');initial=picture('https-reader-initial','reader');anchor=0;network_boundary=len(text())
   def navigate(action):
    nonlocal anchor
    frame=reader_geometry(text(),contract,anchor);anchor=http.reader_move(frame,action);q.key(action);shot=picture('https-reader-'+action,'reader',anchor=anchor);actions.append(dict(action='actual_'+action,anchor=anchor,first=shot['geometry']['first'],end=shot['geometry']['end']))
    if action=='end'and(TAIL.decode()not in ''.join(shot['expected_rows'][2:-1])or shot['geometry']['end']!=shot['geometry']['total_lines']):raise RuntimeError('HTTPS End omitted unique final document byte')
    return shot
   navigate('pgdn')
   if anchor==0:raise RuntimeError('HTTPS resume fixture never left Home')
   retained=anchor;key('v',r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1');picture('https-overview','status','Verified');key('v',r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1');resumed=picture('https-reader-reopened','reader',anchor=anchor)
   if resumed['geometry']['source_anchor']!=retained:raise RuntimeError('HTTPS overview/reopen lost requested source anchor')
   original=resumed['geometry']['tile'];prior_viewport=resumed['geometry']['viewport']
   for name in('https-reader-restored-tile','https-reader-zoom-roundtrip'):
    key('grave_accent',r'\[shell.palette.item\] idx=0')
    for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
    start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',start);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
    shot=picture(name,'reader',anchor=anchor)
    if shot['geometry']['source_anchor']!=retained:raise RuntimeError('HTTPS Zoom/restoration changed requested source anchor')
    if shot['geometry']['viewport']<=prior_viewport:raise RuntimeError('HTTPS Zoom did not advance actual held-shell viewport')
    if name=='https-reader-restored-tile'and shot['geometry']['tile']==original:raise RuntimeError('HTTPS Zoom did not change actual geometry')
    prior_viewport=shot['geometry']['viewport']
    actions.append(dict(action=name,anchor=anchor,tile=shot['geometry']['tile']))
   if shot['geometry']['tile']!=original:raise RuntimeError('HTTPS Zoom roundtrip did not restore actual geometry')
   navigate('end');navigate('home')
   key('q',r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1');wait(r'\[browser.https.cleared\] body_visible=0 owned_storage=erased network=0');picture('https-cleared','status','Cancelled');layouts=len(re.findall(r'\[browser.reader.layout\]',text()));key('v',r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1');picture('https-cannot-reopen','status','Cancelled')
   if len(re.findall(r'\[browser.reader.layout\]',text()))!=layouts or re.search(r'\[net\.(?:job|dns)\.egress\.submit\]|\[net.job.resolved.admitted\]',text()[network_boundary:]):raise RuntimeError('reader navigation issued network work or cleared document reopened')
   actions.append(dict(action='positive_released_document_local_navigation_clear',body_bytes=len(BODY),cancelled_document_cannot_reopen=True))
  elif a.mode.startswith('partial-'):
   event(peer.partial_sent,'actual authenticated partial HTTP body sent');wait(r'\[browser.https.progress\] ticket=2 body_received=1024 authenticated=1 body_visible=0',boundary);picture('https-partial','status','Pending');boundary=len(text())
   if a.mode=='partial-cancel':key('q',r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1');wait(r'\[browser.https.view\] state=Cancelled controlled_profile=1 public_internet=0',boundary);picture('https-cancelled','status','Cancelled')
   else:
    q.key_sequence(['f11']);wait(r'\[browser.https.closed\] pending=1 terminal=1 owned_storage=erased body_visible=0',boundary);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',boundary);cleanup=cleanup_receipts(text(),15,15,boundary);q.cmd('screendump',{'filename':str(out/'https-closed.ppm')})
   actions.append(dict(action='actual_Q_partial_body'if a.mode=='partial-cancel'else'actual_F11_partial_body',body_received=1024,body_visible=0))
  else:
   wait(r'\[browser.https.view\] state=Unavailable controlled_profile=1 public_internet=0',boundary)
   event(peer.flight_sent if a.mode=='wrong-name'else peer.response_sent,'actual negative HTTPS fixture sent');picture('https-unavailable','status','Unavailable');actions.append(dict(action='actual_B_rejected_document',body_visible=0,authenticated=a.mode!='wrong-name'))
  time.sleep(.25);health()
 except Exception as exc:failure=repr(exc)
 finally:
  host_end=time.time();peer.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 manifest['host_utc_end']=host_end;(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=inspect_wire(raw,peer.rows,a.mode);dns=dns_capture(raw,'echo',attempts=peer.attempts,expected=peer.dns)if peer.dns else dict(ok=False,errors=['actual controlled DNS absent']);document=inspect_document(trace,a.mode,wire,dns,raw,host_start,host_end,contract);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 (out/'peer-exchanges.json').write_text(json.dumps(dict(tls=peer.rows,dns=peer.dns,dns_send_attempts=peer.attempts,keylog_enabled=False,scope='Actual endpoint public HTTP fixture observations; no private TLS or entropy material.'),indent=2)+'\n');result=dict(ok=failure is None and not peer.errors and all(r['ok']for r in(wire,dns,document,authority,waiting)),error=failure,server_errors=peer.errors,mode=a.mode,binding=binding,wire=wire,dns=dns,document=document,authority=authority,self_wait=waiting,pixels=pixels,actions=actions,cleanup=cleanup,scope=SCOPE);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,document=document['joins'])))

if __name__=='__main__':main()
