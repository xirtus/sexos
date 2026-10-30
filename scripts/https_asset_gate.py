#!/usr/bin/env python3
"""Controlled QOI endpoint, frozen native capture and independently replayable checks.

Never builds. Endpoint sockets are public lab fixtures, not a forwarding proxy.
State-publication ACKs are joined to raw framebuffer pixels, never called pixels.
"""
import argparse,ast,hashlib,json,os,re,shutil,socket,subprocess,time
from pathlib import Path
import https_document_gate as https
import tls_responsiveness_gate as desktop
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from check_disk_persistence import ppm_rgb
from check_desktop_pixels import font_from_source
from check_dns_capture import inspect as dns_capture
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_nonunix_target import inspect as target_check

MODES=('ready','wrong-mime','invalid-qoi','partial-clear','partial-close')
REQUEST=b'GET /art.qoi HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: image/qoi\r\n\r\n'
WIDTH=HEIGHT=48
BASE=(0x30,0x38,0x60)
SOURCE_FILES=(
 'apps/kaleidoscope/src/main.rs','apps/kaleidoscope/src/asset_view.rs','apps/kaleidoscope/src/asset_decode.rs',
 'apps/kaleidoscope/src/tls_probe.rs','apps/kaleidoscope/src/tls_exchange.rs','apps/kaleidoscope/src/bitmap_delivery.rs',
 'apps/kaleidoscope/src/browser_command.rs','apps/kaleidoscope/src/document_import.rs',
 'crates/sex-http/src/codec.rs','crates/sex-http/src/lib.rs','crates/sex-tls/src/lib.rs','crates/sex-tls/src/verifier.rs',
 'crates/sex-tls/src/probe_fixture.rs','crates/sex-trust-config/src/lib.rs','crates/sex-net-config/src/lib.rs',
 'crates/sex-graphics/src/qoi.rs','crates/sex-pdx/src/lib.rs','crates/sex-pdx/src/network_job.rs','crates/sex-pdx/src/bitmap_batch.rs','crates/sex-pdx/src/desktop_style.rs',
 'servers/sexnet/src/main.rs','servers/sexnet/src/jobs.rs','servers/sexnet/src/dns_jobs.rs',
 'servers/sexnet/src/trust_policy.rs','servers/sexnet/src/certificate_time.rs','servers/sexnet/src/entropy.rs',
 'kernel/src/app_resources.rs','kernel/src/init.rs',
 'servers/sexdisplay/src/main.rs','servers/sexdisplay/src/bitmap_transaction.rs',
 'servers/sexdisplay/src/surface_lifecycle.rs','servers/sexdisplay/src/surface_groups.rs')
RUNNER_FILES=('scripts/https_asset_gate.py','scripts/check_https_asset.py','scripts/https_document_gate.py',
 'scripts/tls_exchange_gate.py','scripts/tls_responsiveness_gate.py','scripts/network_contention_gate.py',
 'scripts/check_disk_persistence.py','scripts/check_dns_capture.py','scripts/check_tcp_capture.py',
 'scripts/check_desktop_pixels.py','scripts/desktop_close_gate.py','scripts/desktop_focus.py',
 'scripts/check_nic_command.py','scripts/check_self_wait.py','scripts/check_nonunix_target.py',
 'tests/fixtures/tls-controlled/server.pem','tests/fixtures/tls-controlled/server-PUBLIC-TEST-key.pem')
def script_dependencies(root,names):
 """Freeze the complete local import closure, including function-local imports."""
 found=set();pending=list(names);root=Path(root)
 while pending:
  name=pending.pop()
  if name in found:continue
  found.add(name)
  # The freezer binds public endpoint fixtures with runners, but PEM data is
  # never Python source and must not enter the local-module import scanner.
  if not name.endswith('.py'):continue
  tree=ast.parse((root/name).read_text())
  for node in ast.walk(tree):
   modules=[a.name for a in node.names]if isinstance(node,ast.Import)else[node.module]if isinstance(node,ast.ImportFrom)and node.module else[]
   for module in modules:
    relative='scripts/'+module.split('.')[0]+'.py'
    if(root/relative).is_file()and relative not in found:pending.append(relative)
 return tuple(sorted(found))
RUNNER_FILES=script_dependencies(Path(__file__).resolve().parents[1],RUNNER_FILES)
SCOPE='Actual explicit G on immutable BOOT HTTPS endpoint, authenticated bounded image/qoi, exact positive RELEASE before bounded decode/publication, complete scaled straight-alpha pixels in raw PPM and positive Q/CLEAR erasure. Public controlled root and endpoint only; no TLS key logs, proxy, public Internet, runtime capability mint, arbitrary URL, execution or hostile-content containment claim.'

def fingerprint(data):
 value=0xcbf29ce484222325
 for byte in data:value=((value^byte)*0x100000001b3)&0xffffffffffffffff
 return value

def rgba():
 """Procedural, deliberately non-grayscale and mixed-alpha public oracle."""
 return bytes(value for y in range(HEIGHT)for x in range(WIDTH)for value in
              ((x*5+y*3+17)&255,(y*5+x*2+43)&255,((x//6+y//6)%2*137+x+y+29)&255,(0,128,255)[(x//8+y//8)%3]))

def qoi():
 # All-RGBA subset of the official CC0 QOI specification: qoiformat.org.
 pixels=rgba();out=bytearray(b'qoif'+WIDTH.to_bytes(4,'big')+HEIGHT.to_bytes(4,'big')+b'\4\0')
 for at in range(0,len(pixels),4):out.append(255);out.extend(pixels[at:at+4])
 out.extend(b'\0'*7+b'\1');assert len(out)==11542;return bytes(out)

BODY=qoi()
_RGBA=rgba()
PIXEL_HASH=fingerprint(bytes(value for at in range(0,len(_RGBA),4)for value in
                             (_RGBA[at+2],_RGBA[at+1],_RGBA[at],_RGBA[at+3])))

def response(mode):
 body=BODY[:-1]+b'\2'if mode=='invalid-qoi'else BODY
 mime=b'text/plain'if mode=='wrong-mime'else b'image/qoi'
 header=b'HTTP/1.1 200 OK\r\nContent-Type: '+mime+b'\r\nContent-Length: '+str(len(body)).encode()+b'\r\nConnection: close\r\n\r\n'
 return header+(body[:1024]if mode.startswith('partial-')else body)

def decode_fixture(body):
 """Independent strict decoder for the public fixture's full-RGBA QOI subset."""
 if len(body)!=11542 or body[:14]!=b'qoif\0\0\0\x30\0\0\0\x30\4\0'or body[-8:]!=b'\0'*7+b'\1':raise ValueError('fixture QOI header/extent/end differs')
 result=bytearray()
 for at in range(14,len(body)-8,5):
  if body[at]!=255:raise ValueError('fixture does not use full RGBA opcodes')
  result.extend(body[at+1:at+5])
 if bytes(result)!=rgba():raise ValueError('public procedural RGBA source differs')
 return bytes(result)

class AssetPeer(https.tls.ControlledPeer):
 def __init__(self,root,mode):
  if mode not in MODES:raise ValueError('unknown asset mode')
  super().__init__(root,mode);self.count=2 if mode=='ready'else 1
 def serve_dns(self,sock):
  while not self.stop.is_set():
   try:data,remote=sock.recvfrom(513)
   except socket.timeout:continue
   ticket=len(self.dns)+1
   if ticket>self.count or data!=https.tls.dns_query(ticket):raise ValueError('extra/foreign asset DNS query')
   reply=https.tls.dns_response(ticket);sent=sock.sendto(reply,remote);self.dns.append(dict(ticket=ticket,response_hex=reply.hex()));self.attempts.append(dict(ticket=ticket,kind='answer',response_hex=reply.hex(),sent_bytes=sent))
 def serve_tls(self,listener,ctx):
  for attempt in range(1,self.count+1):
   sock=self.accept(listener)
   if sock is None:return
   row=dict(attempt=attempt,handshake_complete=False,sni=None,tls_version=None,cipher=None,request_hex=None,response_hex=None,application_bytes_received=0,extra_application_bytes=0,unwrap_completed=False);self.rows.append(row)
   wrapped=ctx.wrap_socket(sock,server_side=True,do_handshake_on_connect=False);self.live.append(wrapped);wrapped.do_handshake();row.update(handshake_complete=True,sni=getattr(wrapped,'controlled_sni',None),tls_version=wrapped.version(),cipher=wrapped.cipher()[0])
   if(row['sni'],row['tls_version'],row['cipher'])!=(https.tls.HOST,'TLSv1.3',https.tls.CIPHER):raise ValueError('asset TLS profile differs')
   request=bytearray()
   while len(request)<len(REQUEST):
    part=wrapped.recv(512)
    if not part:raise ValueError('guest closed before asset GET')
    request.extend(part)
    if len(request)>len(REQUEST):raise ValueError('extra application request')
   if bytes(request)!=REQUEST:raise ValueError('fixed asset GET/Accept differs')
   wire=response(self.mode);row.update(request_hex=bytes(request).hex(),application_bytes_received=len(request))
   for at in range(0,len(wire),1024):wrapped.sendall(wire[at:at+1024])
   row['response_hex']=wire.hex()
   if self.mode=='wrong-mime'or self.mode.startswith('partial-'):
    # These intentionally incomplete application exchanges need no peer
    # close_notify. Their exact request and all intended response writes are
    # observed now; watch for forbidden extra application bytes until EOF or
    # the gate has independently observed positive cleanup and stops us.
    self.completed.set();wrapped.settimeout(.1)
    while not self.stop.is_set():
     try:extra=wrapped.recv(512)
     except socket.timeout:continue
     except OSError:break
     if not extra:break
     row['extra_application_bytes']+=len(extra)
     raise ValueError('extra decrypted asset application data')
   else:
    plain=wrapped.unwrap();row['unwrap_completed']=True;plain.close()
   if row['extra_application_bytes']:raise ValueError('extra decrypted asset application data')
  self.completed.set()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('unrequested asset connection')
   except socket.timeout:pass

def source_contract(root):
 root=Path(root);sources={name:(root/name).read_bytes()for name in SOURCE_FILES};style=sources['crates/sex-pdx/src/desktop_style.rs'].decode();display=sources['servers/sexdisplay/src/main.rs'].decode();codec=sources['crates/sex-http/src/codec.rs'].decode();view=sources['apps/kaleidoscope/src/asset_view.rs'].decode()
 for literal in ('INSET: u32 = 8','LINE: u32 = 9','width >= 256'):
  if literal not in style:raise ValueError('source-bound text/image placement changed')
 for literal in ('b"/art.qoi"','b"image/qoi"','MAX_BODY: usize = 12288','MAX_HEADERS: usize = 1024'):
  if literal not in codec:raise ValueError('source-bound fixed binary HTTP contract changed')
 if 'min(4)'not in view or '2*LINE*scale(w)'not in view or '0x00303860u32'not in display:raise ValueError('source-bound fit/background contract changed')
 return dict(source_sha256={n:hashlib.sha256(data).hexdigest()for n,data in sources.items()},font=font_from_source(display),base=list(BASE),pixel_hash=hex(PIXEL_HASH),body_hash=hex(fingerprint(BODY)),body_sha256=hashlib.sha256(BODY).hexdigest())

def binding_check(root,iso,path):
 result=frozen_binding(root,iso,path);held=json.loads(Path(path).read_text())
 for name in SOURCE_FILES:
  if held['sources'].get(name)!=hashlib.sha256((Path(root)/name).read_bytes()).hexdigest():raise ValueError('asset production dependency absent from frozen closure: '+name)
 for name in RUNNER_FILES:
  if held['runner_sha256'].get(name)!=hashlib.sha256((Path(root)/name).read_bytes()).hexdigest():raise ValueError('asset checker dependency absent from frozen closure: '+name)
 return result

def prerequisites(trace,intents,admitted,wire,host_start,host_end):
 markers=('[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1')
 for marker in markers:
  if trace.count(marker)!=1 or trace.count(marker.split(']')[0]+']')!=1:raise ValueError('exact BOOT/raw source authority differs')
 if any(re.search(r'\b(?:key|keylog|traffic_secret|master_secret|seed_bytes|seed_value|secret)=',line)for line in trace.splitlines()if re.search(r'\[(?:browser\.(?:asset|tls|https)|entropy\.)',line)):raise ValueError('private TLS/seed material leaked to metadata')
 policies=records(trace,'[trust.policy.snapshot]',r'\[trust.policy.snapshot\] owner=15 task=15 seq=(\d+) profile=1 root_bytes=376 origin=held_boot_grant')
 sources=records(trace,'[clock.source.snapshot]',r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=(\d+) attempts=([1-4]) precision=1 profile=q35_cmos_utc ordinal=(\d+)')
 clocks=records(trace,'[certificate.time.snapshot]',r'\[certificate.time.snapshot\] owner=15 task=15 seq=(\d+) seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained')
 seeds=records(trace,'[entropy.seed.admitted]',r'\[entropy.seed.admitted\] owner=15 task=15 ticket=(\d+) bytes=32 source=held_boot_grant')
 clean=records(trace,'[entropy.seed.released]',r'\[entropy.seed.released\] owner=15 task=15 ticket=(\d+) service_payload=erased receipt=retained')
 checks=records(trace,'[browser.trust.self_check]',r'\[browser.trust.self_check\] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0')
 prepared=records(trace,'[browser.tls.prepared]',r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0')
 duplex=records(trace,'[net.job.duplex.admitted]',r'\[net.job.duplex.admitted\] owner=15 task=15 ticket=(\d+) kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route')
 if any(len(group)!=len(intents)for group in(policies,sources,clocks,seeds,clean,checks,prepared,duplex)):raise ValueError('missing/extra source preparation or service admission')
 previous=0
 for i,parts in enumerate(zip(intents,admitted,wire['joins'],policies,sources,clocks,seeds,clean,checks,prepared,duplex),1):
  intent,admit,flow,policy,source,clock,seed,release,check,prep,service=parts
  if[int(policy[1]),int(source[3]),int(clock[1]),int(seed[1]),int(release[1]),int(service[1])]!=[i,i,i,i,i,i+1]:raise ValueError('prerequisite/duplex incarnation not fresh')
  seconds=int(source[1]);task,private,initial,utc=map(int,prep.groups())
  if seconds!=int(clock[2])or seconds!=utc or not host_start-1<=seconds<=host_end+2 or seconds<previous:raise ValueError('actual source/retained/prepared UTC differs')
  previous=seconds
  if(task,private)!=(int(admit[4]),int(admit[5]))or initial!=flow['initial_tls_bytes']or not 0<initial<=4096:raise ValueError('private preparation/actual ClientHello differs')
  produced=re.search(rf'\[entropy.seed.progress\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={i} state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace)
  state=re.search(rf'\[net.job.state\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={i+1} ',trace)
  authenticated=re.search(rf'\[browser.asset.authenticated\] ticket={i+1} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1',trace)
  # BEGIN reserves the service ticket before its receipt reaches the App.
  # The client then writes ClientHello and submits START; only afterward can
  # Connecting progress to Sending and the peer become authenticated.
  if not produced or not state or not authenticated or not intent.start()<policy.start()<source.start()<clock.start()<check.start()<prep.start()<service.start()<admit.start()<state.start()<authenticated.start()or not policy.start()<seed.start()<produced.start()<release.start()<check.start():raise ValueError('source/admission ownership and positive cleanup order differs')

def incoming_records(data,mode):
 """Inspect outer framing only; rejected streams may end in encrypted data.

 The caller must separately join the exact negative application/cleanup
 receipts. A truncated tail is neither decrypted nor authenticated evidence.
 """
 if mode not in('wrong-mime','partial-clear','partial-close'):
  return https.tls.tls_records(data),None
 records=[];offset=0
 while offset<len(data):
  left=data[offset:];header=left[:5]
  length=int.from_bytes(header[3:5],'big')if len(header)==5 else None
  if len(header)<5 or len(left)<5+length:
   if not records or not any(r['type']==23 for r in records):raise ValueError('negative tail precedes complete ServerHello/AEAD prefix')
   https.tls.hello(records[0],False)
   if header[:3]!=b'\x17\x03\x03'[:min(3,len(header))]:raise ValueError('negative tail kind/version differs')
   if len(header)>=4 and max(17,header[3]<<8)>min(1041,(header[3]<<8)|255):raise ValueError('negative tail declared extent exceeds fixture bound')
   if length is not None and not 17<=length<=1041:raise ValueError('negative tail declared extent exceeds fixture bound')
   return records,dict(offset=offset,available_bytes=len(left),header_bytes=len(header),payload_available=max(0,len(left)-5),declared_bytes=length,sha256=hashlib.sha256(left).hexdigest(),authenticated=False,scope='outer_framing_only_no_decrypted_tail')
  record=https.tls.tls_records(left[:5+length])[0]
  if records and(record['type']not in(20,23)or record['version']!=0x0303 or record['type']==23 and record['length']>1041):raise ValueError('negative complete record prefix differs')
  records.append(record);offset+=5+length
 return records,None

def inspect_wire(raw,rows,mode):
 flows,_=https.tls.reassemble(raw);tls=sorted((f for f in flows if f['peer_port']==46080),key=lambda f:f['guest_port']);echo=[f for f in flows if f['peer_port']==46078];count=2 if mode=='ready'else 1
 if len(flows)!=count+1 or len(tls)!=count or len(rows)!=count or len(echo)!=1:raise ValueError('extra/missing endpoint flows')
 if len(tls)!=count or len(rows)!=count:raise ValueError('missing actual TLS endpoint observations')
 if echo[0]['guest_port']!=48001 or echo[0]['outgoing']!=echo[0]['incoming']or echo[0]['outgoing']!=https.tls.ECHO or not echo[0]['fin_in']or not echo[0]['fin_out']:raise ValueError('startup Echo differs')
 result=[]
 for index,(flow,row)in enumerate(zip(tls,rows),1):
  if flow['guest_port']!=48001+index or row['attempt']!=index or row['handshake_complete']is not True or(row['sni'],row['tls_version'],row['cipher'])!=(https.tls.HOST,'TLSv1.3',https.tls.CIPHER):raise ValueError('TLS identity/ticket/profile differs')
  if row['request_hex']!=REQUEST.hex()or row['response_hex']!=response(mode).hex()or row['application_bytes_received']!=len(REQUEST)or row['extra_application_bytes']!=0:raise ValueError('decrypted fixed request/response differs')
  if mode in('ready','invalid-qoi')and row['unwrap_completed']is not True:raise ValueError('guest encrypted local close not observed')
  outgoing=https.tls.tls_records(flow['outgoing']);incoming,tail=incoming_records(flow['incoming'],mode);hello=https.tls.hello(outgoing[0],True);https.tls.hello(incoming[0],False)
  if not all(r['type']in(20,23)for r in outgoing[1:]+incoming[1:])or not any(r['type']==23 for r in incoming):raise ValueError('post-Hello plaintext/missing AEAD records')
  if any(token in stream for token in(REQUEST,BODY,b'GET /art.qoi',b'HTTP/1.1')for stream in(flow['outgoing'],flow['incoming'])):raise ValueError('application plaintext escaped TLS')
  if mode in('ready','invalid-qoi')and(flow['rst_in']or flow['rst_out']or not flow['fin_in']or not flow['fin_out']):raise ValueError('incomplete successful encrypted flow')
  result.append(dict(ticket=index+1,initial_tls_bytes=outgoing[0]['length']+5,outgoing_bytes=len(flow['outgoing']),incoming_bytes=len(flow['incoming']),client_hello=hello,incoming_tail=tail))
 return dict(ok=True,joins=result)

def records(trace,prefix,pattern):
 found=list(re.finditer(pattern,trace))
 if len(found)!=trace.count(prefix):raise ValueError('malformed/foreign '+prefix)
 return found

def exact_cleanup(trace,start):
 result=cleanup_receipts(trace,15,15,start);suffix=trace[start:];roles={};mask=0
 for m in re.finditer(r'\[app.service.ack\] target=15 task=15 holder=(\d+) role=(\d+) acked=(\d+) required=31',suffix):
  holder,role,acked=map(int,m.groups())
  if {1:3,2:1,4:11,8:13,16:10}.get(role)!=holder or role in roles or acked!=mask|role:raise ValueError('foreign/duplicate/nonaccumulating service cleanup ACK')
  mask=acked;roles[role]=start+m.start()
 if set(roles)!={1,2,4,8,16}or mask!=31:raise ValueError('incomplete actual five-holder cleanup')
 retired=re.search(r'\[app.private.retired\] target=15 task=15 [^\r\n]*acked=31 required=31 all_flushed=1 pool_reusable=1',suffix)
 if not retired or max(roles.values())>=start+retired.start()or start+retired.start()>=result['pool_offset']:raise ValueError('private retirement/pool preceded exact service cleanup')
 return result

def inspect_trace(trace,mode,wire,raw,host_start,host_end,contract,dns):
 if re.search(r'\[browser.import.(?:intent|submit|control|event)\]|\[files.browser.import.created\]',trace):raise ValueError('binary asset entered text import/storage')
 for marker in('[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0'):
  if trace.count(marker)!=1:raise ValueError('missing exact immutable BOOT authority')
 intents=records(trace,'[browser.asset.intent]',r'\[browser.asset.intent\] accepted=1 explicit=1 public_internet=0');admitted=records(trace,'[browser.asset.admitted]',r'\[browser.asset.admitted\] ticket=(\d+) dns_generation=(\d+) seq=(\d+) task_bytes=(\d+) private_bytes=(\d+) profile=1 authority=held_route');auth=records(trace,'[browser.asset.authenticated]',r'\[browser.asset.authenticated\] ticket=(\d+) profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1')
 if len(intents)!=len(admitted)or len(auth)!=len(intents)or len(intents)!=len(wire['joins']):raise ValueError('missing explicit asset intent/admission/authentication')
 g_inputs=list(re.finditer(r'\[shell.browser.input\] code=34 down=1 slot=43 accepted=1',trace))
 if len(g_inputs)!=len(intents)or any(key.start()>=intent.start()for key,intent in zip(g_inputs,intents)):raise ValueError('explicit G lacks held-shell accepted key')
 prerequisites(trace,intents,admitted,wire,host_start,host_end)
 resolution=https.http.resolution_joins(trace,raw,[(i+2,i+1)for i in range(len(intents))],dns)
 if not resolution['ok']:raise ValueError(str(resolution['errors']))
 previous=0
 for i,(intent,admit,authenticated,flow)in enumerate(zip(intents,admitted,auth,wire['joins']),1):
  ticket,generation,seq,task,private=map(int,admit.groups())
  if(ticket,generation)!=(i+1,i)or int(authenticated[1])!=ticket or not previous<seq<=0xffffff or not 0<task<=32768 or private!=task+8248+12288 or not intent.start()<admit.start()<authenticated.start():raise ValueError('resource lease/private/intent correlation differs')
  previous=seq
  policy=re.search(rf'\[trust.policy.snapshot\] owner=15 task=15 seq={i} profile=1 root_bytes=376 origin=held_boot_grant',trace);clock=re.search(rf'\[certificate.time.snapshot\] owner=15 task=15 seq={i} seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace);seed=re.search(rf'\[entropy.seed.released\] owner=15 task=15 ticket={i} service_payload=erased receipt=retained',trace)
  if not policy or not clock or not seed or not host_start-1<=int(clock[1])<=host_end+2 or not intent.start()<policy.start()<clock.start()<admit.start()or not policy.start()<seed.start()<admit.start():raise ValueError('missing fresh policy/UTC/positive seed cleanup')
 bodies=records(trace,'[browser.asset.body]',r'\[browser.asset.body\] ticket=(\d+) bytes=(\d+) hash=(0x[0-9a-f]+) parsed=1 authenticated=1 visible=0');released=records(trace,'[browser.asset.released]',r'\[browser.asset.released\] ticket=(\d+) tx=(\d+) rx=(\d+) body_bytes=(\d+) body_hash=(0x[0-9a-f]+) positive_release=1 authenticated=1');decoded=records(trace,'[browser.asset.decoded]',r'\[browser.asset.decoded\] ticket=(\d+) width=48 height=48 pixels=2304 hash=(0x[0-9a-f]+) bounded_turn=128')
 expected=2 if mode=='ready'else 1 if mode=='invalid-qoi'else 0
 if len(bodies)!=len(released)or len(released)!=expected:raise ValueError('partial/MIME rejection published body, or successful RELEASE missing')
 body=response(mode).split(b'\r\n\r\n',1)[1]
 for i,(parsed,release)in enumerate(zip(bodies,released)):
  flow=wire['joins'][i];ticket=flow['ticket'];want=(ticket,len(body),hex(fingerprint(body)))
  if(int(parsed[1]),int(parsed[2]),parsed[3])!=want or tuple(map(int,release.groups()[:4]))!=(ticket,flow['outgoing_bytes'],flow['incoming_bytes'],len(body))or release[5]!=want[2]or not auth[i].start()<parsed.start()<release.start():raise ValueError('actual authenticated body/released wire cursor differs')
 if mode=='ready':
  if len(decoded)!=2 or any(int(m[1])!=i+2 or m[2]!=hex(PIXEL_HASH)or m.start()<released[i].end()for i,m in enumerate(decoded)):raise ValueError('decode lacks prior exact positive RELEASE')
  reject=re.search(r'\[browser.import.rejected\] reason=asset_not_text effects=0',trace)
  clear=next((m for m in re.finditer(r'\[browser.asset.cleared\] viewport=(\d+) sequence=(\d+) positive_ack=1',trace)if reject and reject.end()<m.start()<intents[1].start()),None)
  if not reject or not clear or not released[0].start()<reject.start()<clear.start()<intents[1].start():raise ValueError('missing binary import rejection/positive CLEAR/fresh explicit G')
  ikey=re.search(r'\[shell.browser.input\] code=23 down=1 slot=43 accepted=1',trace[released[0].end():reject.start()]);qkey=re.search(r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1',trace[reject.end():clear.start()])
  if not ikey or not qkey:raise ValueError('binary import rejection/Q lacks real accepted I/Q input')
  clear_commit=re.search(rf'\[display.bitmap.committed\] target=15 surface=300 paint={clear[2]} width=0 height=0 scale=0 pixels=unconfirmed',trace[:clear.start()])
  if not clear_commit or clear_commit.start()<reject.end():raise ValueError('positive CLEAR lacks exact display-owned commit')
 elif decoded or '[browser.asset.applied]'in trace:raise ValueError('negative asset became decoded/published')
 if mode=='invalid-qoi':
  rejection=records(trace,'[browser.asset.rejected]',r'\[browser.asset.rejected\] reason=invalid_qoi effects=0 owned_storage=erased')
  if len(rejection)!=1 or rejection[0].start()<released[0].end():raise ValueError('actual final-marker decoder rejection absent/out of order')
 if mode=='wrong-mime':
  cleanup=re.search(r'\[browser.tls.cleanup\] ticket=2 outcome=Unavailable authenticated=1 document=0 tls_payload=0 positive_release=1',trace)
  rejection=records(trace,'[browser.asset.rejected]',r'\[browser.asset.rejected\] ticket=2 positive_release=1 authenticated=1 failure=Some\(Application\) http=Some\(UnsupportedContentType\)')
  view=re.search(r'\[browser.asset.view\] state=Unavailable controlled_profile=1 public_internet=0',trace)
  if not cleanup or len(rejection)!=1 or not view or not auth[0].end()<cleanup.start()<rejection[0].start()<view.start():raise ValueError('MIME rejection lacks actual parser diagnosis/positive cleanup')
 if mode.startswith('partial-'):
  progress=re.search(r'\[browser.asset.progress\] ticket=2 body_received=1024 authenticated=1 body_visible=0',trace)
  if not progress:raise ValueError('actual private partial ingress missing')
  if mode=='partial-close':
   close=re.search(r'\[browser.asset.closed\] pending=1 terminal=1 owned_storage=erased',trace)
   if not close or close.start()<progress.end():raise ValueError('partial Close missing')
   exact_cleanup(trace,close.start())
   ack=re.search(r'\[net.app.cleanup.ack\] target=15 task=15 client_leases=0 shared_cache=retained ok=1',trace[close.end():]);ack_end=close.end()+ack.end()
   for marker in('[trust.policy.retired] target=15 task=15 before_cleanup_ack=1','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1'):
    at=trace.find(marker,close.end())
    if not close.end()<at<ack_end:raise ValueError('prerequisite not retired before actual network cleanup ACK')
   if re.search(r'\[net\.(?:job|dns)\.egress\.submit\]',trace[ack_end:]):raise ValueError('retired Browser emitted network work after cleanup ACK')
  elif not re.search(r'\[browser.tls.cleanup\] ticket=2 outcome=Cancelled authenticated=1 document=0 tls_payload=0 positive_release=1',trace[progress.end():]):raise ValueError('partial clear lacks positive network cleanup')
 return dict(ok=True,resources=len(intents),released=len(released),decoded=len(decoded))

def geometry(trace,require_ack=True):
 tile,viewport=https.viewport(trace);x=8;y=44+18*(2 if tile['w']>=256 else 1);scale=min(max(0,tile['w']-x-8)//48,max(0,tile['h']-y-8)//48,4)
 if scale<1:raise ValueError('actual Browser viewport cannot fit fixture')
 expected=dict(x=x,y=y,width=48,height=48,scale=scale,viewport=viewport,tile=tile)
 if require_ack:
  layouts=list(re.finditer(r'\[browser.asset.layout\] ticket=(\d+) viewport=(\d+) x=(\d+) y=(\d+) width=(\d+) height=(\d+) scale=(\d+) hash=(0x[0-9a-f]+) pixels=unconfirmed',trace))
  if not layouts:raise ValueError('image layout missing')
  m=layouts[-1];ticket=int(m[1]);values=tuple(map(int,m.groups()[1:7]))
  if values!=(viewport,x,y,48,48,scale)or m[8]!=hex(PIXEL_HASH):raise ValueError('layout differs from actual source geometry')
  release=re.search(rf'\[browser.asset.released\] ticket={ticket} .*positive_release=1 authenticated=1',trace[:m.start()]);decode=re.search(rf'\[browser.asset.decoded\] ticket={ticket} width=48 height=48 pixels=2304 hash={hex(PIXEL_HASH)} bounded_turn=128',trace[:m.start()])
  if not release or not decode or not release.start()<decode.start()<m.start():raise ValueError('layout precedes released bounded decode')
  text_ack=re.search(rf'\[browser.asset.text.applied\] viewport={viewport} sequence=([1-9]\d*) bytes=(\d+) rows=\d+ positive_ack=1 pixels=unconfirmed',trace[:m.start()])
  if not text_ack:raise ValueError('bitmap precedes current-viewport positive text clearing')
  if not re.search(rf'\[display.text.committed\] target=15 surface=300 paint={text_ack[1]} bytes={text_ack[2]} changed=[01] pixels=unconfirmed',trace[:text_ack.start()]):raise ValueError('text ACK lacks exact display-owned commit')
  ack=re.search(rf'\[browser.asset.applied\] ticket={ticket} viewport={viewport} sequence=([1-9]\d*) x={x} y={y} width=48 height=48 scale={scale} hash={hex(PIXEL_HASH)} positive_ack=1 pixels=unconfirmed',trace[m.end():])
  if not ack:raise ValueError('actual source geometry lacks exact positive Applied')
  display=re.search(rf'\[display.bitmap.committed\] target=15 surface=300 paint={ack[1]} width=48 height=48 scale={scale} pixels=unconfirmed',trace[m.end():m.end()+ack.start()])
  if not display:raise ValueError('bitmap ACK lacks prior exact display-owned commit')
  clear=list(re.finditer(r'\[browser.asset.cleared\]',trace))
  if clear and clear[-1].start()>m.start():raise ValueError('image superseded by CLEAR')
  expected.update(ticket=ticket,sequence=int(ack[1]))
 return expected

def verify_pixels(trace,ppm,visible=True,resolution=None):
 """Every scaled pixel and one-pixel guard: straight alpha + real body clip."""
 g=geometry(trace,visible);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
 if resolution and(width,height)!=tuple(resolution):raise ValueError('framebuffer resolution differs')
 tile=g['tile'];data=decode_fixture(BODY);checked=colored=0
 if tile['x']<0 or tile['y']<50 or tile['w']<=0 or tile['h']<=0 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('actual Browser frame lies outside framebuffer')
 if not(8<=g['x']and 36<=g['y']and g['x']+48*g['scale']<=tile['w']-8 and g['y']+48*g['scale']<=tile['h']-8):raise ValueError('complete image lies outside actual Browser body')
 for ly in range(g['y']-1,g['y']+48*g['scale']+1):
  for lx in range(g['x']-1,g['x']+48*g['scale']+1):
   sx=tile['x']+lx;sy=tile['y']+ly
   if not(0<=sx<width and 0<=sy<height):continue
   # Actual compositor clips inside the Browser surface, never shifts origins.
   clipped=not(8<=lx<tile['w']-8 and 36<=ly<tile['h']-8)
   inside=g['x']<=lx<g['x']+48*g['scale']and g['y']<=ly<g['y']+48*g['scale']
   if clipped:continue
   expected=BASE
   if visible and inside:
    at=(((ly-g['y'])//g['scale'])*48+(lx-g['x'])//g['scale'])*4;r,green,b,a=data[at:at+4];expected=tuple((fg*a+bg*(255-a)+127)//255 for fg,bg in zip((r,green,b),BASE));colored+=expected!=BASE
   actual=tuple(pixels[(sy*width+sx)*3:(sy*width+sx)*3+3]);checked+=1
   if actual!=expected:raise ValueError(f'RGBA/scale/clipping/stale pixel differs local=({lx},{ly}) screen=({sx},{sy}) wanted={expected} actual={actual}')
 if not checked or visible and not colored:raise ValueError('no complete visible image evidence')
 return dict(ok=True,checked_pixels=checked,colored_pixels=colored,geometry=g,visible=visible)

def frame_plan(mode):
 return [('initial-image',True,2),('cleared-image',False,None),('fresh-image',True,3),('zoomed-image',True,3)]if mode=='ready'else [('closed',False,None)]if mode=='partial-close'else [('negative-image',False,None)]

def validate_frame_stages(trace,mode,frames):
 plan=frame_plan(mode)
 if[(s['name'],s['visible'])for s in frames]!=[(name,visible)for name,visible,_ in plan]:raise ValueError('native frame stage order/visibility differs')
 previous=-1;geometries=[]
 for shot,(name,visible,ticket)in zip(frames,plan):
  boundary=shot['serial_end']
  if not isinstance(boundary,int)or not previous<boundary<=len(trace):raise ValueError('frame serial boundaries not strictly ordered')
  previous=boundary;prefix=trace[:boundary]
  if hashlib.sha256(prefix.encode()).hexdigest()!=shot['serial_sha256']:raise ValueError('frame/serial boundary differs')
  if visible:
   g=geometry(prefix,True)
   if g['ticket']!=ticket or prefix.count('[browser.asset.intent]')!=ticket-1:raise ValueError('frame belongs to wrong/future fetch')
   geometries.append(g)
  elif name=='cleared-image':
   clear=list(re.finditer(r'\[browser.asset.cleared\] viewport=(\d+) sequence=(\d+) positive_ack=1',prefix));rejected=re.search(r'\[browser.import.rejected\] reason=asset_not_text effects=0',prefix)
   if not clear or not rejected or not frames[0]['serial_end']<rejected.start()<clear[-1].start()or prefix.count('[browser.asset.intent]')!=1:raise ValueError('clear frame lacks explicit I denial/Q and positive erasure phase')
  elif name=='closed':
   closed=re.search(r'\[browser.asset.closed\] pending=1 terminal=1 owned_storage=erased',prefix)
   if not closed:raise ValueError('Close frame precedes actual Close')
   exact_cleanup(prefix,closed.start())
  else:
   marker=r'\[browser.asset.rejected\] reason=invalid_qoi effects=0'if mode=='invalid-qoi'else rf'\[browser.tls.cleanup\] ticket=2 outcome={"Unavailable"if mode=="wrong-mime"else"Cancelled"} .*positive_release=1'
   if not re.search(marker,prefix):raise ValueError('negative frame precedes rejection/positive cleanup')
 if mode=='ready'and(geometries[0]['viewport']!=geometries[1]['viewport']or geometries[2]['viewport']<=geometries[1]['viewport']or not geometries[0]['sequence']<geometries[1]['sequence']<geometries[2]['sequence']):raise ValueError('fresh image/actual resize publication phases differ')

def verify_evidence(root,iso,binding,evidence):
 evidence=Path(evidence);bound=binding_check(root,iso,binding);manifest=json.loads((evidence/'manifest.json').read_text());contract=source_contract(root)
 if manifest['binding']!=bound or manifest['source_sha256']!=contract['source_sha256']or manifest['keylog_enabled']is not False:raise ValueError('evidence/source/image binding differs')
 required_raw={'serial.log','wire.pcap','peer-exchanges.json'}|{s['file']for s in manifest['frames']}
 if set(manifest['raw_sha256'])!=required_raw or any(not re.fullmatch(r'[a-z-]+\.ppm',s['file'])for s in manifest['frames']):raise ValueError('missing/foreign raw evidence closure')
 if manifest['mode']not in MODES or manifest['resolution']not in('1280x800','1024x768'):raise ValueError('unknown frozen native profile')
 command=manifest['command'];profile=https.tls.command_profile(command)
 if not profile['ok']or command[command.index('-cdrom')+1]!=str(Path(iso).resolve()):raise ValueError('native command/image differs')
 for name,wanted in manifest['raw_sha256'].items():
  path=(evidence/name).resolve()
  if not path.is_relative_to(evidence.resolve())or hashlib.sha256(path.read_bytes()).hexdigest()!=wanted:raise ValueError('raw evidence hash/path differs: '+name)
 trace=(evidence/'serial.log').read_text(errors='replace');raw=(evidence/'wire.pcap').read_bytes();peer=json.loads((evidence/'peer-exchanges.json').read_text());mode=manifest['mode']
 if peer['keylog_enabled']is not False or peer['errors']:raise ValueError('endpoint errors/private key logging')
 wire=inspect_wire(raw,peer['tls'],mode);dns=dns_capture(raw,'echo',attempts=peer['dns_send_attempts'],expected=peer['dns'])
 if not dns['ok']:raise ValueError(str(dns['errors']))
 joins=inspect_trace(trace,mode,wire,raw,manifest['host_utc_start'],manifest['host_utc_end'],contract,dns);shots=[]
 validate_frame_stages(trace,mode,manifest['frames'])
 for shot in manifest['frames']:
  prefix=trace[:shot['serial_end']]
  if hashlib.sha256(prefix.encode()).hexdigest()!=shot['serial_sha256']:raise ValueError('frame/serial boundary differs')
  if shot['name']=='closed':
   width,height,_=ppm_rgb((evidence/shot['file']).read_bytes(),min_width=1,min_height=1)
   if(width,height)!=tuple(map(int,manifest['resolution'].split('x'))):raise ValueError('Close capture framebuffer resolution differs')
   continue
  if shot['name']=='cleared-image'and '[browser.asset.cleared]'not in prefix:raise ValueError('erasure frame lacks positive CLEAR')
  shots.append(verify_pixels(prefix,(evidence/shot['file']).read_bytes(),shot['visible'],tuple(map(int,manifest['resolution'].split('x')))))
 if mode=='ready'and shots[0]['geometry']['viewport']==shots[-1]['geometry']['viewport']:raise ValueError('actual viewport/scale transition missing')
 authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace);target=target_check(Path(root))
 if not all(r['ok']for r in(authority,waiting,target)):raise ValueError('native authority/self-wait/non-Unix target check failed')
 return dict(ok=True,binding=bound,mode=mode,wire=wire,trace=joins,pixels=shots,authority=authority,self_wait=waiting,target=target,scope=SCOPE)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',type=Path,required=True);p.add_argument('--binding',type=Path,required=True);p.add_argument('--source-disk',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--mode',choices=MODES,default='ready');p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args();root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);binding=a.binding.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh native asset evidence directory required')
 bound=binding_check(root,iso,binding);contract=source_contract(root);target=target_check(root)
 if not target['ok']:raise ValueError(str(target))
 peer=AssetPeer(root,a.mode);peer.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-https-asset-{os.getpid()}.sock';command=desktop.native_command(iso,disk,serial,sock,pcap,a.model,a.resolution);profile=https.tls.command_profile(command)
 if not profile['ok']:raise ValueError(str(profile))
 manifest=dict(binding=bound,mode=a.mode,resolution=a.resolution,model=a.model,command=command,host_utc_start=time.time(),source_sha256=contract['source_sha256'],input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),keylog_enabled=False,frames=[],actions=[])
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None
 def text():return serial.read_text(errors='replace')if serial.exists()else ''
 def health():
  if proc.poll()is not None or time.monotonic()-began>300:raise RuntimeError('native asset gate exited/exceeded300s')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(f.exists()and f.stat().st_size>MAX_PCAP for f in(serial,pcap)):raise RuntimeError('raw evidence exceeds bound')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();m=re.search(pattern,text()[start:])
   if m:return m
   time.sleep(.01)
  raise TimeoutError(pattern)
 def key(code,pattern):start=len(text());q.key(code);return wait(pattern,start)
 def picture(name,visible=False):
  path=out/(name+'.ppm');deadline=time.monotonic()+25
  while True:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=text()
   try:
    if name!='closed':verify_pixels(snapshot,path.read_bytes(),visible,tuple(map(int,a.resolution.split('x'))))
    break
   except(AssertionError,ValueError)as exc:
    if time.monotonic()>deadline:raise RuntimeError(str(exc))
    time.sleep(.1)
  manifest['frames'].append(dict(name=name,file=path.name,visible=visible,serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest()))
 try:
  deadline=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,text,key)
  boundary=len(text());key('g',r'\[shell.browser.input\] code=34 down=1 slot=43 accepted=1');wait(r'\[browser.asset.intent\] accepted=1 explicit=1 public_internet=0',boundary)
  if a.mode=='ready':
   wait(r'\[browser.asset.applied\] ticket=2 ',boundary);picture('initial-image',True);key('i',r'\[browser.import.rejected\] reason=asset_not_text effects=0');boundary=len(text());key('q',r'\[browser.asset.clear.intent\]');wait(r'\[browser.asset.cleared\].*positive_ack=1',boundary);picture('cleared-image')
   boundary=len(text());key('g',r'\[browser.asset.intent\] accepted=1');wait(r'\[browser.asset.applied\] ticket=3 ',boundary);picture('fresh-image',True)
   key('grave_accent',r'\[shell.palette.item\] idx=0')
   for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
   boundary=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',boundary);wait(r'\[browser.asset.applied\] ticket=3 ',boundary);picture('zoomed-image',True)
  elif a.mode.startswith('partial-'):
   wait(r'\[browser.asset.progress\] ticket=2 body_received=1024 authenticated=1 body_visible=0',boundary)
   if a.mode=='partial-close':boundary=len(text());q.key_sequence(['f11']);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',boundary);picture('closed')
   else:key('q',r'\[browser.asset.clear.intent\]');wait(r'\[browser.tls.cleanup\] ticket=2 outcome=Cancelled .*positive_release=1',boundary);picture('negative-image')
  else:
   wait(r'\[browser.asset.rejected\] reason=invalid_qoi effects=0'if a.mode=='invalid-qoi'else r'\[browser.asset.view\] state=Unavailable',boundary);picture('negative-image')
  deadline=time.monotonic()+10
  while not peer.completed.wait(.01):
   health()
   if time.monotonic()>deadline:raise TimeoutError('actual endpoint observations incomplete')
 except Exception as exc:failure=repr(exc)
 finally:
  manifest['host_utc_end']=time.time();peer.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 (out/'peer-exchanges.json').write_text(json.dumps(dict(tls=peer.rows,dns=peer.dns,dns_send_attempts=peer.attempts,errors=peer.errors,keylog_enabled=False),indent=2)+'\n');manifest['raw_sha256']={name:hashlib.sha256((out/name).read_bytes()).hexdigest()for name in['serial.log','wire.pcap','peer-exchanges.json']+[f['file']for f in manifest['frames']]if(out/name).exists()};(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 result=dict(ok=False,error=failure,scope=SCOPE)
 if failure is None:
  try:result=verify_evidence(root,iso,binding,out)
  except Exception as exc:result.update(error=repr(exc))
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(result))

if __name__=='__main__':main()
