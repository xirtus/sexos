#!/usr/bin/env python3
"""Controlled QOI endpoint, frozen native capture and independently replayable checks.

Never builds. Endpoint sockets are public lab fixtures, not a forwarding proxy.
State-publication ACKs are joined to raw framebuffer pixels, never called pixels.
"""
from clock_source_evidence import browser_clock_sources
import argparse,ast,hashlib,json,os,re,shutil,socket,subprocess,time
from pathlib import Path
import https_document_gate as https
import tls_responsiveness_gate as desktop
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from launcher_workflow_gate import pointer_park
from clipboard_workflow_gate import hardware_keys
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from check_disk_persistence import ppm_rgb
from check_desktop_pixels import font_from_source,glyph
from check_dns_capture import inspect as dns_capture
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_nonunix_target import inspect as target_check

MODES=('ready','wrong-mime','invalid-qoi','partial-clear','partial-close')
REQUEST=b'GET /art.qoi HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: image/qoi\r\n\r\n'
WIDTH=HEIGHT=48
BASE=(0x30,0x38,0x60)
SOURCE_FILES=(
 'apps/kaleidoscope/src/main.rs','apps/kaleidoscope/src/asset_view.rs','apps/kaleidoscope/src/asset_presentation.rs','apps/kaleidoscope/src/asset_decode.rs',
 'apps/kaleidoscope/src/tls_probe.rs','apps/kaleidoscope/src/tls_exchange.rs','apps/kaleidoscope/src/bitmap_delivery.rs',
 'apps/kaleidoscope/src/browser_command.rs','apps/kaleidoscope/src/document_import.rs',
 'crates/sex-http/src/codec.rs','crates/sex-http/src/lib.rs','crates/sex-tls/src/lib.rs','crates/sex-tls/src/verifier.rs',
 'crates/sex-tls/src/probe_fixture.rs','crates/sex-trust-config/src/lib.rs','crates/sex-net-config/src/lib.rs',
 'crates/sex-graphics/src/qoi.rs','crates/sex-pdx/src/lib.rs','crates/sex-pdx/src/network_job.rs','crates/sex-pdx/src/bitmap_batch.rs','crates/sex-pdx/src/desktop_style.rs',
 'servers/sexnet/src/main.rs','servers/sexnet/src/jobs.rs','servers/sexnet/src/dns_jobs.rs',
 'servers/sexnet/src/trust_policy.rs','servers/sexnet/src/certificate_time.rs','servers/sexnet/src/entropy.rs',
 'kernel/src/app_resources.rs','kernel/src/init.rs','servers/silk-shell/src/main.rs','servers/sexinput/src/main.rs',
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
SCOPE='Actual explicit G on immutable BOOT HTTPS endpoint, authenticated bounded image/qoi, exact positive RELEASE before bounded decode/publication, complete centered Fit/1:1 straight-alpha image and source-font controls in raw PPM, current-generation text/bitmap acknowledgments, prior image-box erasure inside the live owned body, and positive Q/CLEAR erasure. Public controlled root and endpoint only; no TLS key logs, proxy, public Internet, runtime capability mint, arbitrary URL, execution or hostile-content containment claim.'

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
 for literal in ('Mode { Fit, Actual }','columns(w)>=30','{4} else {5}','(chrome_rows(w)as u32)*LINE*scale(w)','Mode::Actual=>1','(available_w-image_w)/2','(available_h-image_h)/2','min(24)','512/(cols+1)', 'F Fit  A 1:1  G fetch  Q clear','QOI / controlled HTTPS lab root'):
  if literal not in view:raise ValueError('source-bound retained image/chrome contract changed: '+literal)
 shell=sources['servers/silk-shell/src/main.rs'].decode()
 if 'req.type_id==0x202&&req.caller_pd==3&&req.caller_task==3'not in sources['apps/kaleidoscope/src/main.rs'].decode()or 'item.destination.subject.pd==15&&status==0'not in shell:raise ValueError('Browser actual exact Shell task/held return guard changed')
 for literal in ('(raw * (screen_dim - 1) / TABLET_RAW_MAX).clamp(0, screen_dim - 1)','const TABLET_RAW_MAX: i32 = 32767;','[shell.abs.normalize] raw_x={} raw_y={} sx={} sy={} accepted={} reason={}'):
  if literal not in shell:raise ValueError('source-bound pointer normalization changed')
 input_source=sources['servers/sexinput/src/main.rs'].decode();physical='sex_pdx::serial_record!("[sexinput.ps2.scancode] raw={:#x}", scancode);'
 retained='input_arrival.ps2(&mut keyboard_outbox,scancode)'
 if physical not in input_source or retained not in input_source or input_source.index(physical)>=input_source.index(retained):raise ValueError('physical PS2 make receipt must be atomic and precede retained key admission')
 for literal in ('code:(raw&0x7f)as u8,down:(raw&0x80==0)as u8','send(edge.code as u64,edge.down as u64,EV_KEY)','poll_input_delivery(&mut keyboard_outbox,&mut pointer_outbox,send_shell_hid_event)','pdx_call_checked(SLOT_SHELL, OP_HID_EVENT, arg0, arg1, arg2)'):
  if literal not in input_source:raise ValueError('retained physical key must drain through the actual held Shell route: '+literal)
 if 'min(4)'not in view or '0x00303860u32'not in display:raise ValueError('source-bound fit/background contract changed')
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
 try:sources=browser_clock_sources(trace)
 except AssertionError as exc:raise ValueError(str(exc))from exc
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
  if[int(policy[1]),int(clock[1]),int(seed[1]),int(release[1]),int(service[1])]!=[i,i,i,i,i+1]:raise ValueError('prerequisite/duplex incarnation not fresh')
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
 if len(g_inputs)!=len(intents):raise ValueError('explicit G lacks exact held-shell accepted key count')
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
  ikey=re.search(r'\[shell.browser.input\] code=23 down=1 slot=43 accepted=1',trace[released[0].end():clear.start()]);qkey=re.search(r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1',trace[reject.end():intents[1].start()])
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

def image_geometry(tile,mode='Fit'):
 if mode not in ('Fit','Actual'):raise ValueError('unknown image mode')
 factor=2 if tile['w']>=256 else 1;cols=min(79,max(0,(tile['w']-16)//(6*factor)-1));chrome=4 if cols>=30 else 5
 top=36+chrome*9*factor+8;aw=tile['w']-16;ah=tile['h']-top-8
 scale=min(aw//48,ah//48,4)if mode=='Fit'else 1
 if cols<1 or scale<1 or 48*scale>aw or 48*scale>ah:raise ValueError('actual Browser viewport cannot fit fixture')
 return dict(x=8+(aw-48*scale)//2,y=top+(ah-48*scale)//2,width=48,height=48,scale=scale,mode=mode,cols=cols,chrome_rows=chrome)

PRESENT_LAYOUT=r'\[browser.asset.presentation.layout\] generation=(\d+) mode=(Fit|Actual) viewport=(\d+) ticket=(\d+) sequence=(\d+) x=(\d+) y=(\d+) width=(\d+) height=(\d+) scale=(\d+) hash=(0x[0-9a-f]+)'
PRESENT_READY=r'\[browser.asset.presentation.ready\] generation=(\d+) mode=(Fit|Actual) viewport=(\d+) ticket=(\d+) sequence=(\d+) hash=(0x[0-9a-f]+) positive_ack=1 pixels=unconfirmed'
PRESENT_INTENT=r'\[browser.asset.presentation.intent\] generation=(\d+) mode=(Fit|Actual) viewport=(\d+) ticket=(\d+) network=0'
PRESENT_TEXT=r'\[browser.asset.presentation.text\] generation=(\d+) viewport=(\d+) sequence=(\d+) bytes=(\d+) rows=(\d+) positive_ack=1'

def geometry(trace,require_ack=True):
 tile,viewport=https.viewport(trace)
 layouts=records(trace,'[browser.asset.presentation.layout]',PRESENT_LAYOUT)
 current=layouts[-1]if layouts else None;mode=current[2]if current and int(current[3])==viewport else 'Fit'
 expected=image_geometry(tile,mode);expected.update(viewport=viewport,tile=tile)
 if not require_ack:return expected
 if not current:raise ValueError('current image presentation layout missing')
 m=current;generation=int(m[1]);ticket=int(m[4]);sequence=int(m[5]);g=expected
 if generation<1 or int(m[3])!=viewport or tuple(map(int,m.groups()[5:10]))!=tuple(g[k]for k in('x','y','width','height','scale'))or m[11]!=hex(PIXEL_HASH):raise ValueError('presentation differs from actual source geometry')
 oldlayouts=list(re.finditer(rf'\[browser.asset.layout\] ticket={ticket} viewport={viewport} x={g["x"]} y={g["y"]} width=48 height=48 scale={g["scale"]} hash={hex(PIXEL_HASH)} pixels=unconfirmed',trace[:m.start()]));oldlayout=oldlayouts[-1]if oldlayouts else None
 release=re.search(rf'\[browser.asset.released\] ticket={ticket} .*positive_release=1 authenticated=1',trace[:m.start()]);decode=re.search(rf'\[browser.asset.decoded\] ticket={ticket} width=48 height=48 pixels=2304 hash={hex(PIXEL_HASH)} bounded_turn=128',trace[:m.start()])
 if not oldlayout or not release or not decode or not release.start()<decode.start()<oldlayout.start()<m.start():raise ValueError('layout lacks released bounded decode/legacy geometry')
 text_acks=records(trace,'[browser.asset.presentation.text]',PRESENT_TEXT)
 text_ack=next((a for a in text_acks if tuple(map(int,a.groups()[:2]))==(generation,viewport)),None)
 if not text_ack or text_ack.start()>oldlayout.start():raise ValueError('bitmap precedes current-generation text clearing')
 frame=asset_text(tile,mode,'Publishing image');text_seq,nbytes,nrows=map(int,text_ack.groups()[2:])
 if(nbytes,nrows)!=(frame['bytes'],frame['rows']):raise ValueError('presentation text extent differs')
 if not re.search(rf'\[browser.asset.text.applied\] viewport={viewport} sequence={text_seq} bytes={nbytes} rows={nrows} positive_ack=1 pixels=unconfirmed',trace[:text_ack.start()])or not re.search(rf'\[display.text.committed\] target=15 surface=300 paint={text_seq} bytes={nbytes} changed=[01] pixels=unconfirmed',trace[:text_ack.start()]):raise ValueError('text ACK lacks legacy/display-owned commit')
 ready=records(trace,'[browser.asset.presentation.ready]',PRESENT_READY)
 matching=[r for r in ready if r.groups()==(str(generation),mode,str(viewport),str(ticket),str(sequence),hex(PIXEL_HASH))]
 if len(matching)!=1 or matching[0].start()<m.end():raise ValueError('missing/duplicate current presentation Ready')
 end=matching[0].start();legacy=rf'\[browser.asset.applied\] ticket={ticket} viewport={viewport} sequence={sequence} x={g["x"]} y={g["y"]} width=48 height=48 scale={g["scale"]} hash={hex(PIXEL_HASH)} positive_ack=1 pixels=unconfirmed'
 if not re.search(legacy,trace[m.end():end])or not re.search(rf'\[display.bitmap.committed\] target=15 surface=300 paint={sequence} width=48 height=48 scale={g["scale"]} pixels=unconfirmed',trace[m.end():end]):raise ValueError('Ready lacks exact bitmap Applied/display commit')
 intents=records(trace,'[browser.asset.presentation.intent]',PRESENT_INTENT)
 if intents and int(intents[-1][1])>generation:raise ValueError('old Ready superseded by newer presentation intent')
 if trace.rfind('[browser.asset.cleared]')>m.start():raise ValueError('image superseded by CLEAR')
 expected.update(generation=generation,ticket=ticket,sequence=sequence)
 return expected

def asset_text(tile,mode,status='Image ready'):
 g=image_geometry(tile,mode);scale=2 if tile['w']>=256 else 1;cols=g['cols'];rows=min(24,max(0,1+(tile['h']-36-7*scale)//(9*scale)),512//(cols+1))
 lines=['QOI / controlled HTTPS lab root',status,('Fit'if mode=='Fit'else'1:1')+f' 48x48 / {g["scale"]}x']
 lines+=['F Fit  A 1:1  G fetch  Q clear']if g['chrome_rows']==4 else ['F Fit  A 1:1','G fetch  Q clear']
 return dict(lines=[line[:cols].ljust(cols)for line in(lines+['']*rows)[:rows]],rows=rows,cols=cols,bytes=rows*(cols+1)-1,scale=scale,chrome_rows=g['chrome_rows'])

def verify_pixels(trace,ppm,visible=True,resolution=None,contract=None):
 """Full source-derived RGBA box, guard, old-box erasure and complete chrome."""
 g=geometry(trace,visible);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
 if resolution and(width,height)!=tuple(resolution):raise ValueError('framebuffer resolution differs')
 tile=g['tile'];data=decode_fixture(BODY);checked=colored=erased=0
 if tile['x']<0 or tile['y']<50 or tile['w']<=0 or tile['h']<=0 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('actual Browser frame lies outside framebuffer')
 coordinates={(tile['x']+x,tile['y']+y)for y in range(g['y']-1,g['y']+48*g['scale']+1)for x in range(g['x']-1,g['x']+48*g['scale']+1)}
 # Inspect every historic image rectangle intersecting the CURRENT owned image
 # area. Pixels outside this live Browser body may belong to another surface.
 top=36+g['chrome_rows']*9*(2 if tile['w']>=256 else 1)+8
 for m in re.finditer(PRESENT_LAYOUT,trace):
  prior=trace[:m.start()];old_tile,_=https.viewport(prior);x,y,_,_,scale=map(int,m.groups()[5:10])
  for sy in range(old_tile['y']+y,old_tile['y']+y+48*scale):
   for sx in range(old_tile['x']+x,old_tile['x']+x+48*scale):
    if tile['x']+8<=sx<tile['x']+tile['w']-8 and tile['y']+top<=sy<tile['y']+tile['h']-8:coordinates.add((sx,sy))
 for sx,sy in coordinates:
  lx=sx-tile['x'];ly=sy-tile['y']
  if not(8<=lx<tile['w']-8 and 36<=ly<tile['h']-8):continue
  inside=g['x']<=lx<g['x']+48*g['scale']and g['y']<=ly<g['y']+48*g['scale'];expected=BASE
  if visible and inside:
   at=(((ly-g['y'])//g['scale'])*48+(lx-g['x'])//g['scale'])*4;r,green,b,a=data[at:at+4];expected=tuple((fg*a+bg*(255-a)+127)//255 for fg,bg in zip((r,green,b),BASE));colored+=expected!=BASE
  else:erased+=1
  actual=tuple(pixels[(sy*width+sx)*3:(sy*width+sx)*3+3]);checked+=1
  if actual!=expected:raise ValueError(f'RGBA/old rectangle/guard pixel differs local=({lx},{ly}) wanted={expected} actual={actual}')
 text_pixels=0
 if visible:
  contract=contract or source_contract(Path(__file__).resolve().parents[1]);frame=asset_text(tile,g['mode']);color=(0xcd,0xd6,0xf4);scale=frame['scale']
  for row,line in enumerate(frame['lines'][:frame['chrome_rows']]):
   for column,char in enumerate(line+' '):
    bitmap=glyph(contract['font'],char)
    for gy in range(9):
     for gx in range(6):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        sx=tile['x']+8+(column*6+gx)*scale+dx;sy=tile['y']+36+(row*9+gy)*scale+dy
        if sx>=tile['x']+tile['w']-8 or sy>=tile['y']+tile['h']-8:continue
        actual=tuple(pixels[(sy*width+sx)*3:(sy*width+sx)*3+3]);text_pixels+=1
        if actual!=(color if on else BASE):raise ValueError('complete image status/metrics/help glyph or padding differs')
 if not checked or visible and not colored:raise ValueError('no complete visible image evidence')
 return dict(ok=True,checked_pixels=checked,colored_pixels=colored,erased_pixels=erased,text_pixels=text_pixels,geometry=g,visible=visible)

def frame_plan(mode):
 return [('initial-image',True,2),('actual-image',True,2),('refitted-image',True,2),('cleared-image',False,None),('fresh-image',True,3),('zoomed-image',True,3),('zoomed-actual',True,3),('restored-actual',True,3),('restored-fit',True,3)]if mode=='ready'else [('closed',False,None)]if mode=='partial-close'else [('negative-image',False,None)]

def validate_frame_stages(trace,mode,frames):
 plan=frame_plan(mode)
 layouts=records(trace,'[browser.asset.presentation.layout]',PRESENT_LAYOUT);ready=records(trace,'[browser.asset.presentation.ready]',PRESENT_READY);text_acks=records(trace,'[browser.asset.presentation.text]',PRESENT_TEXT)
 if mode=='ready':
  if len(layouts)!=8 or len(ready)!=8:raise ValueError('missing/extra retained presentation layout/Ready')
  seen=set()
  for layout,ack in zip(layouts,ready):
   key=layout.groups()[:5]+(layout[11],)
   if key!=ack.groups()or not layout.start()<ack.start()or key[0]in seen:raise ValueError('foreign/stale/duplicate/reordered presentation Ready')
   seen.add(key[0])
  current=[(a[1],a[2])for a in text_acks]
  if len(current)!=len(set(current)):raise ValueError('duplicate current-generation text receipt')
 elif layouts or ready:raise ValueError('negative asset published presentation')
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
 if mode=='ready':
  if [g['mode']for g in geometries]!=['Fit','Actual','Fit','Fit','Fit','Actual','Actual','Fit']:raise ValueError('retained Fit/Actual modes differ')
  vp=[g['viewport']for g in geometries]
  if not(vp[0]==vp[1]==vp[2]==vp[3]and vp[4]==vp[5]>vp[3]and vp[6]==vp[7]>vp[5]):raise ValueError('retained/Zoom/restore viewports differ')
  if any(a['sequence']>=b['sequence']or a['generation']>=b['generation']for a,b in zip(geometries,geometries[1:])):raise ValueError('publication generation/sequence did not advance')
  expected_controls=[('Actual',30,geometries[1],1),('Fit',33,geometries[2],2),('Actual',30,geometries[5],6),('Fit',33,geometries[7],8)]
  intents=records(trace,'[browser.asset.presentation.intent]',PRESENT_INTENT)
  if len(intents)!=4:raise ValueError('unexpected/missing explicit retained control')
  for intent,(mode,code,g,index)in zip(intents,expected_controls):
   if intent.groups()!=(str(g['generation']),mode,str(g['viewport']),str(g['ticket'])):raise ValueError('control intent belongs to another presentation')
   start=frames[index-1]['serial_end'];end=frames[index]['serial_end']
   keys=list(re.finditer(rf'\[shell.browser.input\] code={code} down=1 slot=43 accepted=1',trace[start:end]))if start<intent.start()else []
   layout=next(a for a in layouts if int(a[1])==g['generation']);ack=next(a for a in ready if int(a[1])==g['generation'])
   if len(keys)!=1 or not start<intent.start()<layout.start()<ack.start()<end:raise ValueError('retained control lacks exact fresh accepted-key/intent/layout/Ready window')
  for code in(30,33):
   if len(re.findall(rf'\[shell.browser.input\] code={code} down=1 slot=43 accepted=1',trace))!=2:raise ValueError('extra retained control key')

KEY_CODES={'g':34,'a':30,'f':33,'i':23,'q':16,'grave_accent':41,'k':37,'ret':28,'tab':15,'scroll_lock':70,'f11':87,'down':80}
BROWSER_EFFECTS={'g':r'\[browser.asset.intent\] accepted=1 explicit=1 public_internet=0','a':r'\[browser.asset.presentation.intent\] generation=\d+ mode=Actual viewport=\d+ ticket=\d+ network=0','f':r'\[browser.asset.presentation.intent\] generation=\d+ mode=Fit viewport=\d+ ticket=\d+ network=0','i':r'\[browser.import.rejected\] reason=asset_not_text effects=0','q':r'\[browser.asset.clear.intent\] pending=[01] owned_storage=erased'}
PHYSICAL=r'\[sexinput.ps2.scancode\] raw=(0x[0-9a-f]+)(?=\r?\n|$)'

def capture_key_action(q,code,pattern,text,wait,record):
 if code not in KEY_CODES or code=='down':raise ValueError('unsupported live hardware key')
 start=len(text());action=dict(key=code,code=KEY_CODES[code],kind='browser'if code in BROWSER_EFFECTS else'close'if code=='f11'else'menu',transport='input-send-event',serial_start=start,host_start=time.monotonic());record(action)
 hardware_keys(q,(code,));action.update(serial_qmp_return=len(text()),host_qmp_return=time.monotonic())
 wait(rf'\[sexinput.ps2.scancode\] raw={hex(KEY_CODES[code])}(?:\r?\n|$)',start)
 if code in BROWSER_EFFECTS:wait(BROWSER_EFFECTS[code],start);wait(rf'\[shell.browser.input\] code={KEY_CODES[code]} down=1 slot=43 accepted=1',start)
 result=wait(pattern,start);action.update(serial_end=len(text()),host_end=time.monotonic());return result

def verify_key_actions(trace,frames,actions,mode,resolution):
 """Actual QMP window -> pre-forward PS2 make -> handler; return may race.

 Shell logs accepted after pdx_call. Its callee may already have run: the
 accepted receipt is required in the exact action window without ordering it
 before that callee. The indivisible PS2 make precedes forwarding in source.
 """
 if not isinstance(actions,list)or not actions:raise ValueError('actual QMP key windows missing')
 final=frames[-1]['serial_end'];previous=0;live=[];makes=[];held=[];checks=[];menu_k=0
 for action in actions:
  code=action.get('key');kind=action.get('kind');start=action.get('serial_start');returned=action.get('serial_qmp_return');end=action.get('serial_end')
  if code not in KEY_CODES or action.get('code')!=KEY_CODES[code]or kind not in('boot','browser','menu','close'):raise ValueError('foreign QMP key/action kind')
  if any(type(x)is not int for x in(start,returned,end))or not previous<=start<=returned<=end<=final:raise ValueError('QMP key serial windows overlap/reverse/outside capture')
  if not all(isinstance(action.get(k),(int,float))for k in('host_start','host_qmp_return','host_end'))or not action['host_start']<=action['host_qmp_return']<=action['host_end']:raise ValueError('missing/reversed actual QMP return measurements')
  previous=end
  if kind=='boot':
   if resolution!='1024x768'or code not in('down','ret')or live or action.get('transport')!='send-key':raise ValueError('foreign late/unsupported boot-menu key')
   continue
  if action.get('transport')!='input-send-event'or end<=start:raise ValueError('missing actual key hardware event window')
  live.append(code);segment=trace[start:end];physical=records(segment,'[sexinput.ps2.scancode]',PHYSICAL);keydown=[m for m in physical if int(m[1],16)<128]
  if len(keydown)!=1 or int(keydown[0][1],16)!=KEY_CODES[code]:raise ValueError('missing/duplicate/foreign/out-of-window physical PS2 make')
  make=start+keydown[0].start();make_end=start+keydown[0].end();makes.append(make)
  if kind=='browser':
   if code not in BROWSER_EFFECTS:raise ValueError('non-Browser key assigned held Browser authority')
   handlers=list(re.finditer(BROWSER_EFFECTS[code],segment));accepted=list(re.finditer(rf'\[shell.browser.input\] code={KEY_CODES[code]} down=1 slot=43 accepted=1',segment))
   if len(handlers)!=1 or len(accepted)!=1 or not make_end<start+handlers[0].start():raise ValueError('physical make/handler/held-route return causal join differs')
   if make_end>=start+accepted[0].start():raise ValueError('held return preceded physical make')
   held.append(start+accepted[0].start())
   for focus in re.finditer(rf'\[silk-shell.key.recv\] code={KEY_CODES[code]} down=1 mod=(\d+) focused=(\d+)',segment):
    if focus.groups()!=('0','300')or start+focus.start()>start+handlers[0].start():raise ValueError('available Shell key receipt has foreign focus/modifiers/order')
   checks.append(dict(key=code,physical=make,handler=start+handlers[0].start(),held_return=start+accepted[0].start(),callee_before_caller_receipt=handlers[0].start()<accepted[0].start()))
  else:
   if code in BROWSER_EFFECTS or(kind=='close')!=(code=='f11'):raise ValueError('Browser key lacks held-route join')
   patterns={'grave_accent':r'\[shell.palette.item\] idx=0','k':r'\[shell.palette.select\] old=(?:0|9) new=(?:9|8)','ret':r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1','tab':r'\[shell.focus.set\] id=\d+','scroll_lock':r'\[shell.action.spindle\] toggle','f11':r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1'}
   if code=='k':
    old,new=(0,9)if menu_k%2==0 else(9,8);patterns[code]=rf'\[shell.palette.select\] old={old} new={new}';menu_k+=1
   if code not in patterns or not re.search(patterns[code],trace[make:end]):raise ValueError('menu/Close key lacks actual post-make semantic handler')
 first=live.index('g')if'g'in live else len(live)
 if any(code not in('tab','scroll_lock')for code in live[:first]):raise ValueError('unplanned startup focus input')
 expected=['g','a','f','i','q','g','grave_accent','k','k','ret','a','grave_accent','k','k','ret','f']if mode=='ready'else['g','q']if mode=='partial-clear'else['g','f11']if mode=='partial-close'else['g']
 if live[first:]!=expected:raise ValueError('finite physical key action order differs')
 boots=[a['key']for a in actions if a['kind']=='boot']
 if boots!=(['down','ret']if resolution=='1024x768'else[]):raise ValueError('boot-menu hardware request profile differs')
 start=next(a['serial_start']for a in actions if a['kind']!='boot');actual=[m.start()for m in records(trace,'[sexinput.ps2.scancode]',PHYSICAL)if start<=m.start()<final and int(m[1],16)<128]
 if actual!=makes:raise ValueError('extra/missing/reordered physical key outside recorded QMP windows')
 actual_held=[m.start()for m in records(trace,'[shell.browser.input]',r'\[shell.browser.input\] code=\d+ down=1 slot=43 accepted=1')if start<=m.start()<final]
 if actual_held!=sorted(held):raise ValueError('extra/foreign/out-of-window held Browser return')
 for code,pattern in BROWSER_EFFECTS.items():
  actual_handlers=[m.start()for m in re.finditer(pattern,trace[:final])];expected_handlers=[c['handler']for c in checks if c['key']==code]
  if actual_handlers!=expected_handlers:raise ValueError('extra/missing/out-of-window Browser handler event')
 return dict(ok=True,physical_keys=len(makes),browser_keys=len(held),checks=checks,scope='QMP windows and atomic pre-forward PS2 makes establish causality; post-call held-route receipts may occur before or after App events.')

def verify_parking(trace,frames,actions,resolution):
 """Fresh source-normalized safe pointer intent before EACH actual capture."""
 if len(actions)!=len(frames):raise ValueError('missing/extra per-capture pointer parking')
 pattern=r'\[shell.abs.normalize\] raw_x=(\d+) raw_y=(\d+) sx=(\d+) sy=(\d+) accepted=([01]) reason=([a-z_]+)'
 previous=0;checks=[]
 for phase,(shot,action)in enumerate(zip(frames,actions)):
  x=resolution[0]-20-phase%2;y=10
  if action.get('kind')!='park'or(action.get('x'),action.get('y'))!=(x,y)or action.get('capture')!=shot['name']:raise ValueError('foreign/unsafe pointer park intent')
  start=action.get('serial_start');end=action.get('serial_end');boundary=shot['serial_end']
  if any(not isinstance(v,int)for v in(start,end,boundary))or not previous<=start<end<=boundary<=len(trace):raise ValueError('pointer parking outside capture window')
  raw_x=round(x*32767/(resolution[0]-1));raw_y=round(y*32767/(resolution[1]-1));sx=raw_x*(resolution[0]-1)//32767;sy=raw_y*(resolution[1]-1)//32767
  rows=list(re.finditer(pattern,trace[start:boundary]));receipt=[m for m in rows if start+m.end()<=end and tuple(map(int,m.groups()[:4]))==(raw_x,raw_y,sx,sy)and(m[5],m[6])in(('1','ok'),('0','duplicate_sample'))]
  if not receipt:raise ValueError('safe pointer park lacks actual normalization receipt')
  # Later samples must keep the safe position through the screenshot; never
  # mask a cursor or infer movement from transport admission alone.
  for row in rows:
   if start+row.start()>=end and tuple(map(int,row.groups()[2:4]))!=(sx,sy):raise ValueError('pointer moved after per-capture park')
  checks.append(dict(capture=shot['name'],x=sx,y=sy,receipt_offset=start+receipt[-1].start()));previous=boundary
 return dict(ok=True,captures=len(checks),checks=checks)

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
 parking=verify_parking(trace,manifest['frames'],manifest['actions'],tuple(map(int,manifest['resolution'].split('x'))))
 key_actions=verify_key_actions(trace,manifest['frames'],manifest['key_actions'],mode,manifest['resolution'])
 for shot in manifest['frames']:
  prefix=trace[:shot['serial_end']]
  if hashlib.sha256(prefix.encode()).hexdigest()!=shot['serial_sha256']:raise ValueError('frame/serial boundary differs')
  if shot['name']=='closed':
   width,height,_=ppm_rgb((evidence/shot['file']).read_bytes(),min_width=1,min_height=1)
   if(width,height)!=tuple(map(int,manifest['resolution'].split('x'))):raise ValueError('Close capture framebuffer resolution differs')
   continue
  if shot['name']=='cleared-image'and '[browser.asset.cleared]'not in prefix:raise ValueError('erasure frame lacks positive CLEAR')
  shots.append(verify_pixels(prefix,(evidence/shot['file']).read_bytes(),shot['visible'],tuple(map(int,manifest['resolution'].split('x'))),contract))
 if mode=='ready'and shots[0]['geometry']['viewport']==shots[-1]['geometry']['viewport']:raise ValueError('actual viewport/scale transition missing')
 authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace);target=target_check(Path(root))
 if not all(r['ok']for r in(authority,waiting,target)):raise ValueError('native authority/self-wait/non-Unix target check failed')
 return dict(ok=True,binding=bound,mode=mode,wire=wire,trace=joins,pixels=shots,pointer_parking=parking,key_actions=key_actions,authority=authority,self_wait=waiting,target=target,scope=SCOPE)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',type=Path,required=True);p.add_argument('--binding',type=Path,required=True);p.add_argument('--source-disk',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--mode',choices=MODES,default='ready');p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args();root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);binding=a.binding.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh native asset evidence directory required')
 bound=binding_check(root,iso,binding);contract=source_contract(root);target=target_check(root)
 if not target['ok']:raise ValueError(str(target))
 peer=AssetPeer(root,a.mode);peer.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-https-asset-{os.getpid()}.sock';command=desktop.native_command(iso,disk,serial,sock,pcap,a.model,a.resolution);profile=https.tls.command_profile(command)
 if not profile['ok']:raise ValueError(str(profile))
 manifest=dict(binding=bound,mode=a.mode,resolution=a.resolution,model=a.model,command=command,host_utc_start=time.time(),source_sha256=contract['source_sha256'],input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),keylog_enabled=False,frames=[],actions=[],key_actions=[])
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;park_phase=0
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
 def key(code,pattern):return capture_key_action(q,code,pattern,text,wait,manifest['key_actions'].append)
 def boot_key(code):
  action=dict(key=code,code=KEY_CODES[code],kind='boot',transport='send-key',serial_start=len(text()),host_start=time.monotonic());manifest['key_actions'].append(action);q.key(code);action.update(serial_qmp_return=len(text()),host_qmp_return=time.monotonic());action.update(serial_end=len(text()),host_end=time.monotonic())
 def picture(name,visible=False):
  nonlocal park_phase
  pointer_park(q,tuple(map(int,a.resolution.split('x'))),text,wait,manifest['actions'].append,park_phase);park_phase+=1
  manifest['actions'][-1].update(capture=name,serial_end=len(text()))
  path=out/(name+'.ppm');deadline=time.monotonic()+25
  while True:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=text()
   try:
    if name!='closed':verify_pixels(snapshot,path.read_bytes(),visible,tuple(map(int,a.resolution.split('x'))),contract)
    break
   except(AssertionError,ValueError)as exc:
    if time.monotonic()>deadline:
     (out/'failed-frame.json').write_text(json.dumps(dict(name=name,file=path.name,serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest(),screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),error=str(exc)),indent=2)+'\n');raise RuntimeError(str(exc))
    time.sleep(.1)
  manifest['frames'].append(dict(name=name,file=path.name,visible=visible,serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest()))
 try:
  deadline=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);boot_key('down');boot_key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,text,key)
  boundary=len(text());key('g',r'\[shell.browser.input\] code=34 down=1 slot=43 accepted=1');wait(r'\[browser.asset.intent\] accepted=1 explicit=1 public_internet=0',boundary)
  if a.mode=='ready':
   wait(r'\[browser.asset.presentation.ready\].*ticket=2 ',boundary);picture('initial-image',True)
   for code,name in [('a','actual-image'),('f','refitted-image')]:
    boundary=len(text());key(code,r'\[browser.asset.presentation.intent\]');wait(r'\[browser.asset.presentation.ready\].*ticket=2 ',boundary);picture(name,True)
   key('i',r'\[browser.import.rejected\] reason=asset_not_text effects=0');boundary=len(text());key('q',r'\[browser.asset.clear.intent\]');wait(r'\[browser.asset.cleared\].*positive_ack=1',boundary);picture('cleared-image')
   boundary=len(text());key('g',r'\[browser.asset.intent\] accepted=1');wait(r'\[browser.asset.presentation.ready\].*ticket=3 ',boundary);picture('fresh-image',True)
   key('grave_accent',r'\[shell.palette.item\] idx=0')
   for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
   boundary=len(text());key('ret',r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1');wait(r'\[browser.asset.presentation.ready\].*ticket=3 ',boundary);picture('zoomed-image',True)
   boundary=len(text());key('a',r'\[browser.asset.presentation.intent\]');wait(r'\[browser.asset.presentation.ready\].*ticket=3 ',boundary);picture('zoomed-actual',True)
   key('grave_accent',r'\[shell.palette.item\] idx=0')
   for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
   boundary=len(text());key('ret',r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1');wait(r'\[browser.asset.presentation.ready\].*ticket=3 ',boundary);picture('restored-actual',True)
   boundary=len(text());key('f',r'\[browser.asset.presentation.intent\]');wait(r'\[browser.asset.presentation.ready\].*ticket=3 ',boundary);picture('restored-fit',True)
  elif a.mode.startswith('partial-'):
   wait(r'\[browser.asset.progress\] ticket=2 body_received=1024 authenticated=1 body_visible=0',boundary)
   if a.mode=='partial-close':boundary=len(text());key('f11',r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1');picture('closed')
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
