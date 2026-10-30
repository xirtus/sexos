#!/usr/bin/env python3
"""Finite one-incarnation TLS/HTTPS command lifecycle and independent wire joins."""
import argparse,hashlib,json,math,os,re,shutil,socket,ssl,subprocess,threading,time
from pathlib import Path
import https_document_gate as https
import tls_exchange_gate as tls
import tls_prepare_gate as preparation
import http_content_gate as http
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from certificate_time_gate import RTC_NORMAL
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from check_dns_capture import inspect as dns_capture
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_nonunix_target import inspect as target_check

# Seven actual TLS connections and two local preparations, all one incarnation.
PLAN=('full','full','full','partial','full','partial','partial')
FULL=(1,2,3,5);PARTIAL=(4,6,7)
TLS_TICKETS=(2,3,4,5,6,7,9)
DNS_GENERATIONS=(1,2,3,4,5,6,8)
PLAIN_BODY=b'PLAIN HTTP AFTER PRIVATE TLS / DOCUMENT H BYTE Z'
MAX_CONNECTIONS=8
SCOPE='Finite same-incarnation replacement, clear/refetch, private preparation, plain HTTP supersession and final Close under held BOOT authority. Exact endpoint OpenSSL, fresh source receipts, independently reassembled wire and distinct document glyphs are separate requirements. No proxy/keylog, seed/private TLS capture, publicInternet/publicCA, transport-EOF inference, physical/SMP or general desktop acceptance claim.'

def body(attempt):
 if not 1<=attempt<=len(PLAN):raise ValueError('unknown finite lifecycle document')
 tail=f'FINAL LIFECYCLE HTTPS DOCUMENT {attempt:02d} BYTE Z'.encode();text=b''.join(f'{line:03d} / LIFECYCLE HTTPS DOCUMENT {attempt:02d} / held authority; source bytes are data.\n'.encode()for line in range(80));return text[:3072-len(tail)-1]+b'\n'+tail

def response(attempt):
 content=body(attempt);sent=content if PLAN[attempt-1]=='full'else content[:1024]
 return b'HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: '+str(len(content)).encode()+b'\r\nConnection: close\r\n\r\n'+sent

def plain_response():return b'HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: '+str(len(PLAIN_BODY)).encode()+b'\r\nConnection: close\r\n\r\n'+PLAIN_BODY

class LifecyclePeer(tls.ControlledPeer):
 """Actual fixed endpoint with bounded per-connection handlers, no forwarding."""
 def __init__(self,root,plain_request):
  super().__init__(root,'lifecycle');self.plain_request=plain_request;self.public_rows=[];self.sent={n:threading.Event()for n in range(1,8)};self.finished={n:threading.Event()for n in range(1,8)}
 def serve_echo(self,listener):
  for index in range(2):
   sock=self.accept(listener)
   if sock is None:return
   wanted=tls.ECHO if index==0 else self.plain_request;data=bytearray()
   while len(data)<len(wanted):
    chunk=sock.recv(4096)
    if not chunk:raise ValueError('short actual public startup/HTTP request')
    data.extend(chunk)
    if len(data)>len(wanted):raise ValueError('extra public startup/HTTP request bytes')
   if bytes(data)!=wanted:raise ValueError('foreign public request or unexpected plain HTTP ordering')
   reply=tls.ECHO if index==0 else plain_response();sock.sendall(reply);sock.shutdown(socket.SHUT_WR)
   if sock.recv(1):raise ValueError('extra public request tail')
   self.public_rows.append(dict(index=index+1,payload_hex=wanted.hex(),response_hex=reply.hex(),request_bytes=len(wanted),response_bytes=len(reply),finished=True));sock.close()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('extra public startup/HTTP connection')
   except socket.timeout:pass
 def serve_dns(self,sock):
  while not self.stop.is_set():
   try:data,remote=sock.recvfrom(513)
   except socket.timeout:continue
   ticket=int.from_bytes(data[:2],'big')if len(data)>=2 else 0
   if data!=tls.dns_query(ticket)or ticket!=len(self.dns)+1 or not 1<=ticket<=8:raise ValueError('extra/duplicate/foreign lifecycle DNS query')
   reply=tls.dns_response(ticket);sent=sock.sendto(reply,remote);self.dns.append(dict(ticket=ticket,response_hex=reply.hex()));self.attempts.append(dict(ticket=ticket,kind='answer',response_hex=reply.hex(),sent_bytes=sent))
 def serve_tls(self,listener,ctx):
  def guarded(sock,attempt):
   try:self.connection(sock,ctx,attempt)
   except Exception as exc:
    if not self.stop.is_set():self.errors.append(type(exc).__name__+': '+str(exc))
  while not self.stop.is_set():
   sock=self.accept(listener)
   if sock is None:return
   attempt=len(self.rows)+1
   if attempt>len(PLAN):sock.close();raise ValueError('extra lifecycle TLS connection')
   row=dict(attempt=attempt,kind=PLAN[attempt-1],handshake_complete=False,sni=None,tls_version=None,cipher=None,request_hex=None,response_hex=None,application_bytes_received=0,extra_application_bytes=0,unwrap_completed=False);self.rows.append(row)
   thread=threading.Thread(target=guarded,args=(sock,attempt),daemon=True);self.threads.append(thread);thread.start()
 def connection(self,sock,ctx,attempt):
  row=self.rows[attempt-1];wrapped=ctx.wrap_socket(sock,server_side=True,do_handshake_on_connect=False);self.live.append(wrapped);wrapped.do_handshake();row.update(handshake_complete=True,sni=getattr(wrapped,'controlled_sni',None),tls_version=wrapped.version(),cipher=wrapped.cipher()[0])
  if(row['tls_version'],row['cipher'],row['sni'])!=('TLSv1.3',tls.CIPHER,tls.HOST):raise ValueError('foreign actual lifecycle TLS profile')
  request=bytearray()
  while len(request)<len(https.REQUEST):
   chunk=wrapped.recv(4096)
   if not chunk:raise ValueError('short actual lifecycle GET')
   request.extend(chunk)
   if len(request)>len(https.REQUEST):raise ValueError('extra lifecycle GET bytes')
  if bytes(request)!=https.REQUEST:raise ValueError('foreign lifecycle HTTPS request')
  wire=response(attempt)
  for offset in range(0,len(wire),1024):wrapped.sendall(wire[offset:offset+1024])
  row.update(request_hex=bytes(request).hex(),response_hex=wire.hex(),application_bytes_received=len(request));self.sent[attempt].set()
  if PLAN[attempt-1]=='full':
   plain=wrapped.unwrap();row['unwrap_completed']=True;plain.close();self.finished[attempt].set();return
  wrapped.settimeout(.1)
  while not self.stop.is_set():
   try:chunk=wrapped.recv(4096)
   except socket.timeout:continue
   except(ssl.SSLZeroReturnError,ssl.SSLEOFError,ConnectionResetError):break
   if not chunk:break
   row['extra_application_bytes']+=len(chunk);raise ValueError('partial lifecycle endpoint received extra application bytes')
  wrapped.close()

def inspect_wire(raw,rows,public_rows,plain_request):
 errors=[];joins=[]
 try:
  flows,packets=tls.reassemble(raw);private=[f for f in flows if f['peer_port']==46080];public=[f for f in flows if f['peer_port']==46078]
  assert len(flows)==9 and len(private)==len(rows)==7 and len(private)<=MAX_CONNECTIONS and len(public)==len(public_rows)==2,'lifecycle flow counts differ from finite 7TLS/2public plan'
  assert [f['guest_port']-48000 for f in private]==list(TLS_TICKETS),'TLS route tickets missing/reused/foreign'
  assert [f['guest_port']-48000 for f in public]==[1,8],'startup/plain HTTP route tickets differ'
  for index,(flow,row)in enumerate(zip(public,public_rows)):
   request=tls.ECHO if index==0 else plain_request;reply=tls.ECHO if index==0 else plain_response()
   assert row.get('payload_hex')==request.hex()and row.get('response_hex')==reply.hex()and row.get('finished')is True,'actual plain endpoint observation differs'
   assert flow['outgoing']==request and flow['incoming']==reply and flow['fin_out']and flow['fin_in'],'actual plain TCP bytes/FIN differ'
  for attempt,(flow,row)in enumerate(zip(private,rows),1):
   assert row.get('attempt')==attempt and row.get('kind')==PLAN[attempt-1],'lifecycle endpoint reordered or selected foreign body'
   outgoing=tls.tls_records(flow['outgoing']);incoming=tls.tls_records(flow['incoming']);client=tls.hello(outgoing[0],True);server=tls.hello(incoming[0],False)
   assert(row.get('tls_version'),row.get('cipher'),row.get('sni'))==('TLSv1.3',tls.CIPHER,tls.HOST)and row.get('handshake_complete')is True,'actual lifecycle SSL handshake absent'
   assert row.get('request_hex')==https.REQUEST.hex()and row.get('response_hex')==response(attempt).hex()and row.get('application_bytes_received')==94 and row.get('extra_application_bytes')==0,'actual lifecycle HTTP request/response differs'
   assert sum(r['length']for r in outgoing if r['type']==23)>=len(https.REQUEST)+17 and sum(r['length']for r in incoming if r['type']==23)>=len(response(attempt))+17,'lifecycle encrypted application extent absent'
   for clear in(https.REQUEST,b'HTTP/1.1 200 OK',body(attempt)[:64],f'FINAL LIFECYCLE HTTPS DOCUMENT {attempt:02d} BYTE Z'.encode()):assert clear not in flow['incoming']and clear not in flow['outgoing'],'lifecycle HTTP plaintext escaped encryption'
   if attempt in FULL:assert row.get('unwrap_completed')is True and flow['fin_out']and flow['fin_in']and not flow['rst_out']and not flow['rst_in'],'full lifecycle response lacks close_notify/clean transport completion'
   else:assert row.get('unwrap_completed')is False,'partial lifecycle response fabricated completion'
   joins.append(dict(attempt=attempt,kind=PLAN[attempt-1],ticket=TLS_TICKETS[attempt-1],dns_generation=DNS_GENERATIONS[attempt-1],syn=flow['syn'],client_hello=client,server_hello=server,initial_tls_bytes=outgoing[0]['length']+5,tx_bytes=len(flow['outgoing']),rx_bytes=len(flow['incoming']),body_bytes=len(body(attempt))if attempt in FULL else 0,body_hash=hex(tls.fingerprint(body(attempt)))if attempt in FULL else None))
 except(AssertionError,ValueError,TypeError,KeyError,IndexError)as exc:errors.append(str(exc));packets=locals().get('packets',0)
 return dict(ok=not errors,errors=errors,joins=joins,tcp_packets=packets,scope='Independent checksum/TCP flow reassembly, exact route tickets, actual endpoint SSL HTTP observations and Hello/ciphertext bounds for seven distinct TLS attempts plus startup/plain HTTP. Local L preparations may add no TCP flow. No negative FIN/EOF or serial-only authentication claim.')

def load_contract(root):
 root=Path(root);contract=https.load_contract(root);contract['plain_request']=http.load_pixel_contract(root)['request'];app=(root/'apps/kaleidoscope/src/main.rs').read_text()
 if 'ALLOCATOR.usage()'not in app or 'self.tls.allocated_payload_bytes()+self.exchange.allocated_payload_bytes()'not in app or '[browser.tls.cleanup] ticket={} outcome={:?}'not in app:raise ValueError('source-bound actual allocator/payload/cleanup receipt producer changed')
 for name in('scripts/tls_lifecycle_gate.py','apps/kaleidoscope/src/browser_command.rs','sex-rt/src/heap.rs','apps/kaleidoscope/src/tls_probe.rs'):
  contract['source_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 return contract

def resources(trace):
 matches=list(re.finditer(r'\[browser.resources\] reason=(command|change|clear|close) heap_used=(\d+) heap_free=(\d+) heap_total=(\d+) tls_payload=(\d+) tls_pending=([01]) route_busy=([01]) providers_busy=([01]) queued=(None|Some\((?:Echo|Dns|HttpDns|Http|TlsDns|TlsStart|TlsPrepare|Seed|Clock)\)) body_visible=([01])',trace));assert len(matches)==trace.count('[browser.resources]'),'foreign/malformed resource snapshot'
 rows=[]
 for m in matches:
  reason,used,free,total,payload,tls_pending,route,providers,queued,visible=m.groups();used,free,total,payload=map(int,(used,free,total,payload));assert used+free==total==65536 and 0<=payload<=used and payload<49152,'actual allocator/payload accounting bound differs'
  if reason in('command','clear','close'):assert used==payload==0 and visible=='0','supersession/clear/close retained private TLS/document allocation'
  rows.append(dict(reason=reason,used=used,free=free,total=total,payload=payload,tls_pending=int(tls_pending),route_busy=int(route),providers_busy=int(providers),queued=queued,body_visible=int(visible),offset=m.start(),end=m.end()))
 return rows

def prerequisites(trace,contract,host_start,host_end):
 """Nine distinct source cycles: six B, two local L, final B."""
 starts=list(re.finditer(r'\[browser.command.started\] kind=(TlsDns|TlsPrepare) accepted=1',trace));assert len(starts)==trace.count('[browser.command.started]')==9 and [m[1]for m in starts]==['TlsDns']*6+['TlsPrepare']*2+['TlsDns'],'missing/duplicated/failed queued TLS command start'
 policies=list(re.finditer(r'\[trust.policy.snapshot\] owner=15 task=15 seq=(\d+) profile=1 root_bytes=376 origin=held_boot_grant',trace));sources=list(re.finditer(r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=(\d+) attempts=([1-4]) precision=1 profile=q35_cmos_utc ordinal=(\d+)',trace));clocks=list(re.finditer(r'\[certificate.time.snapshot\] owner=15 task=15 seq=(\d+) seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace));seeds=list(re.finditer(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=(\d+) bytes=32 source=held_boot_grant',trace));shared=list(re.finditer(r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0',trace));checks=list(re.finditer(r'\[browser.trust.self_check\] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',trace))
 for prefix,values in(('[trust.policy.snapshot]',policies),('[clock.source.snapshot]',sources),('[certificate.time.snapshot]',clocks),('[entropy.seed.admitted]',seeds),('[browser.tls.prepared]',shared),('[browser.trust.self_check]',checks)):assert len(values)==trace.count(prefix)==9,'source/preparation count differs from exact nine fresh cycles: '+prefix
 assert [int(m[1])for m in policies]==[int(m[1])for m in clocks]==[int(m[1])for m in seeds]==[int(m[3])for m in sources]==list(range(1,10)),'source sequence/ordinal reused, skipped or reordered'
 rows=[];previous_utc=0
 for index,(start,policy,source,clock,seed,prepared,check)in enumerate(zip(starts,policies,sources,clocks,seeds,shared,checks),1):
  task,private,initial,utc=map(int,prepared.groups());document=index not in(7,8);assert 0<task<=32768 and private==task+contract['pipe_bytes']+(12288 if document else 0)and private<49152 and 0<initial<=4096,'source-bound private preparation extent differs'
  assert utc==int(source[1])==int(clock[2])and previous_utc<=utc and math.floor(host_start)-1<=utc<=math.ceil(host_end)+1,'fresh UTC ordinal/source/service/host bracket differs';previous_utc=utc
  produced=re.search(rf'\[entropy.seed.progress\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={index} state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace);released=re.search(rf'\[entropy.seed.released\] owner=15 task=15 ticket={index} service_payload=erased receipt=retained',trace)
  assert produced and released and start.start()<policy.start()<source.start()<clock.start()<prepared.start()and policy.start()<seed.start()<produced.start()<released.start()<check.start()<prepared.start(),'fresh held source cycle ordering/positive seed cleanup differs'
  rows.append(dict(cycle=index,document=document,task_bytes=task,private_bytes=private,initial_tls_bytes=initial,utc_snapshot=utc,started_offset=start.start(),prepared_offset=prepared.start()))
 assert trace.count('[entropy.seed.released]')==9,'extra/foreign seed release'
 return rows

def reader_geometry(trace,attempt,contract,anchor=0):
 ticket=TLS_TICKETS[attempt-1];expected=body(attempt);releases=list(re.finditer(r'\[browser.https.released\] ticket=(\d+) tx=\d+ rx=\d+ request_bytes=94 body_bytes=(\d+) body_hash=(0x[0-9a-f]+) positive_release=1 authenticated=1 public_test_root=1',trace));assert releases,'no positively released lifecycle document';release=releases[-1]
 assert(int(release[1]),int(release[2]),release[3])==(ticket,len(expected),hex(tls.fingerprint(expected))),'reader points to an old/foreign lifecycle document'
 assert not re.search(r'\[browser.command.queued\]|\[browser.https.cleared\]|\[browser.https.closed\]|\[browser.resources\] reason=(?:command|clear|close) ',trace[release.end():]),'released lifecycle reader superseded or cleared'
 tile,sequence=https.viewport(trace);frame=http.reader_frame(expected,tile['w'],tile['h'],contract,anchor,b'/welcome');frame.update(tile=tile,viewport=sequence,document_ticket=ticket,attempt=attempt)
 layouts=list(re.finditer(r'\[browser.reader.layout\] first=(\d+) end=(\d+) body_rows=(\d+) rows=(\d+) cols=(\d+) total_lines=(\d+) source_bytes=(\d+) viewport=(\d+) origin=authenticated_body tls=1 public_test_root=1 pixels=unconfirmed',trace));wanted=tuple(frame[k]for k in('first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'));assert layouts and layouts[-1].start()>release.end()and tuple(map(int,layouts[-1].groups()))==wanted,'lifecycle reader geometry/navigation differs'
 assert re.search(rf'\[browser.view.applied\] bytes={frame["bytes"]} http=770 body_bytes={len(expected)} pixels=unconfirmed',trace[layouts[-1].end():]),'lifecycle reader lacks positive atomic Applied'
 return frame

def verify_reader_pixels(trace,ppm,attempt,contract,anchor=0,resolution=None):
 try:return https.frame_pixels(ppm,reader_geometry(trace,attempt,contract,anchor),contract,resolution)
 except(AssertionError,ValueError,TypeError,KeyError,IndexError)as exc:return dict(ok=False,errors=[str(exc)])

def mixed_drains(trace,raw):
 """Validate every destructive READ sample against its actual admitted kind."""
 errors=[];admitted={};resolved={};samples=[]
 try:
  flows,_=tls.reassemble(raw);captured={f['guest_port']-48000:f for f in flows}
  declarations=list(re.finditer(r'\[net.job\.([a-z_]+)\.admitted\][^\r\n]*',trace));assert len(declarations)==len(re.findall(r'\[net.job\.[^\]\r\n]*\.admitted',trace)),'malformed admitted kind marker'
  for line in declarations:
   kind=line[1]
   if kind=='resolved':
    match=re.fullmatch(r'\[net.job.resolved.admitted\] owner=15 task=15 ticket=(\d+) dns_generation=(\d+) observed=(\d+) expires=(\d+) now=(\d+) fixed=1 at_begin=1',line[0]);assert match,'malformed/foreign resolved admission';ticket,generation,observed,expires,now=map(int,match.groups());assert ticket not in resolved and 2<=ticket<=9 and generation==ticket-1 and observed<=now<expires and 0<expires-observed<=60000,'duplicate/foreign/expired resolved admission';resolved[ticket]=line.start();continue
   assert kind in('stream','duplex'),'unknown admitted network kind'
   if kind=='stream':
    match=re.fullmatch(r'\[net.job.stream.admitted\] owner=15 task=15 ticket=(\d+) kind=9 ring=1024 max_wire=13312 total_ticks=30000 idle_ticks=5000 authority=held_route',line[0]);maximum=13312;port=46078;expected_kind=9
   else:
    match=re.fullmatch(r'\[net.job.duplex.admitted\] owner=15 task=15 ticket=(\d+) kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route',line[0]);maximum=32768;port=46080;expected_kind=10
   assert match,'malformed/foreign admitted kind/owner/ring/wire policy';ticket=int(match[1]);assert ticket not in admitted,'duplicate/reused admitted mixed-flow ticket'
   assert ticket in resolved and resolved[ticket]<line.start(),'data admission lacks prior exact resolved ticket'
   assert ticket in captured and captured[ticket]['peer_port']==port,'admitted protocol differs from actual captured endpoint'
   admitted[ticket]=dict(kind=expected_kind,maximum=maximum,offset=line.start(),captured_rx=len(captured[ticket]['incoming']))
  assert set(resolved)==set(admitted)==set(TLS_TICKETS)|{8}and admitted[8]['kind']==9 and all(admitted[t]['kind']==10 for t in TLS_TICKETS),'mixed admitted ticket/kind set differs from lifecycle plan'
  previous={};complete=set()
  lines=list(re.finditer(r'\[net.job.stream.drain\][^\r\n]*',trace));assert len(lines)==trace.count('[net.job.stream.drain'),'malformed drain marker';terminal=re.search(r'\[net.app.cleanup.ack\] target=15 task=15 client_leases=0 shared_cache=retained ok=1',trace)
  for line in lines:
   match=re.fullmatch(r'\[net.job.stream.drain\] owner=15 task=15 ticket=(\d+) produced=(\d+) consumed=(\d+) buffered=(\d+) ring=1024 state_before=(Receiving|Complete) receipt=retained not_delivery=1',line[0]);assert match,'malformed/foreign mixed drain owner/ring/state/receipt'
   ticket,produced,consumed,buffered=map(int,match.groups()[:4]);state=match[5];assert ticket in admitted,'drain has no exact admitted ticket/kind';policy=admitted[ticket]
   assert line.start()>policy['offset'],'drain precedes exact kind admission'
   assert not terminal or line.start()<terminal.start(),'retired incarnation emitted late destructive READ receipt'
   assert 0<consumed<=produced<=min(policy['maximum'],policy['captured_rx'])and buffered==produced-consumed and buffered<=1024,'drain cursor/ring/captured-wire bound differs'
   old=previous.get(ticket)
   assert old is None or consumed>old[1]and produced>=old[0],'sampled READ cursor repeated or regressed'
   assert ticket not in complete or state=='Complete','drain state regressed after Complete'
   if state=='Complete':complete.add(ticket)
   previous[ticket]=(produced,consumed);samples.append(dict(ticket=ticket,kind=policy['kind'],produced=produced,consumed=consumed,buffered=buffered,state=state,offset=line.start(),captured_rx=policy['captured_rx'],maximum=policy['maximum']))
  assert set(previous)==set(admitted),'admitted lifecycle flow lacks an actual destructive READ sample'
 except(AssertionError,ValueError,KeyError,IndexError,TypeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,admitted=admitted,samples=samples,scope='Every shared stream.drain sample is validated without filtering: exact prior owner/task/ticket/kind, source wire/ring policy, monotonic destructive READ cursors, captured incoming-byte bound and retained receipt semantics. Kind9 and kind10 remain distinct; sampled consumption is not delivery or TLS authentication.')

def inspect_lifecycle(trace,wire,dns,raw,host_start,host_end,contract):
 errors=[];joined={}
 try:
  assert wire['ok']and dns['ok'],'independent lifecycle TLS/DNS evidence absent'
  for marker in('[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1'):assert trace.count(marker)==trace.count(marker.split(']')[0]+']')==1,'missing/foreign BOOT authority or raw-source denial'
  assert not re.search(r'\b(?:keylog|traffic_secret|master_secret|seed_bytes|seed_value|secret)=',trace),'private TLS/seed material captured'
  samples=resources(trace);cycles=prerequisites(trace,contract,host_start,host_end);queued=list(re.finditer(r'\[browser.command.queued\] kind=(TlsDns|TlsPrepare|HttpDns) document=([01]) retained=1 accepted_replay=0',trace));expected=[('TlsDns','1')]*6+[('TlsPrepare','0')]*2+[('HttpDns','0'),('TlsDns','1')]
  assert len(queued)==trace.count('[browser.command.queued]')==10 and[m.groups()for m in queued]==expected,'queued lifecycle commands lost, replayed or reordered'
  for command in queued:
   snapshot=next((s for s in samples if s['offset']>command.start()and s['reason']=='command'),None);assert snapshot and snapshot['queued']=='Some('+command[1]+')'and snapshot['used']==snapshot['payload']==snapshot['body_visible']==snapshot['tls_pending']==0,'queued command failed to erase old private payload before replacement'
  resolution=http.resolution_joins(trace,raw,[(ticket,ticket-1)for ticket in range(2,10)],dns);assert resolution['ok'],resolution['errors']
  admissions=list(re.finditer(r'\[browser.https.admitted\] ticket=(\d+) dns_generation=(\d+) seq=(\d+) task_bytes=(\d+) private_bytes=(\d+) profile=1 authority=held_route',trace));assert len(admissions)==trace.count('[browser.https.admitted]')==7 and[(int(m[1]),int(m[2]))for m in admissions]==list(zip(TLS_TICKETS,DNS_GENERATIONS)),'lifecycle admission ticket/DNS mapping differs'
  sequences=[int(m[3])for m in admissions];assert all(0<a<b<=0xffffff for a,b in zip(sequences,sequences[1:])),'held route admission fence reused or regressed'
  source_doc=[c for c in cycles if c['document']];doc_preps=list(re.finditer(r'\[browser.https.prepared\] task_bytes=(\d+) private_bytes=(\d+) tx_bytes=(\d+) utc_snapshot=(\d+) request_bytes=94 document_limit=12288 network=0',trace));assert len(doc_preps)==trace.count('[browser.https.prepared]')==7,'document source preparation count differs'
  services=list(re.finditer(r'\[net.job.duplex.admitted\] owner=15 task=15 ticket=(\d+) kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route',trace));assert len(services)==trace.count('[net.job.duplex.admitted]')==7 and[int(s[1])for s in services]==list(TLS_TICKETS),'actual duplex service admissions differ'
  auth=list(re.finditer(r'\[browser.https.authenticated\] ticket=(\d+) profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1',trace));assert len(auth)==trace.count('[browser.https.authenticated]')==7 and[int(a[1])for a in auth]==list(TLS_TICKETS),'actual lifecycle authentication receipt missing/foreign'
  released=list(re.finditer(r'\[browser.https.released\] ticket=(\d+) tx=(\d+) rx=(\d+) request_bytes=94 body_bytes=(\d+) body_hash=(0x[0-9a-f]+) positive_release=1 authenticated=1 public_test_root=1',trace));bodies=list(re.finditer(r'\[browser.https.body\] ticket=(\d+) bytes=(\d+) hash=(0x[0-9a-f]+) parsed=1 authenticated=1 visible=0',trace));assert len(released)==trace.count('[browser.https.released]')==len(bodies)==trace.count('[browser.https.body]')==4 and[int(m[1])for m in released]==[TLS_TICKETS[n-1]for n in FULL],'partial/stale lifecycle body published'
  clean=list(re.finditer(r'\[browser.tls.cleanup\] ticket=(\d+) outcome=(Verified|Cancelled|Unavailable) authenticated=([01]) document=([01]) tls_payload=(\d+) positive_release=1',trace));assert len(clean)==trace.count('[browser.tls.cleanup]')==6 and[int(m[1])for m in clean]==list(TLS_TICKETS[:-1]),'missing/foreign/duplicate actual positive old-route RELEASE'
  for attempt,(admission,source,prep,service,authenticated,flow)in enumerate(zip(admissions,source_doc,doc_preps,services,auth,wire['joins']),1):
   ticket=TLS_TICKETS[attempt-1];assert tuple(map(int,prep.groups()))==(source['task_bytes'],source['private_bytes'],source['initial_tls_bytes'],source['utc_snapshot'])and tuple(map(int,admission.groups()[3:]))==(source['task_bytes'],source['private_bytes']),'source/document/admission private metadata mismatch'
   assert source['initial_tls_bytes']==flow['initial_tls_bytes']and source['prepared_offset']<service.start()<admission.start()<authenticated.start(),'fresh preparation/admission/authentication order differs'
   progress=list(re.finditer(rf'\[browser.https.progress\] ticket={ticket} body_received=(\d+) authenticated=1 body_visible=0',trace));assert progress and max(int(p[1])for p in progress)==(3072 if attempt in FULL else 1024)and all(authenticated.start()<p.start()and 0<int(p[1])<=3072 for p in progress),'lifecycle receive extent was visible/foreign or did not reach controlled prefix'
   if attempt in FULL:
    full_index=FULL.index(attempt);positive=released[full_index];parsed=bodies[full_index];cleanup=next(m for m in clean if int(m[1])==ticket);digest=hex(tls.fingerprint(body(attempt)))
    assert tuple(map(int,positive.groups()[:4]))==(ticket,flow['tx_bytes'],flow['rx_bytes'],3072)and positive[5]==digest and(int(parsed[1]),int(parsed[2]),parsed[3])==(ticket,3072,digest),'current lifecycle body/hash/ciphertext cursors differ from independent endpoint'
    assert authenticated.start()<parsed.start()<cleanup.start()<positive.start()and cleanup.groups()[1:]==('Verified','1','1','12288'),'full body published before actual exact RELEASE or future not erased'
    next_command=next((q.start()for q in queued if q.start()>positive.start()),len(trace));assert any(cleanup.start()<s['offset']<next_command and s['body_visible']==1 and s['payload']==12288 and s['route_busy']==s['tls_pending']==s['providers_busy']==0 for s in samples),'released body retains old future/route/provider storage'
   elif attempt in(4,6):
    cleanup=next(m for m in clean if int(m[1])==ticket);next_cycle=cycles[4 if attempt==4 else 6];assert cleanup.groups()[1:]==('Cancelled','1','1','0')and progress[-1].start()<cleanup.start()<next_cycle['started_offset'],'replacement started before correlated old partial positive RELEASE/erasure'
    assert not re.search(rf'\[net.job.egress.submit\] ticket={ticket} ',trace[cleanup.end():]),'superseded ticket emitted new owned egress after RELEASE'
  clears=[s for s in samples if s['reason']=='clear'];assert len(clears)==2 and all(s['used']==s['payload']==s['body_visible']==s['tls_pending']==0 for s in clears),'Q did not erase retained/private preparation storage'
  for cycle,end in((cycles[6],clears[1]['offset']),(cycles[7],queued[8].start())):
   assert any(cycle['prepared_offset']<s['offset']<end and s['payload']==cycle['task_bytes']+8192 and s['tls_pending']==1 and s['route_busy']==s['providers_busy']==s['body_visible']==0 and s['queued']=='None'for s in samples),'private L did not own only fresh bounded local task/queues'
   assert not re.search(r'\[net.job.(?:duplex|stream|resolved).admitted\]|\[browser.https.admitted\]',trace[cycle['prepared_offset']:end]),'private L admitted new network work'
  plain_release=re.search(r'\[browser.http.released\] ticket=8 view=Ready private_job=0',trace);assert plain_release and queued[8].start()<plain_release.start()<queued[9].start(),'L→H lost real plain HTTP result';stream=mixed_drains(trace,raw);assert stream['ok'],stream['errors']
  closed=re.search(r'\[browser.https.closed\] pending=1 terminal=1 owned_storage=erased body_visible=0',trace);assert closed,'final partial TLS Close missing';local_close=[s for s in samples if s['reason']=='close'];assert len(local_close)==1 and closed.start()<local_close[0]['offset']and local_close[0]['used']==local_close[0]['payload']==local_close[0]['body_visible']==0,'final local ownership not zero before task exit';cleanup=cleanup_receipts(trace,15,15,closed.start())
  for marker in('[trust.policy.retired] target=15 task=15 before_cleanup_ack=1','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1'):assert closed.start()<trace.index(marker)<cleanup['ack_offset'],'source retirement follows terminal ACK'
  assert not re.search(r'\[net\.(?:job|dns)\.egress\.submit\]|\[browser.https.released\]|\[browser.tls.prepared\]',trace[cleanup['ack_offset']:]),'retired incarnation performed new network/preparation work'
  joined=dict(cycles=cycles,route_sequences=sequences,resolution=resolution,stream=stream,resources=samples,cleanup=cleanup)
 except(AssertionError,ValueError,KeyError,TypeError,IndexError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joined,scope=SCOPE)

def private_geometry(trace,expected,contract):
 # Independently compose the actual preceding public statuses and panel.
 layouts=list(re.finditer(r'\[browser.tls.layout\] base=(\d+) bytes=64 viewport=(\d+) state=(\w+) pixels=unconfirmed',trace))
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
 label=preparation.LABELS[expected]
 status=(b'\n\nTLS preparation: '+label+b'\nL prep Q clear Y peer').ljust(64,b' ')
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
 if not any('TLS preparation:'in row['text']for row in rows):raise ValueError('controlled HTTPS label missing')
 return dict(tile=dict(x=x,y=y,w=w,h=h),base=base,viewport=sequence,scale=scale,columns=cols,rows=rows,status_text=label.decode())

def verify_private_pixels(trace,ppm,expected,contract,resolution=None):
 from check_disk_persistence import ppm_rgb
 from check_desktop_pixels import glyph
 try:
  g=private_geometry(trace,expected,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=g['tile'];scale=g['scale'];color=contract['color'].to_bytes(3,'big');checked=ink=mismatch=0
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
 p=argparse.ArgumentParser();p.add_argument('--iso',type=Path,required=True);p.add_argument('--source-disk',type=Path,required=True);p.add_argument('--binding',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh lifecycle evidence directory required')
 bound=frozen_binding(root,iso,a.binding.resolve(strict=True));held=json.loads(a.binding.read_text());contract=load_contract(root);target=target_check(root)
 if not target['ok']or held['runner_sha256'].get('scripts/tls_lifecycle_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('lifecycle source/target not exact frozen cohort')
 peer=LifecyclePeer(root,contract['plain_request']);peer.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-tls-lifecycle-{os.getpid()}.sock';resolution=tuple(map(int,a.resolution.split('x')))
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',RTC_NORMAL,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}'];profile=tls.command_profile(command)
 if not profile['ok']:raise RuntimeError(str(profile))
 host_start=time.time();manifest=dict(binding=bound,command=command,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),openssl_version=ssl.OPENSSL_VERSION,model=a.model,resolution=a.resolution,platform_profile=profile,target=target,host_utc_start=host_start,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),pixel_source_sha256=contract['source_sha256'],plan=list(PLAN),body_sha256={str(n):hashlib.sha256(body(n)).hexdigest()for n in range(1,8)},plain_request_hex=contract['plain_request'].hex(),keylog_enabled=False)
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;pixels=[];actions=[];cleanup=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>480:raise TimeoutError('bounded480second lifecycle workflow')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(path.exists()and path.stat().st_size>MAX_PCAP for path in(serial,pcap)):raise RuntimeError('bounded lifecycle raw evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.01)
  raise TimeoutError(pattern)
 def event(event,name):
  deadline=time.monotonic()+30
  while not event.wait(.01):
   health()
   if time.monotonic()>deadline:raise TimeoutError(name)
 def key(code,pattern):
  start=len(text());q.key(code);return wait(pattern,start)
 def picture(name,kind,attempt=None,state=None,anchor=0):
  path=out/(name+'.ppm');deadline=time.monotonic()+25;result=dict(ok=False,errors=['no frame'])
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)})
   if kind=='reader':result=verify_reader_pixels(text(),path.read_bytes(),attempt,contract,anchor,resolution)
   elif kind=='private':result=verify_private_pixels(text(),path.read_bytes(),state,contract,resolution)
   elif kind=='plain':
    result=http.verify_http_pixels(text(),path.read_bytes(),'Ready',PLAIN_BODY,contract,resolution)
    if result['ok']:
     tile,_=https.viewport(text())
     if result['geometry']['tile']!=tile:result=dict(ok=False,errors=['plain HTTP pixel oracle not bound to actual restored held viewport'])
   else:result=https.verify_status_pixels(text(),path.read_bytes(),state,'ready',contract,resolution)
   if result['ok']:break
   time.sleep(.1)
  result.update(screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256'],kind=kind,attempt=attempt,expected=state,anchor=anchor);pixels.append(result);(out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n')
  if not result['ok']:raise RuntimeError(str(result['errors']))
  return result
 def zoom():
  key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
  start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',start);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
 def fetch(attempt):
  start=len(text());q.key('b');wait(r'\[shell.browser.input\] code=48 down=1 slot=43 accepted=1',start);ticket=TLS_TICKETS[attempt-1];wait(rf'\[browser.https.admitted\] ticket={ticket} ',start);event(peer.sent[attempt],'actual lifecycle response sent')
  if attempt in FULL:
   wait(rf'\[browser.https.released\] ticket={ticket} ',start);event(peer.finished[attempt],'actual lifecycle guest close_notify');initial=picture(f'document-{attempt}-initial','reader',attempt,anchor=0);frame=reader_geometry(text(),attempt,contract,0);anchor=http.reader_move(frame,'end');q.key('end');end=picture(f'document-{attempt}-end','reader',attempt,anchor=anchor)
   if f'FINAL LIFECYCLE HTTPS DOCUMENT {attempt:02d} BYTE Z'not in ''.join(end['expected_rows'][2:-1])or end['geometry']['end']!=end['geometry']['total_lines']:raise RuntimeError('current lifecycle End omitted unique body marker')
  else:
   wait(rf'\[browser.https.progress\] ticket={ticket} body_received=1024 authenticated=1 body_visible=0',start);picture(f'document-{attempt}-partial','status',attempt,'Pending')
  actions.append(dict(action='B',attempt=attempt,ticket=ticket,kind=PLAN[attempt-1],body_visible=attempt in FULL,serial_start=start,serial_end=len(text())))
 def clear(name):
  start=len(text());q.key('q');wait(r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1',start);wait(r'\[browser.resources\] reason=clear heap_used=0 heap_free=65536 heap_total=65536 tls_payload=0 ',start);picture(name,'status'if name=='document-cleared'else'private',state='Cancelled');actions.append(dict(action='Q',serial_start=start,serial_end=len(text())))
 def local(name):
  start=len(text());q.key('l');wait(r'\[shell.browser.input\] code=38 down=1 slot=43 accepted=1',start);wait(r'\[browser.tls.view\] state=HelloPending authenticated=0 network=0',start);picture(name,'private',state='HelloPending');actions.append(dict(action='L',serial_start=start,serial_end=len(text())))
 try:
  deadline=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('lifecycle QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,text,key);zoom()
  fetch(1);fetch(2);clear('document-cleared');fetch(3);fetch(4);fetch(5);fetch(6);local('partial-to-local');clear('local-cleared');local('local-before-http');zoom()
  start=len(text());q.key('h');wait(r'\[shell.browser.input\] code=35 down=1 slot=43 accepted=1',start);wait(r'\[browser.http.released\] ticket=8 view=Ready private_job=0',start);picture('plain-after-private','plain');actions.append(dict(action='H',serial_start=start,serial_end=len(text())));zoom();fetch(7)
  start=len(text());q.key_sequence(['f11']);wait(r'\[browser.https.closed\] pending=1 terminal=1 owned_storage=erased body_visible=0',start);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',start);cleanup=cleanup_receipts(text(),15,15,start);q.cmd('screendump',{'filename':str(out/'closed.ppm')});actions.append(dict(action='F11',serial_start=start,serial_end=len(text())));time.sleep(.25);health()
 except Exception as exc:failure=repr(exc)
 finally:
  host_end=time.time();peer.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 manifest['host_utc_end']=host_end;(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=inspect_wire(raw,peer.rows,peer.public_rows,contract['plain_request']);dns=dns_capture(raw,'echo',attempts=peer.attempts,expected=peer.dns)if peer.dns else dict(ok=False,errors=['missing lifecycle DNS']);lifecycle=inspect_lifecycle(trace,wire,dns,raw,host_start,host_end,contract);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 (out/'peer-exchanges.json').write_text(json.dumps(dict(tls=peer.rows,public=peer.public_rows,dns=peer.dns,dns_send_attempts=peer.attempts,keylog_enabled=False,scope='Actual finite endpoint public fixture observations only.'),indent=2)+'\n');result=dict(ok=failure is None and not peer.errors and all(v['ok']for v in(wire,dns,lifecycle,authority,waiting)),error=failure,server_errors=peer.errors,binding=bound,wire=wire,dns=dns,lifecycle=lifecycle,authority=authority,self_wait=waiting,pixels=pixels,actions=actions,cleanup=cleanup,scope=SCOPE);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,tls_connections=len(peer.rows),source_cycles=len(lifecycle['joins']['cycles']),cleanup=cleanup)))

if __name__=='__main__':main()
