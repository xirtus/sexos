#!/usr/bin/env python3
"""Representative connected desktop + exact same-image recovery; never builds.

This finite workflow does not replace alpha_session's 1800s/12-cycle policy.
All app input is real QMP hardware, never an injected application operation.
"""
import argparse,gzip,hashlib,json,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
import clipboard_workflow_gate as clip
import http_content_gate as http
import https_asset_gate as asset
import https_import_gate as storage
import launcher_workflow_gate as launch
import quil_readability_gate as readability
import spindle_durable_reboot_gate as history
import tls_exchange_gate as tls
import tls_responsiveness_gate as desktop
from desktop_close_gate import QMP
from check_document_disk import recover,fnv
from check_desktop_pixels import final_view,verify as linen_pixels
from check_quil_ui_pixels import view as quil_view,verify as quil_pixels
from check_spindle_viewport_pixels import verify as spindle_pixels
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP

NAME='connote';BODY=http.READER_BODY;FINAL=BODY+b'z';COMMAND='echo connected108';DRAFT=b'draft'
SERIAL_UNITS=http.WHEEL_SERIAL_UNITS
LIVE_FRAMES=('quil-network-wait','image-fit','image-actual','reader','reader-down','reader-up','reader-hidden','reader-returned','browser-copied','quil-edited','quil-saved','linen-selected','quil-reopened','spindle-recalled','spindle-brought')
COLD_FRAMES=('quil-cold','spindle-cold')
SCOPE=('Representative connected session, not full alpha acceptance: one live boot and exact same-image/terminal-disk cold boot per profile. '
       'Actual fixed public-lab-root TLS QOI response is withheld while real Quil typing completes; this proves network-wait overlap, not simultaneous CPU execution. '
       'Positive image Clear precedes separate plainHTTP2048 reader/USB wheel; explicit clipboard replacement, append z, typed SaveAs2049, Linen reopen; '
       'exact Spindle12 history barrier/readback and same-instance workspace BringHere, then ordered exact-incarnation app Close and cold document/history reopen. '
       'All authority remains existing BOOT routes in GLOBAL_VAS. No publicweb/publicCA, arbitrary endpoint, Unix/POSIX, sustained1800s alpha, power-loss/crash, physical/SMP or hostile-isolation claim.')
sha=launch.sha
KEY_CODES={char:code for base,letters in((0x10,'qwertyuiop'),(0x1e,'asdfghjkl'),(0x2c,'zxcvbnm'),(2,'1234567890'))for code,char in enumerate(letters,base)}
KEY_CODES.update(ctrl=0x1d,shift=0x2a,alt=0x38,tab=0x0f,ret=0x1c,spc=0x39,grave_accent=0x29,up=0x48,down=0x50,end=0x4f,home=0x47,f11=0x57)

def source_contract(root):
 root=Path(root);parts=dict(text=clip.source_contract(root),wheel=http.load_wheel_contract(root),image=asset.source_contract(root),launcher=launch.source_contract(root),quil=readability.source_contract(root),import_view=storage.source_contract(root));hashes={}
 for value in parts.values():
  for name,digest in value['source_sha256'].items():
   if name in hashes and hashes[name]!=digest:raise ValueError('conflicting connected source hash: '+name)
   hashes[name]=digest
 extra=('servers/sexfiles/src/spindle_history.rs','servers/sexfiles/src/history_disk.rs','servers/sexfiles/src/history_journal.rs','crates/sex-pdx/src/spindle_history.rs','crates/sex-pdx/src/history_recovery.rs','apps/spindle/src/history_persistence.rs','scripts/connected_desktop_gate.py','scripts/tests/test_connected_desktop_gate.py','scripts/spindle_durable_reboot_gate.py')
 for name in extra:
  path=root/name
  if not path.is_file():raise ValueError('connected dependency missing: '+name)
  hashes[name]=sha(path.read_bytes())
 for name in asset.script_dependencies(root,('scripts/connected_desktop_gate.py','scripts/tests/test_connected_desktop_gate.py')):hashes[name]=sha((root/name).read_bytes())
 parts.update(source_sha256=hashes,name_keys=clip.name_keyboard(NAME,(root/'servers/quil/src/main.rs').read_text()));return parts

def one(trace,pattern):
 values=list(re.finditer(pattern,trace))
 if len(values)!=1:raise ValueError('missing/duplicate exact receipt: '+pattern)
 return values[0]

def history_state(disk,required=True):
 try:meta=history.snapshot(disk)
 except RuntimeError as exc:
  if required or str(exc)!='no committed boot A journal snapshot' or any(disk[1808*512:(1808+18)*512]):raise
  return dict(commands=[],generation=0)
 offset=(1808+9*meta['bank'])*512;payload=disk[offset+512:offset+512+meta['size']];pos=24;commands=[]
 for _ in range(meta['count']):
  n=int.from_bytes(payload[pos:pos+2],'little');pos+=2;commands.append(payload[pos:pos+n].decode('ascii'));pos+=n
 if pos!=len(payload):raise ValueError('history payload not exactly covered')
 return dict(meta,commands=commands,payload_sha256=sha(payload),payload_hex=payload.hex())

def history_join(trace,expected,recovery=False):
 kind='loaded'if recovery else'saved';size=expected['size'];fingerprint=expected['fingerprint']
 receipt=one(trace,rf'\[spindle.history.receipt\] target=12 task=12 kind={kind} handle=(\d+) bytes={size} count={expected["count"]} hash={fingerprint} error=0')
 handle=int(receipt[1]);opened=one(trace,rf'\[files.history.open\] target=12 handle={handle} mode={"load"if recovery else"save"} seq=(\d+)');closed=one(trace,rf'\[files.history.close\] target=12 handle={handle} seq=(\d+) ok=1')
 if handle==0 or not opened.start()<closed.start()<receipt.start()or int(closed[1])<=int(opened[1]):raise ValueError('history handle/open/close/receipt order differs')
 if recovery:
  effect=one(trace,rf'\[files.history.recovered\] logical=12 source=12 source_task=12 target=12 generation={expected["generation"]} bytes={size} source_hash={fingerprint} owned_hash={fingerprint} data_hash={expected["data_hash"]} boot_grant=53')
  if effect.start()>=opened.start():raise ValueError('cold history open preceded BOOT recovery')
 else:
  effect=one(trace,rf'\[files.history.durable\] logical=12 target=12 generation={expected["generation"]} bank={expected["bank"]} bytes={size} hash={fingerprint} barriers=3 readback=1')
  commit=one(trace,rf'\[files.history.commit\] target=12 handle={handle} seq=(\d+) bytes={size} hash={fingerprint}')
  if not opened.start()<effect.start()<commit.start()<closed.start()or not int(opened[1])<int(commit[1])<int(closed[1]):raise ValueError('history durable readback/commit/close order differs')
 return dict(handle=handle,receipt=receipt.start(),effect=effect.start(),generation=expected['generation'])

def verify_keys(trace,actions):
 previous=0;makes=[]
 for action in actions:
  if action['kind']!='key':continue
  keys=action['keys'];start=action['serial_start'];end=action['serial_end'];returned=action['serial_qmp_return']
  if not 1<=len(keys)<=3 or len(set(keys))!=len(keys)or any(k not in KEY_CODES for k in keys):raise ValueError('unsupported actual hardware key/chord')
  if action['transport']!='input-send-event'or action['events']!=[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for down,ks in((True,keys),(False,list(reversed(keys))))for k in ks]:raise ValueError('key is not exact QMP hardware down/up')
  if any(type(n)is not int for n in(start,returned,end))or not previous<=start<=returned<=end<=len(trace):raise ValueError('key windows overlap/out of bounds')
  if not action['host_start']<=action['host_qmp_return']<=action['host_end']:raise ValueError('key host ordering differs')
  matches=list(re.finditer(r'\[sexinput.ps2.scancode\] raw=(0x[0-9a-f]+)(?=\n|$)',trace[start:end]));actual=[(start+m.start(),int(m[1],16))for m in matches if int(m[1],16)<128]
  if [code for _,code in actual]!=[KEY_CODES[k]for k in keys]:raise ValueError('missing/foreign/duplicate physical PS2 key')
  makes.extend(pos for pos,_ in actual);previous=end
 if not makes:raise ValueError('missing real keyboard inputs')
 first=next(a['serial_start']for a in actions if a['kind']=='key');actual=[m.start()for m in re.finditer(r'\[sexinput.ps2.scancode\] raw=(0x[0-9a-f]+)(?=\n|$)',trace)if int(m[1],16)<128 and m.start()>=first]
 if actual!=makes:raise ValueError('unrecorded physical key outside QMP window')
 return dict(ok=True,physical_makes=len(makes))

def inspect_wire(raw,peer,request,recovery=False):
 flows,packets=tls.reassemble(raw);by_port={}
 for flow in flows:by_port.setdefault(flow['peer_port'],[]).append(flow)
 plain=sorted(by_port.get(46078,[]),key=lambda f:f['guest_port']);encrypted=by_port.get(46080,[])
 if len(flows)!=(1 if recovery else 3)or len(plain)!=(1 if recovery else 2)or len(encrypted)!=(0 if recovery else 1)or len(peer['plain'])!=len(plain)or len(peer['tls'])!=len(encrypted):raise ValueError('connected multi-resource TCP closure differs')
 for index,(flow,row)in enumerate(zip(plain,peer['plain'])):
  expected_request=tls.ECHO if index==0 else request;expected_response=tls.ECHO if index==0 else http.wire_response('reader',BODY);ticket=1 if index==0 else 3
  if(flow['guest_port'],flow['outgoing'],flow['incoming'])!=(48000+ticket,expected_request,expected_response)or row['request_hex']!=expected_request.hex()or row['response_hex']!=expected_response.hex():raise ValueError('exact connected Echo/plainHTTP bytes differ')
  if not flow['fin_out']or not flow['fin_in']or flow['rst_out']or flow['rst_in']:raise ValueError('plain connected flow incomplete/reset')
 image=[]
 if not recovery:
  flow=encrypted[0];row=peer['tls'][0]
  if flow['guest_port']!=48002 or(row['sni'],row['tls_version'],row['cipher'])!=(tls.HOST,'TLSv1.3',tls.CIPHER)or row['handshake_complete']is not True or row['unwrap_completed']is not True:raise ValueError('actual independent TLS endpoint handshake/close_notify absent')
  if row['request_hex']!=asset.REQUEST.hex()or row['response_hex']!=asset.response('ready').hex()or row['application_bytes_received']!=len(asset.REQUEST)or row['extra_application_bytes']!=0:raise ValueError('actual SSL image request/body differs')
  if not flow['fin_out']or not flow['fin_in']or flow['rst_out']or flow['rst_in']:raise ValueError('image encrypted flow lacks clean transfer')
  outgoing=tls.tls_records(flow['outgoing']);incoming=tls.tls_records(flow['incoming']);client=tls.hello(outgoing[0],True);tls.hello(incoming[0],False)
  if not all(r['type']in(20,23)for r in outgoing[1:]+incoming[1:])or not any(r['type']==23 for r in outgoing)or not any(r['type']==23 for r in incoming):raise ValueError('TLS plaintext/missing application records')
  if any(token in stream for stream in(flow['outgoing'],flow['incoming'])for token in(asset.REQUEST,asset.BODY,b'GET /art.qoi',b'HTTP/1.1')):raise ValueError('image plaintext leaked onto TLS wire')
  image=[dict(ticket=2,initial_tls_bytes=outgoing[0]['length']+5,outgoing_bytes=len(flow['outgoing']),incoming_bytes=len(flow['incoming']),client_hello=client)]
 dns=http.dns_capture(raw,'echo',attempts=peer['attempts'],expected=peer['dns'])if not recovery else no_cold_dns(raw)
 if not dns['ok']or len(peer['dns'])!=(0 if recovery else 2)or len(peer['attempts'])!=len(peer['dns']):raise ValueError('connected DNS closure differs')
 return dict(ok=True,flows=len(flows),packets=packets,image=dict(joins=image),dns=dns)

def no_cold_dns(raw):
 # reassemble() has already checked IPv4 headers; a cold fixture permits no
 # guest UDP at all, including an unanswered or malformed DNS datagram.
 for frame in tls.frames(raw):
  if len(frame)>=34 and frame[12:14]==b'\x08\0':
   ip=frame[14:]
   if ip[9]==17 and (ip[12:16]==tls.LOCAL or ip[16:20]==tls.LOCAL):raise ValueError('cold read-only session captured guest UDP/DNS')
 return dict(ok=True,queries=[],responses=[],packets=[],scope='No captured guest IPv4 UDP datagram in the exact cold PCAP; not merely an empty endpoint list.')

class ConnectedPeer(tls.ControlledPeer):
 def __init__(self,root,request,recovery=False):
  super().__init__(root,'ready');self.request=request;self.recovery=recovery;self.plain=[];self.request_received=threading.Event();self.release_image=threading.Event()
 def serve_echo(self,listener):
  for index in range(1 if self.recovery else 2):
   sock=self.accept(listener)
   if sock is None:return
   data=bytearray()
   while True:
    part=sock.recv(4096)
    if not part:break
    data.extend(part)
    if len(data)>4096:raise ValueError('connected request oversized')
   expected=tls.ECHO if index==0 else self.request;response=tls.ECHO if index==0 else http.wire_response('reader',BODY)
   if bytes(data)!=expected:raise ValueError('connected actual request differs')
   for offset in range(0,len(response),1024):sock.sendall(response[offset:offset+1024])
   self.plain.append(dict(request_hex=bytes(data).hex(),response_hex=response.hex()));sock.shutdown(socket.SHUT_WR);sock.close()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('extra connected plaintext flow')
   except socket.timeout:pass
 def serve_dns(self,sock):
  while not self.stop.is_set():
   try:data,remote=sock.recvfrom(513)
   except socket.timeout:continue
   ticket=len(self.dns)+1
   if self.recovery or ticket>2 or data!=http.dns_query(ticket):raise ValueError('extra/foreign connected DNS query')
   reply=http.dns_response(ticket);sent=sock.sendto(reply,remote);self.dns.append(dict(ticket=ticket,response_hex=reply.hex()));self.attempts.append(dict(ticket=ticket,kind='answer',response_hex=reply.hex(),sent_bytes=sent))
 def serve_tls(self,listener,ctx):
  if self.recovery:
   while not self.stop.is_set():
    try:extra,_=listener.accept();extra.close();raise ValueError('cold read-only recovery opened TLS')
    except socket.timeout:pass
   return
  sock=self.accept(listener)
  if sock is None:return
  wrapped=ctx.wrap_socket(sock,server_side=True);self.live.append(wrapped);row=dict(handshake_complete=True,sni=getattr(wrapped,'controlled_sni',None),tls_version=wrapped.version(),cipher=wrapped.cipher()[0],extra_application_bytes=0,unwrap_completed=False);self.rows.append(row);data=bytearray()
  while len(data)<len(asset.REQUEST):
   part=wrapped.recv(512)
   if not part:raise ValueError('image request ended early')
   data.extend(part)
  if bytes(data)!=asset.REQUEST:raise ValueError('fixed connected image request differs')
  row.update(request_hex=bytes(data).hex(),application_bytes_received=len(data),host_request_received=time.monotonic());self.request_received.set();deadline=time.monotonic()+120
  while not self.release_image.wait(.05):
   if self.stop.is_set():return
   if time.monotonic()>deadline:raise TimeoutError('bounded connected response hold')
  row['host_response_release']=time.monotonic();wire=asset.response('ready')
  for offset in range(0,len(wire),1024):wrapped.sendall(wire[offset:offset+1024])
  row['response_hex']=wire.hex();plain=wrapped.unwrap();row['unwrap_completed']=True;plain.close();self.completed.set()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('extra connected TLS flow')
   except socket.timeout:pass

def appended_history(commands):
 result=(list(commands)+[COMMAND])[-128:]
 while 24+sum(2+len(s.encode('ascii'))for s in result)>4096:result.pop(0)
 return result

def exact_cleanup(trace,target,task,start):
 result=cleanup_receipts(trace,target,task,start);mask=0;roles={}
 for pos,row in launch.rows(trace[start:],'app.service.ack'):
  if row.get('target')!=str(target)or row.get('task')!=str(task):continue
  holder,role,acked,required=(int(row[k])for k in('holder','role','acked','required'))
  if role in roles or {1:3,2:1,4:11,8:13,16:10}.get(role)!=holder or required!=31 or acked!=mask|role:raise ValueError('cleanup holder/role/replay/accumulation differs')
  mask=acked;roles[role]=start+pos
 retired=one(trace[start:],rf'\[app.private.retired\] target={target} task={task} [^\n]*acked=31 required=31 all_flushed=1 pool_reusable=1')
 if set(roles)!={1,2,4,8,16}or not max(roles.values())<start+retired.start()<result['pool_offset']:raise ValueError('private cleanup preceded complete terminal custody')
 request=one(trace[start:],rf'\[shell.app.close.request\] target={target} task={task} surface={ {12:153,15:300,9:201}[target]}(?=\s|$)')
 if start+request.start()>=min(roles.values()):raise ValueError('cleanup not preceded by exact held Close request')
 if target==12:
  discarded=one(trace[start:],r'\[files.history.retire\] target=12 staged=0 pending=0 terminal=1')
  if start+discarded.start()>=roles[4]:raise ValueError('history custody survived Files cleanup ACK')
 return result

def no_cold_history_write(seed,disk):
 if seed[1808*512:(1808+18)*512]!=disk[1808*512:(1808+18)*512]:raise ValueError('cold recovery wrote history bank/header/padding')
 if history_state(seed)!=history_state(disk):raise ValueError('cold semantic history changed')
 return True

def command_before_history(trace,expected):
 command=one(trace,r'\[spindle.instance.command\] target=12 task=12 name=echo ok=1');proof=history_join(trace,expected)
 if command.start()>=proof['effect']:raise ValueError('history effect preceded command dispatch')
 return proof

FAULTS=r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial\.record\.overflow|\[fault\.kill\]|unexpected[_ ]user[_ ]fault|\[sexfiles\.defer\.drop\]'
def reject_faults(trace):
 if re.search(FAULTS,trace,re.I):raise ValueError('guest fault/deferred drop/diagnostic overflow')

def command_profile(manifest,iso):
 command=manifest['command'];resolution=manifest['resolution'];model=manifest['model']
 if resolution not in('1280x800','1024x768')or model not in('e1000e','e1000')or command[command.index('-cdrom')+1]!=str(Path(iso).resolve())or not tls.command_profile(command)['ok']:raise ValueError('actual sealed native TLS/CPU/RTC profile differs')
 if command.count('usb-tablet,bus=xhci.0')!=1 or command.count('nec-usb-xhci,id=xhci')!=1 or command.count('VGA,xres='+resolution.replace('x',',yres='))!=1 or command.count(model+',netdev=peer,mac=52:54:00:12:34:56')!=1:raise ValueError('actual device/NIC/framebuffer profile differs from manifest')
 return True

def overlay_join(trace,anchor=0):
 layout=one(trace,r'\[browser.import.layout\] state=NoDocument [^\n]*')
 refused=one(trace,r'\[browser.clipboard.refused\] ticket=(\d+) reason=reader_hidden effects=0')
 stopped=one(trace,rf'\[browser.clipboard.stopped\] ticket={refused[1]} state=Unsupported payload_erased=1')
 dismissed=one(trace,rf'\[browser.import.dismissed\] source=http anchor={anchor} bytes={len(BODY)} restored=1 network=0 selection_preserved=1')
 if int(refused[1])==0 or not layout.start()<refused.start()<stopped.start()<dismissed.start():raise ValueError('hidden reader refusal/cancellation/return order differs')
 if re.search(r'\[browser.clipboard.(?:copy|published)\]|\[browser.reader.wheel\]|\[browser.http.document\]|\[net.(?:job|dns).egress.submit\]',trace[layout.end():dismissed.start()]):raise ValueError('hidden reader exported/scrolled/refetched')
 return dict(refused_ticket=int(refused[1]),dismissed=dismissed.start())

def mixed_drains(trace,raw):
 flows,_=tls.reassemble(raw);captured={f['guest_port']-48000:f for f in flows};policies={};previous={};samples=[]
 patterns={2:(10,32768,r'\[net.job.duplex.admitted\] owner=15 task=15 ticket=2 kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route'),3:(9,13312,r'\[net.job.stream.admitted\] owner=15 task=15 ticket=3 kind=9 ring=1024 max_wire=13312 total_ticks=30000 idle_ticks=5000 authority=held_route')}
 for ticket,(kind,maximum,pattern)in patterns.items():
  admitted=one(trace,pattern);policies[ticket]=(kind,maximum,admitted.start())
 for tag in('net.job.duplex.admitted','net.job.stream.admitted'):
  if trace.count('['+tag+']')!=1:raise ValueError('missing/foreign/duplicate mixed data admission')
 lines=list(re.finditer(r'\[net.job.stream.drain\] owner=15 task=15 ticket=(\d+) produced=(\d+) consumed=(\d+) buffered=(\d+) ring=1024 state_before=(Receiving|Complete) receipt=retained not_delivery=1(?=\n|$)',trace))
 if len(lines)!=trace.count('[net.job.stream.drain]'):raise ValueError('malformed/foreign mixed READ sample')
 terminal=trace.find('[net.app.cleanup.ack] target=15 task=15')
 complete=set()
 for line in lines:
  ticket,produced,consumed,buffered=map(int,line.groups()[:4]);state=line[5]
  if ticket not in policies or ticket not in captured:raise ValueError('READ lacks actual prior ticket/kind/wire')
  kind,maximum,offset=policies[ticket];flow=captured[ticket]
  if flow['peer_port']!=(46080 if kind==10 else 46078)or line.start()<offset or terminal>=0 and line.start()>terminal:raise ValueError('READ protocol/admission/retirement order differs')
  if not 0<consumed<=produced<=min(maximum,len(flow['incoming']))or buffered!=produced-consumed or buffered>1024:raise ValueError('READ counters/ring/captured bytes differ')
  old=previous.get(ticket)
  if old and not(produced>=old[0]and consumed>old[1])or ticket in complete and state!='Complete':raise ValueError('READ counters/state repeated or regressed')
  if state=='Complete':complete.add(ticket)
  previous[ticket]=(produced,consumed);samples.append(dict(ticket=ticket,kind=kind,produced=produced,consumed=consumed))
 if set(previous)!={2,3}:raise ValueError('each admitted mixed flow requires actual READ samples')
 return dict(ok=True,samples=samples,scope='All mixed kind10/kind9 sampled destructive reads, exact owner/task/ticket/admission, respective bounds and captured incoming bytes; no filtering or delivery inference.')

def check_document(trace,disk,seed,recovery=False,expected=None):
 if recovery:
  meta=expected;actual=recover(disk,meta['object'])
  if actual['payload']!=FINAL or actual['identity']!=meta['identity']:raise ValueError('cold raw document bytes/identity differ')
  if storage.workspace(seed)!=storage.workspace(disk):raise ValueError('read-only cold open mutated workspace')
  for row in storage.workspace(seed)['rows']:
   if row['object']>=2 and storage.object_bytes(seed,row['object'])!=storage.object_bytes(disk,row['object']):raise ValueError('cold read changed document journal')
  if '[files.document.durable] caller=9 'in trace:raise ValueError('cold reopen rewrote document')
 else:
  meta=storage.import_disk(seed,disk,NAME,FINAL)
  durable=one(trace,rf'\[files.document.durable\] caller=9 object={meta["object"]} identity={meta["identity"]} generation={meta["generation"]} bank={meta["bank"]} bytes={len(FINAL)} hash={hex(fnv(FINAL))} barriers=3 readback=1')
  saved=one(trace,rf'\[quil.persist.save.ok\] bytes={len(FINAL)} hash={hex(fnv(FINAL))} doc={meta["object"]}(?=\s|$)');clean=one(trace,r'\[quil.document.saved\] saved_revision=(\d+) revision=(\d+) dirty=0')
  if not durable.start()<saved.start()<clean.start()or clean[1]!=clean[2]:raise ValueError('Save lacks exact durable effect/current revision acknowledgement')
 loads=list(re.finditer(rf'\[quil.persist.load.ok\] bytes={len(FINAL)} hash={hex(fnv(FINAL))} doc={meta["object"]}(?=\s|$)',trace))
 if len(loads)!=1:raise ValueError('missing/duplicate exact user-selected load')
 opened=one(trace,r'\[shell.document.open\] target=(?:9|15) linen=200')
 selected=list(re.finditer(r'\[linen.object_select.current\] selected=(\d+)',trace))
 # The complete Linen pixel oracle separately binds the selected visible name.
 if opened.start()>=loads[0].start():raise ValueError('load preceded explicit Files UI')
 return meta

def inspect_session(trace,peer,wire,raw,c,manifest,evidence,seed,disk,before_save=None):
 cold=manifest['recovery'];expected=evidence.get('expected_document');doc=check_document(trace,disk,seed if cold else before_save,cold,expected);h=history_state(disk)
 if cold:
  no_cold_history_write(seed,disk)
  if re.search(r'\[browser.(?:asset.intent|http.document|clipboard.copy)\]|\[quil.clipboard.paste\]|\[files.history.durable\]',trace):raise ValueError('cold reopen repeated source/export/storage mutation')
 else:
  copied=clip.clipboard_join(trace,BODY)
  auth=one(trace,r'\[browser.asset.authenticated\] ticket=2 profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1')
  admit=one(trace,r'\[browser.asset.admitted\] ticket=(2) dns_generation=(1) seq=(\d+) task_bytes=(\d+) private_bytes=(\d+) profile=1 authority=held_route');intent=one(trace,r'\[browser.asset.intent\] accepted=1 explicit=1 public_internet=0')
  asset.prerequisites(trace,[intent],[admit],wire['image'],manifest['host_utc_start'],manifest['host_utc_end'])
  if int(admit[5])!=int(admit[4])+8248+12288:raise ValueError('image private byte accounting differs')
  parsed=one(trace,rf'\[browser.asset.body\] ticket=2 bytes={len(asset.BODY)} hash={hex(asset.fingerprint(asset.BODY))} parsed=1 authenticated=1 visible=0');flow=wire['image']['joins'][0]
  released=one(trace,rf'\[browser.asset.released\] ticket=2 tx={flow["outgoing_bytes"]} rx={flow["incoming_bytes"]} body_bytes={len(asset.BODY)} body_hash={hex(asset.fingerprint(asset.BODY))} positive_release=1 authenticated=1')
  decoded=one(trace,rf'\[browser.asset.decoded\] ticket=2 width=48 height=48 pixels=2304 hash={hex(asset.PIXEL_HASH)} bounded_turn=128')
  clear=one(trace,r'\[browser.asset.cleared\] viewport=(\d+) sequence=(\d+) positive_ack=1')
  commit=one(trace,rf'\[display.bitmap.committed\] target=15 surface=300 paint={clear[2]} width=0 height=0 scale=0 pixels=unconfirmed')
  http_doc=one(trace,rf'\[browser.http.document\] ticket=3 path=/welcome status=200 private_bytes={len(BODY)} positive_release=1')
  if not intent.start()<admit.start()<auth.start()<parsed.start()<released.start()<decoded.start()<commit.start()<clear.start()<http_doc.start()<copied['copy_offset']:raise ValueError('connected image Clear/document/export ownership sequence differs')
  edits=list(re.finditer(r'\[quil.document.edit\] revision=(\d+) pending=0 bytes=(\d+)',trace[auth.end():parsed.start()]))
  if [int(m[2])for m in edits]!=list(range(1,len(DRAFT)+1)):raise ValueError('actual Quil typing did not complete during image response hold')
  held=evidence['network_wait'];row=peer['tls'][0]
  if not auth.start()<=held['serial_start']<held['serial_end']<=parsed.start()or not row['host_request_received']<=held['host_start']<=held['host_end']<row['host_response_release']:raise ValueError('actual endpoint response was not held through accepted Quil input')
  appended=one(trace,rf'\[quil.document.edit\] revision=\d+ pending=0 bytes={len(FINAL)}(?=\s|$)')
  if copied['paste_offset']>=appended.start():raise ValueError('append preceded atomic clipboard replacement')
  hidden=overlay_join(trace)
  if hidden['dismissed']>=copied['copy_offset']:raise ValueError('copy occurred before original retained reader returned')
  if h['commands']!=appended_history(history_state(seed,False)['commands'])or h['generation']<=history_state(seed,False)['generation']:raise ValueError('history not exact seed plus bounded appended command')
  command_before_history(trace,h)
  # Exactly one live original instance is opened and brought to scene1.
  actions=launch.completed_actions(trace);spindle=[r for r in actions if r['target']==12]
  if[(r['task'],r['frame'],r['action'],r['origin'],r['to'],r['activated'])for r in spindle]!=[(12,153,'Open',0,0,0),(12,153,'BringHere',0,1,0)]:raise ValueError('Spindle launcher/workspace changed incarnation or activated reserve')
  one(trace,r'\[shell.scene.switch\] from=0 to=1')
  if '[shell.app.relaunch]'in trace:raise ValueError('representative same-instance path unexpectedly activated reserve')
  joined=http.resolution_joins(trace,raw,[(2,1),(3,2)],wire['dns']);mixed_drains(trace,raw)
  if not joined['ok']:raise ValueError(str(joined['errors']))
 history_proof=history_join(trace,h,cold)
 grant='[boot.history.recovery] holder=11 slot=53 logical=12 original=12 task=12 successor=17 successor_task=17 base=1808 sectors=18 runtime_mint=0'
 if trace.count(grant)!=1:raise ValueError('missing exact BOOT history recovery grant')
 cleanup=[]
 for close in evidence['closes']:
  if(close['target'],close['task'])not in((12,12),(15,15),(9,9)):raise ValueError('foreign cleanup subject')
  cleanup.append(exact_cleanup(trace,close['target'],close['task'],close['serial_start']))
 if[(r['target'],r['task'])for r in cleanup]!=[(12,12),(15,15),(9,9)]:raise ValueError('missing/reordered connected app Close')
 scene_return_join(trace,cleanup,cold)
 if not cold:
  network_retired=next(r['ack_offset']for r in cleanup if r['target']==15)
  if re.search(r'\[net.(?:job|dns).egress.submit\]',trace[network_retired:]):raise ValueError('retired Browser emitted network traffic')
 return dict(ok=True,document=doc,history=h,history_receipt=history_proof,cleanup=cleanup)

def scene_return_join(trace,cleanup,cold=False):
 switches=list(re.finditer(r'\[shell.scene.switch\] from=(\d+) to=(\d+)(?=\n|$)',trace))
 expected=[]if cold else[('0','1'),('1','0')]
 if [m.groups()for m in switches]!=expected:raise ValueError('connected workspace switch/return sequence differs')
 if not cold:
  browser_close=one(trace, r'\[shell.app.close.request\] target=15 task=15 surface=300(?=\s|$)')
  if not cleanup[0]['pool_offset']<switches[1].start()<browser_close.start():raise ValueError('scene0 return must follow Close12 and precede Close15')
 return True

def historical_launcher_erasure(trace,image,c,resolution,required=False):
 tags=('launcher.open','shell.palette.open','launcher.text.view','launcher.text.row','launcher.text.admitted','shell.palette.item','launcher.action','command_palette.close')
 if not any('['+tag+']'in trace for tag in tags):
  if required:raise ValueError('post-Spindle frame lacks actual launcher antecedent')
  return None
 # No prior launcher is a valid boot state. Any partial antecedent instead
 # requires the complete source-defined opening, layout and actual close.
 opens=list(re.finditer(rf'\[launcher.open\] count={c["launcher_app_count"]} selected=0 ok=1(?=\n|$)',trace));shell=list(re.finditer(rf'\[shell.palette.open\] ok=1 selected=0 count={c["launcher_count"]}(?=\n|$)',trace))
 if not opens or not shell or len(opens)!=trace.count('[launcher.open]')or len(shell)!=trace.count('[shell.palette.open]'):raise ValueError('incomplete/malformed launcher opening antecedent')
 launch.launcher_layout(trace);last=trace.rfind('[launcher.text.view]')
 if last<max(opens[-1].start(),shell[-1].start())or not any(p>last for p,_ in launch.rows(trace,'command_palette.close')):raise ValueError('latest launcher lacks complete layout/close before app pixels')
 return readability.launcher_erased(trace,image,c,resolution)

def verify_frame(trace,image,frame,c,resolution,doc=None):
 kind=frame['kind'];name=frame['name']
 if kind=='image':
  result=asset.verify_pixels(trace,image,True,resolution,c['image']);geometry=asset.geometry(trace)
  if geometry['mode']!=frame['mode']:raise ValueError('image Fit/Actual mode differs')
  return result
 if kind=='overlay':
  expected=empty_import_geometry(trace,c['import_view']);result=asset.https.frame_pixels(image,expected,c['import_view'],resolution)
 elif kind=='selected':
  result=clip.verify_browser(trace,image,BODY,c['text'],frame['anchor'],(0,len(BODY)),'Unsupported',resolution)
  if not result['ok']:raise ValueError(str(result['errors']))
 elif kind=='reader':
  result=http.verify_reader_pixels(trace,image,BODY,c['wheel'],frame['anchor'],resolution)
  if not result['ok']:raise ValueError(str(result['errors']))
 elif kind=='copied':
  result=clip.verify_browser(trace,image,BODY,c['text'],frame['anchor'],(0,len(BODY)),'Copied',resolution)
  if not result['ok']:raise ValueError(str(result['errors']))
 elif kind=='quil':
  errors=quil_pixels(trace,image,c['text']['font'],None,target=9,resolution=resolution);v=quil_view(trace)
  required=DRAFT.decode()if name=='quil-network-wait'else FINAL[-min(28,len(FINAL)):].decode()
  if required not in ''.join(v['text'].split('\n')):errors.append('independent exact Quil draft/tail missing')
  if name!='quil-network-wait'and v['cursor']!=len(FINAL):errors.append('Quil complete source caret differs')
  if errors:raise ValueError(str(errors))
  result=dict(ok=True,geometry=v)
 elif kind=='linen':
  v,labels=final_view(trace);errors=linen_pixels(trace,image,c['text']['font'],resolution)
  if errors or doc is None or not any(identity==v['selected']and label==NAME for identity,_,label in labels):raise ValueError('actual full Linen saved-name selection/pixels differ: '+str(errors))
  result=dict(ok=True,geometry=v)
 elif kind=='spindle':
  errors=spindle_pixels(trace,image,c['launcher']['font'],target=12,require_prompt='> '+COMMAND,require_echo=False,require_all_ink=False,resolution=resolution)
  if errors:raise ValueError(str(errors))
  result=dict(ok=True,geometry=launch.spindle_geometry(trace,12))
 else:raise ValueError('unsupported connected frame kind')
 if kind in('quil','linen','spindle'):historical_launcher_erasure(trace,image,c['quil'],resolution,kind=='spindle')
 return result

def empty_import_geometry(trace,c):
 tile,viewport=asset.https.viewport(trace);scale=2 if tile['w']>=c['scale_threshold']else 1;cols=min(79,max(0,(tile['w']-2*c['inset'])//(c['advance']*scale)-1));available=tile['h']-c['top']-7*scale;rows=min(32,512//(cols+1),1+available//(c['line']*scale))if cols and available>=0 else 0
 if rows<1:raise ValueError('NoDocument import status cannot fit actual tile')
 lines=[bytearray(b' '*cols)for _ in range(rows)];source=c['import_labels']['NoDocument']+b'\n\n'+c['import_hints'].get('NoDocument',c['import_default_hint']);row=col=0
 for byte in source:
  if byte==10:row+=1;col=0;continue
  if col==cols:row+=1;col=0
  if row>=rows:break
  lines[row][col]=byte if 32<=byte<=126 else 63;col+=1
 payload=b'\n'.join(lines);n=len(payload);digest=hex(fnv(payload));code=storage.STATE_CODES['NoDocument'];layout=one(trace,rf'\[browser.import.layout\] state=NoDocument name= bytes={n} cols={cols} rows={rows} viewport={viewport} hash={digest} pixels=unconfirmed')
 acks=list(re.finditer(rf'\[browser.import.applied\] bytes={n} hash={digest} rows={rows} viewport={viewport} state_code={code} sequence=(\d+) positive_ack=1',trace[layout.end():]))
 if not acks or not re.search(rf'\[browser.view.applied\] bytes={n} http={1024|(code<<11)} body_bytes=0 pixels=unconfirmed',trace[layout.end():]):raise ValueError('empty-name NoDocument frame lacks exact immutable positive Applied')
 return dict(tile=tile,scale=scale,cols=cols,rows=rows,bytes=n,hash=digest,lines=[bytes(line)for line in lines],viewport=viewport)

def frame_semantic(trace,frame,doc=None):
 name=frame['name']
 if name in('quil-reopened','quil-cold'):
  if doc is None:raise ValueError('reopen frame lacks exact durable document metadata')
  one(trace,rf'\[quil.persist.load.ok\] bytes={len(FINAL)} hash={hex(fnv(FINAL))} doc={doc["object"]}(?=\s|$)')
 elif name=='quil-saved':
  one(trace,rf'\[quil.persist.save.ok\] bytes={len(FINAL)} hash={hex(fnv(FINAL))} doc={doc["object"]}(?=\s|$)');clean=one(trace,r'\[quil.document.saved\] saved_revision=(\d+) revision=(\d+) dirty=0')
  if clean[1]!=clean[2]:raise ValueError('saved frame not current clean revision')
 elif name=='reader-returned':overlay_join(trace,frame['anchor'])
 elif name=='spindle-brought':
  matching=[r for r in launch.completed_actions(trace)if(r['target'],r['task'],r['frame'],r['action'],r['origin'],r['to'],r['activated'])==(12,12,153,'BringHere',0,1,0)]
  if len(matching)!=1:raise ValueError('brought frame predates actual same-instance BringHere')
 elif name in('spindle-recalled','spindle-cold'):
  one(trace,rf'\[spindle.history.nav\] dir=up idx=0 len={len(COMMAND)} ok=1')
  kind='loaded'if name=='spindle-cold'else'saved'
  if not re.search(rf'\[spindle.history.receipt\] target=12 task=12 kind={kind} [^\n]*error=0',trace):raise ValueError('Spindle recall frame preceded exact history completion')
 return True

def frame_contract(frames,cold=False):
 expected=COLD_FRAMES if cold else LIVE_FRAMES
 if [f['name']for f in frames]!=list(expected):raise ValueError('missing/reordered finite connected frames')
 kinds={name:'quil'for name in expected}
 kinds.update({'image-fit':'image','image-actual':'image','reader':'reader','reader-down':'reader','reader-up':'reader','reader-hidden':'overlay','reader-returned':'selected','browser-copied':'copied','linen-selected':'linen','spindle-recalled':'spindle','spindle-brought':'spindle','spindle-cold':'spindle'})
 for frame in frames:
  if frame['kind']!=kinds[frame['name']]or frame['screenshot']!=frame['name']+'.ppm':raise ValueError('frame substituted another app oracle/file')
  if frame['kind']=='image'and frame['mode']!=('Fit'if frame['name']=='image-fit'else'Actual'):raise ValueError('image mode phase differs')
 return True

def replay_boot(root,iso,binding,out,c=None):
 root=Path(root).resolve(strict=True);out=Path(out).resolve(strict=True);iso=Path(iso).resolve(strict=True);binding=Path(binding).resolve(strict=True);c=c or source_contract(root);m=json.loads((out/'manifest.json').read_text());bound=frozen_binding(root,iso,binding)
 if m['binding']!=bound or m['source_sha256']!=c['source_sha256']or m['scope']!=SCOPE or m['serial_offset_units']!=SERIAL_UNITS or m['serial_prefix_hash_encoding']!='UTF-8':raise ValueError('connected source/image/character convention binding differs')
 if m.get('error')is not None:raise ValueError('failed native attempt cannot be accepted by replay')
 command_profile(m,iso)
 if m['keylog_enabled']is not False:raise ValueError('TLS secrets capture enabled')
 frames=json.loads((out/'frames.json').read_text());actions=json.loads((out/'actions.json').read_text());evidence=json.loads((out/'evidence.json').read_text());peer=json.loads((out/'peer-exchanges.json').read_text());cold=m['recovery']
 if peer['errors']:raise ValueError('independent endpoint reported failure')
 frame_contract(frames,cold)
 required={'serial.log','wire.pcap','peer-exchanges.json','actions.json','frames.json','evidence.json','seed.img.gz','nvme.img.gz'}|{f['screenshot']for f in frames}
 if not cold:required.add('before-save.img.gz')
 if set(m['raw_sha256'])!=required:raise ValueError('connected raw evidence closure differs')
 for name,digest in m['raw_sha256'].items():
  path=(out/name).resolve(strict=True)
  if not path.is_relative_to(out)or (out/name).is_symlink()or sha(path.read_bytes())!=digest:raise ValueError('raw member changed/unsafe: '+name)
 seed=gzip.decompress((out/'seed.img.gz').read_bytes());disk=gzip.decompress((out/'nvme.img.gz').read_bytes())
 if(sha(seed),sha(disk))!=(m['input_disk_sha256'],m['final_disk_sha256']):raise ValueError('exact raw seed/terminal disk chain differs')
 trace=http.serial_text(out/'serial.log');reject_faults(trace);raw=(out/'wire.pcap').read_bytes();wire=inspect_wire(raw,peer,c['text']['request'],cold);keys=verify_keys(trace,actions)
 joined=inspect_session(trace,peer,wire,raw,c,m,evidence,seed,disk,None if cold else gzip.decompress((out/'before-save.img.gz').read_bytes()))
 f11=[a for a in actions if a.get('kind')=='key'and a.get('keys')==['f11']]
 if len(f11)!=3 or any(a['serial_start']!=close['serial_start']for a,close in zip(f11,evidence['closes'])):raise ValueError('terminal cleanup lacks exact three non-replayed hardware Close windows')
 if len(evidence['wheel'])!=(0 if cold else 2):raise ValueError('missing/extra finite connected wheel actions')
 wheel=[]
 for index,action in enumerate(evidence['wheel']):
  if(action['name'],action['wheel'],action['target'])!=[("down",-1,"body"),("up",1,"body")][index]:raise ValueError('connected wheel order/target differs')
  if index and action['before']!=evidence['wheel'][index-1]['after']:raise ValueError('connected wheel lost retained source anchor')
  wheel.append(http.inspect_wheel_action(trace,action,BODY,c['wheel'],tuple(map(int,m['resolution'].split('x')))))
 for kind in http.WHEEL_PATTERNS:
  if len(http.wheel_rows(trace,kind))!=sum(len(http.wheel_rows(trace[a['serial_start']:a['serial_end']],kind))for a in evidence['wheel']):raise ValueError('extra wheel effect outside actual QMP windows')
 pixels=[];previous=0
 for frame in frames:
  prefix=http.frame_prefix(trace,frame);end=frame['serial_end']
  if end<previous or sha((out/frame['screenshot']).read_bytes())!=frame['screenshot_sha256']:raise ValueError('frame/hash phase differs')
  frame_semantic(prefix,frame,joined['document'])
  if frame['name']in('reader-down','reader-up'):
   action=evidence['wheel'][0 if frame['name']=='reader-down'else 1]
   if end<action['serial_end']or frame['anchor']!=action['after']:raise ValueError('wheel pixel frame predates actual input/retained anchor')
  pixels.append(verify_frame(prefix,(out/frame['screenshot']).read_bytes(),frame,c,tuple(map(int,m['resolution'].split('x'))),joined['document']));previous=end
 parks=[a for a in actions if a['kind']=='park'];asset.verify_parking(trace,frames,parks,tuple(map(int,m['resolution'].split('x'))))
 for check in(nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True),wait_check(trace)):
  if not check['ok']:raise ValueError('connected NIC/self-wait regression: '+str(check.get('errors')))
 return dict(ok=True,acceptance='representative_connected_only',full_alpha_acceptance=False,binding=bound,boots=1,frames=len(frames),recovery=cold,wire=wire,keys=keys,wheel=wheel,joins=joined,scope=SCOPE)


def run_boot(root,iso,binding,seed,out,c,model,resolution_text,recovery=False,expected=None):
 out=Path(out).resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh connected boot output required')
 bound=frozen_binding(root,iso,binding);held=json.loads(binding.read_text())
 for name,digest in c['source_sha256'].items():
  if held.get('sources',{}).get(name,held.get('runner_sha256',{}).get(name))!=digest:raise ValueError('connected dependency omitted from sealed binding: '+name)
 storage.wait_ports(out);seed_bytes=seed.read_bytes()
 if not recovery and any(row['name']==NAME for row in storage.workspace(seed_bytes)['rows']):raise ValueError('connected name already present in source disk')
 disk=out/'nvme.img';disk.write_bytes(seed_bytes);(out/'seed.img.gz').write_bytes(gzip.compress(seed_bytes,mtime=0));serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-connected-{os.getpid()}.sock';resolution=tuple(map(int,resolution_text.split('x')));command=desktop.native_command(iso,disk,serial,sock,pcap,model,resolution_text);peer=ConnectedPeer(root,c['text']['request'],recovery)
 m=dict(binding=bound,source_sha256=c['source_sha256'],scope=SCOPE,recovery=recovery,command=command,model=model,resolution=resolution_text,serial_offset_units=SERIAL_UNITS,serial_prefix_hash_encoding='UTF-8',keylog_enabled=False,input_disk_sha256=sha(seed_bytes),qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),host_utc_start=time.time(),error=None)
 (out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n');frames=[];actions=[];evidence=dict(expected_document=expected,network_wait=None,wheel=[],closes=[]);began=time.monotonic();proc=None;failure=None;park_phase=0;doc=expected
 def text():return http.serial_text(serial)if serial.exists()else''
 def health():
  if time.monotonic()-began>900:raise TimeoutError('bounded900second connected boot')
  if proc is not None and proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise ValueError('bounded connected raw evidence exceeded')
  reject_faults(text())
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.01)
  raise TimeoutError(pattern)
 def key(*keys,pattern=None):
  start=len(text());event=dict(kind='key',keys=list(keys),transport='input-send-event',events=[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for down,ks in((True,keys),(False,tuple(reversed(keys))))for k in ks],serial_start=start,host_start=time.monotonic());actions.append(event);clip.hardware_keys(q,keys);event.update(serial_qmp_return=len(text()),host_qmp_return=time.monotonic())
  # Wait through the physical break edge so the next action cannot reuse a
  # still-held key; semantic effects are observed without resending input.
  wait(rf'\[sexinput.ps2.scancode\] raw={hex(KEY_CODES[keys[0]]|0x80)}(?=\n|$)',start)
  found=wait(pattern,start)if pattern else None;event.update(serial_end=len(text()),host_end=time.monotonic());return found
 def focus(sid):
  for _ in range(24):
   current=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
   if current and int(current[-1])==sid:return
   key('alt','tab',pattern=r'\[shell.app.switch\] old=\d+ new=\d+ live=1 shortcut=alt_tab')
  raise ValueError('bounded actual AltTab failed to focus '+str(sid))
 def picture(name,kind,anchor=0,mode=None):
  nonlocal park_phase
  launch.pointer_park(q,resolution,text,wait,actions.append,park_phase);park_phase+=1;actions[-1].update(capture=name,serial_end=len(text()));path=out/(name+'.ppm');deadline=time.monotonic()+30;error='no complete frame';frame=dict(name=name,kind=kind,anchor=anchor,requested_anchor=anchor if kind in('reader','copied')else None,mode=mode,screenshot=path.name)
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=text()
   try:check=verify_frame(snapshot,path.read_bytes(),frame,c,resolution,doc);break
   except(ValueError,KeyError,IndexError,OverflowError)as exc:error=str(exc);time.sleep(.05)
  else:
   frame.update(serial_end=len(snapshot),serial_sha256=sha(snapshot.encode()),screenshot_sha256=sha(path.read_bytes()));(out/'failed-frame.json').write_text(json.dumps(dict(frame=frame,error=error),indent=2)+'\n');raise ValueError(name+': '+error)
  frame.update(serial_end=len(snapshot),serial_sha256=sha(snapshot.encode()),screenshot_sha256=sha(path.read_bytes()),result=check);frames.append(frame);(out/'frames.json').write_text(json.dumps(frames,indent=2)+'\n');return check
 def launcher(action):
  start=len(text());key('grave_accent',pattern=r'\[shell.palette.item\] idx=0');key('ret',pattern=rf'\[launcher.action\] target=12 task=12 frame=153 action={action} .*activated=0 queued=0 accepted=1 ok=1 pixels=unconfirmed');wait(r'\[command_palette.close\]',start)
 def close(target,sid):
  focus(sid);start=len(text());evidence['closes'].append(dict(target=target,task=target,serial_start=start));key('f11',pattern=rf'\[sext.app.pool.ready\] target={target} task={target} .*ok=1');exact_cleanup(text(),target,target,start)
 def reopen():
  focus(201);key('ctrl','o',pattern=r'\[shell.document.open\] target=9 linen=200');focus(200)
  for _ in range(40):
   try:
    view,labels=final_view(text())
    if any(label==NAME and identity==view['selected']for identity,_,label in labels):break
   except ValueError:pass
   key('j',pattern=r'\[shell.action.select_next_linen\] id=\d+')
  else:raise ValueError('actual Linen selection never reached connected document')
  if not recovery:picture('linen-selected','linen')
  key('ret',pattern=rf'\[quil.persist.load.ok\] bytes={len(FINAL)} hash={hex(fnv(FINAL))} doc={doc["object"]}(?=\s|$)');focus(201);key('end',pattern=rf'\[quil.cursor.move\] old=\d+ new={len(FINAL)} len={len(FINAL)} dir=end ok=1')
 def wheel(name,delta,before):
  tile,_=http.wheel_viewport(text());x,y=tile['x']+tile['w']-24,tile['y']+64;pointer=len(text());launch.pointer_move(q,x,y,resolution,text,wait,lambda row:actions.append(dict(row,kind='move')));after=http.wheel_move(http.reader_geometry(text(),BODY,c['wheel'],before),delta)[0];row=dict(name=name,wheel=delta,target='body',before=before,after=after,x=x,y=y,pointer_start=pointer,serial_start=len(text()),host_start=time.monotonic(),transport='input-send-event',events=http.wheel_events(delta));evidence['wheel'].append(row);q.cmd('input-send-event',{'events':row['events']});row.update(serial_qmp_return=len(text()),host_qmp_return=time.monotonic());deadline=time.monotonic()+30;error='no actual wheel'
  while time.monotonic()<deadline:
   health();row.update(serial_end=len(text()),host_end=time.monotonic())
   try:http.inspect_wheel_action(text(),row,BODY,c['wheel'],resolution);return after
   except ValueError as exc:error=str(exc);time.sleep(.02)
  raise ValueError(error)
 try:
  peer.start();proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);deadline=time.monotonic()+20
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.02)
  q=QMP(sock)
  if resolution_text=='1024x768':wait(r'SexOS compact 1024x768',timeout=15);q.key('down');q.key('ret')
  wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');wait(r'\[linen.disk.publish.done\]');wait(r'\[spindle.history.receipt\] target=12 task=12 kind=(?:loaded|missing) [^\n]*error=\d+')
  if not recovery:
   focus(201);key('ctrl','n',pattern=r'\[quil.new.ok\] bytes=0');focus(300);key('g',pattern=r'\[browser.asset.authenticated\] ticket=2 ')
   deadline=time.monotonic()+30
   while not peer.request_received.is_set():
    health()
    if time.monotonic()>deadline:raise TimeoutError('actual authenticated peer GET not received')
    time.sleep(.01)
   focus(201);held=dict(serial_start=len(text()),host_start=time.monotonic());evidence['network_wait']=held
   for index,char in enumerate(DRAFT.decode(),1):key(char,pattern=rf'\[quil.document.edit\] revision=\d+ pending=0 bytes={index}(?=\s|$)')
   picture('quil-network-wait','quil');held.update(serial_end=len(text()),host_end=time.monotonic());peer.release_image.set();wait(r'\[browser.asset.decoded\] ticket=2 width=48 height=48 pixels=2304 ');focus(300);picture('image-fit','image',mode='Fit');key('a',pattern=r'\[browser.asset.presentation.intent\] generation=\d+ mode=Actual ');picture('image-actual','image',mode='Actual')
   key('q',pattern=r'\[browser.asset.cleared\] viewport=\d+ sequence=\d+ positive_ack=1');key('h',pattern=rf'\[browser.http.document\] ticket=3 path=/welcome status=200 private_bytes={len(BODY)} positive_release=1');key('v',pattern=r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1');picture('reader','reader');anchor=wheel('down',-1,0);picture('reader-down','reader',anchor);anchor=wheel('up',1,anchor);picture('reader-up','reader',anchor)
   key('ctrl','a',pattern=r'\[shell.browser.input\] code=30 down=1 slot=43 accepted=1');key('i',pattern=r'\[browser.import.layout\] state=NoDocument ');picture('reader-hidden','overlay',anchor)
   declined=key('ctrl','c',pattern=r'\[browser.clipboard.refused\] ticket=(\d+) reason=reader_hidden effects=0');wait(rf'\[browser.clipboard.stopped\] ticket={declined[1]} state=Unsupported payload_erased=1');key('v',pattern=rf'\[browser.import.dismissed\] source=http anchor={anchor} bytes={len(BODY)} restored=1 network=0 selection_preserved=1');picture('reader-returned','selected',anchor)
   key('ctrl','a',pattern=r'\[shell.browser.input\] code=30 down=1 slot=43 accepted=1');key('ctrl','c',pattern=rf'\[browser.clipboard.published\] ticket=\d+ bytes={len(BODY)} generation=\d+ hash={hex(fnv(BODY))} positive_ack=1');picture('browser-copied','copied',anchor)
   focus(201);key('ctrl','a',pattern=r'\[quil.selection\] generation=\d+ revision=\d+ bytes=5 anchor=0 caret=5');key('ctrl','v',pattern=rf'\[quil.clipboard.paste\] ticket=\d+ bytes={len(BODY)} generation=\d+ hash={hex(fnv(BODY))} applied=1 atomic=1');key('z',pattern=rf'\[quil.document.edit\] revision=\d+ pending=0 bytes={len(FINAL)}(?=\s|$)');picture('quil-edited','quil')
   before=disk.read_bytes();(out/'before-save.img.gz').write_bytes(gzip.compress(before,mtime=0));key('ctrl','shift','s',pattern=r'\[quil.name.mode\] on=1 save_as=1')
   for char in c['name_keys']:key(char)
   key('ret',pattern=rf'\[quil.persist.save.ok\] bytes={len(FINAL)} hash={hex(fnv(FINAL))} doc=\d+');wait(r'\[quil.document.saved\] saved_revision=(\d+) revision=\1 dirty=0');doc=storage.import_disk(before,disk.read_bytes(),NAME,FINAL);picture('quil-saved','quil');reopen();picture('quil-reopened','quil')
   launcher('Open')
   for index,char in enumerate(COMMAND,1):key('spc'if char==' 'else char,pattern=rf'\[spindle.cursor.view\] target=12 sequence=\d+ pos={index} len={index}(?=\s|$)')
   key('ret',pattern=r'\[spindle.instance.command\] target=12 task=12 name=echo ok=1');wait(r'\[spindle.history.receipt\] target=12 task=12 kind=saved [^\n]*error=0');key('up',pattern=rf'\[spindle.history.nav\] dir=up idx=0 len={len(COMMAND)} ok=1');picture('spindle-recalled','spindle')
   start=len(text());launch.pointer_click(q,c['launcher']['workspace_x'][1]+10,c['launcher']['workspace_y']+10,resolution,text,wait,actions.append);wait(r'\[shell.scene.switch\] from=0 to=1',start);launcher('BringHere');picture('spindle-brought','spindle')
  else:
   reopen();picture('quil-cold','quil');launcher('Open');key('up',pattern=rf'\[spindle.history.nav\] dir=up idx=0 len={len(COMMAND)} ok=1');picture('spindle-cold','spindle')
  close(12,153)
  if not recovery:
   start=len(text());launch.pointer_click(q,c['launcher']['workspace_x'][0]+10,c['launcher']['workspace_y']+10,resolution,text,wait,actions.append);wait(r'\[shell.scene.switch\] from=1 to=0',start)
  close(15,300);close(9,201)
 except Exception as exc:failure=clip.failure_summary(exc)
 finally:
  if proc is not None and proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  peer.close();sock.unlink(missing_ok=True);terminal=disk.read_bytes();(out/'nvme.img.gz').write_bytes(gzip.compress(terminal,mtime=0));(out/'peer-exchanges.json').write_text(json.dumps(dict(plain=peer.plain,tls=peer.rows,dns=peer.dns,attempts=peer.attempts,errors=peer.errors),indent=2)+'\n');(out/'frames.json').write_text(json.dumps(frames,indent=2)+'\n');(out/'actions.json').write_text(json.dumps(actions,indent=2)+'\n');(out/'evidence.json').write_text(json.dumps(evidence,indent=2)+'\n');m.update(host_utc_end=time.time(),final_disk_sha256=sha(terminal),error=failure)
  names=['serial.log','wire.pcap','peer-exchanges.json','actions.json','frames.json','evidence.json','seed.img.gz','nvme.img.gz']+[f['screenshot']for f in frames]+([]if recovery else['before-save.img.gz']);m['raw_sha256']={name:sha((out/name).read_bytes())for name in names if(out/name).is_file()};(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
 if failure or peer.errors:
  result=dict(ok=False,error=failure,peer_errors=peer.errors,full_alpha_acceptance=False,scope=SCOPE);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');raise RuntimeError(str(result))
 result=replay_boot(root,iso,binding,out,c);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def replay(root,iso,binding,out):
 c=source_contract(root);out=Path(out);live=replay_boot(root,iso,binding,out/'session',c);cold=replay_boot(root,iso,binding,out/'recovery',c);a=json.loads((out/'session/manifest.json').read_text());b=json.loads((out/'recovery/manifest.json').read_text())
 if a['binding']!=b['binding']or a['final_disk_sha256']!=b['input_disk_sha256']or live['joins']['document']!=cold['joins']['document']or live['joins']['history']!=cold['joins']['history']:raise ValueError('same-image terminal-disk/document/history cold chain differs')
 return dict(ok=True,acceptance='representative_connected_only',full_alpha_acceptance=False,boots=2,frames=live['frames']+cold['frames'],same_image=True,same_disk_state=True,session=live,recovery=cold,scope=SCOPE)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');p.add_argument('--check',action='store_true');a=p.parse_args();root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);binding=a.binding.resolve(strict=True);out=a.out.resolve()
 if a.check:print(json.dumps(replay(root,iso,binding,out)));return
 if a.source_disk is None:p.error('native connected session requires --source-disk')
 out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh connected cohort directory required')
 c=source_contract(root);live=run_boot(root,iso,binding,a.source_disk.resolve(strict=True),out/'session',c,a.model,a.resolution);run_boot(root,iso,binding,out/'session/nvme.img',out/'recovery',c,a.model,a.resolution,True,live['joins']['document']);result=replay(root,iso,binding,out);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()
