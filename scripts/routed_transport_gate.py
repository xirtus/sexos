#!/usr/bin/env python3
"""Bounded real-QEMU gateway fixture. Only peer packets are produced here.

The off-subnet endpoint is explicit test data, not an app endpoint grant. No TAP,
host routes, root, Internet, DNS/HTTP, hostile-DMA or physical-hardware proof.
"""
import argparse,hashlib,json,re,select,shutil,socket,struct,subprocess,time
from pathlib import Path
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_tcp_capture import checksum,fingerprint,inspect as tcp_check

LOCAL=bytes([10,0,2,15]);GATEWAY=bytes([10,0,2,2]);REMOTE=bytes([198,51,100,9])
GUEST_MAC=bytes.fromhex('525400123456');REMOTE_MAC=bytes.fromhex('020000000009')
GATEWAY_MAC=bytes.fromhex('020000000002');RENEWED_MAC=bytes.fromhex('020000000003')
PORT=46076;SILENT_PORT=61000;MAX_FRAME=1514;MAX_PACKETS=4096;MAX_CAPTURE=4*1024*1024
MAX_STARTUP_SYN=8
# Pinned smoltcp0.14.0 iface/neighbor.rs: ENTRY_LIFETIME=60000ms,
# SILENT_TIME=1000ms. Host delay is a trigger, never proof of guest tick expiry.
HOST_BOUND=240;EXPIRY_BOUND=180
ROUTE_MARKER=r'\[sexnet.route.ready\] gateway=10\.0\.2\.2 default=1 routes=1 route_capacity=2 neighbors=8 discovery_ms=1000 expiry_ms=60000 heap=0'
FAULT=r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|to=Quarantined|sexnet.transport.failed|serial\.record\.overflow|\[sexfiles.defer.drop\]'

def digest(data):return hashlib.sha256(data).hexdigest()
def arp_query(mac=GUEST_MAC):
 return b'\xff'*6+mac+b'\x08\x06'+struct.pack('!HHBBH',1,0x800,6,4,1)+mac+LOCAL+b'\0'*6+GATEWAY+b'\0'*18
def arp_reply(mac,guest=GUEST_MAC):
 return guest+mac+b'\x08\x06'+struct.pack('!HHBBH',1,0x800,6,4,2)+mac+GATEWAY+guest+LOCAL+b'\0'*18
def decode_arp(frame):
 if len(frame)<14 or frame[12:14]!=b'\x08\x06':return None
 if len(frame)<42 or frame[14:20]!=struct.pack('!HHBB',1,0x800,6,4):raise ValueError('malformed Ethernet/IPv4 ARP')
 opcode=int.from_bytes(frame[20:22],'big')
 if opcode not in(1,2)or frame[6:12]!=frame[22:28]:raise ValueError('invalid ARP operation/source binding')
 return dict(opcode=opcode,sender=frame[28:32],target=frame[38:42],mac=frame[22:28],target_mac=frame[32:38])
def peer_tcp(port,seq,ack,flags,body=b''):
 if not 1<=port<=65535 or len(body)>256:raise ValueError('bounded peer TCP turn required')
 tcp=bytearray(struct.pack('!HHIIBBHHH',port,PORT,seq&0xffffffff,ack&0xffffffff,5<<4,flags,4096,0,0)+body)
 tcp[16:18]=checksum(REMOTE+LOCAL+b'\0\x06'+len(tcp).to_bytes(2,'big')+tcp).to_bytes(2,'big')
 ip=bytearray(b'\x45\0'+(20+len(tcp)).to_bytes(2,'big')+b'\0\0\x40\0\x40\x06\0\0'+REMOTE+LOCAL)
 ip[10:12]=checksum(ip).to_bytes(2,'big');frame=GUEST_MAC+REMOTE_MAC+b'\x08\0'+ip+tcp
 return bytes(frame).ljust(60,b'\0')
def decode_tcp(frame):
 if len(frame)<14 or frame[12:14]!=b'\x08\0':return None
 ip=frame[14:]
 if len(ip)<20:raise ValueError('short IPv4')
 ih=(ip[0]&15)*4;length=int.from_bytes(ip[2:4],'big')
 if ip[0]>>4!=4 or ih<20 or length<ih or length>len(ip)or checksum(ip[:ih]):raise ValueError('invalid IPv4 header/checksum')
 if int.from_bytes(ip[6:8],'big')&0x3fff:raise ValueError('fragmented IPv4 fixture/capture')
 if ip[9]!=6:raise ValueError('unexpected non-TCP IPv4 output')
 tcp=ip[ih:length]
 if len(tcp)<20 or checksum(ip[12:20]+b'\0\x06'+len(tcp).to_bytes(2,'big')+tcp):raise ValueError('invalid TCP length/checksum')
 th=(tcp[12]>>4)*4
 if th<20 or th>len(tcp):raise ValueError('invalid TCP header length')
 sport,dport,seq,ack=struct.unpack_from('!HHII',tcp)
 return dict(source=ip[12:16],destination=ip[16:20],sport=sport,dport=dport,seq=seq,ack=ack,flags=tcp[13],body=tcp[th:])
def startup_syn(segment):
 """Only the normal Browser's first fixed on-link Echo SYN is background.

 No arbitrary flow, response, payload or reset is exempted. The probe has no
 fixture server here; its existing owned deadline ends it. Keep every frame.
 """
 return (segment['source'],segment['destination'],segment['sport'],segment['dport'],segment['flags'],segment['ack'],segment['body'])==(LOCAL,GATEWAY,48001,46078,2,0,b'')
def pcap(captures,direction=None,exclude_port=None):
 out=bytearray(struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,MAX_FRAME,1))
 for item in captures:
  if direction is not None and item['direction']!=direction:continue
  data=bytes.fromhex(item['hex']);segment=decode_tcp(data)
  if segment and exclude_port in(segment['sport'],segment['dport']):continue
  seconds=int(item['host_time']);micros=int((item['host_time']-seconds)*1e6)
  out.extend(struct.pack('<IIII',seconds,micros,len(data),len(data)));out.extend(data)
 return bytes(out)
def inspect_routing(captures,mode):
 """Pure oracle over captured bytes; it never substitutes for a real guest."""
 errors=[];gateway=None;pending_query=False;queries=0;replies=0;tcp_packets=0;silent_tcp=0;startup=0;last_query=-1;renewal_query=-1;renewal_reply=-1;first_reply=-1
 try:
  if mode not in('normal','recover','silence'):raise ValueError('invalid mode')
  if len(captures)>MAX_PACKETS or sum(len(bytes.fromhex(x['hex']))for x in captures)>MAX_CAPTURE:raise ValueError('capture bound exceeded')
  for index,item in enumerate(captures):
   frame=bytes.fromhex(item['hex'])
   if not 14<=len(frame)<=MAX_FRAME or digest(frame)!=item['sha256']:raise ValueError('capture length/hash mismatch')
   direction=item['direction'];arp=decode_arp(frame)
   if direction not in('guest_tx','peer_fixture'):raise ValueError('unknown frame custody')
   if arp:
    if direction=='guest_tx':
     if arp['opcode']!=1 or arp['sender']!=LOCAL or arp['target']!=GATEWAY or arp['mac']!=GUEST_MAC or frame[:6]!=b'\xff'*6:raise ValueError('guest ARP must discover configured gateway, never remote IP')
     queries+=1;last_query=index;pending_query=True
     if gateway is not None:renewal_query=index
    else:
     if arp['opcode']!=2 or arp['sender']!=GATEWAY or arp['target']!=LOCAL or arp['mac']not in(GATEWAY_MAC,RENEWED_MAC)or frame[:6]!=GUEST_MAC or arp['target_mac']!=GUEST_MAC or last_query<0:raise ValueError('unbound gateway ARP fixture')
     if arp['mac']==RENEWED_MAC:
      if gateway!=GATEWAY_MAC or renewal_query<=first_reply:raise ValueError('renewal lacks fresh gateway discovery')
      renewal_reply=index
     elif gateway==RENEWED_MAC:raise ValueError('gateway MAC reverted after renewal')
     gateway=arp['mac'];replies+=1;pending_query=False
     if first_reply<0:first_reply=index
    continue
   segment=decode_tcp(frame)
   if segment is None:raise ValueError('unexpected Ethernet protocol after driver startup')
   if direction=='guest_tx':
    if startup_syn(segment):
     if gateway is None or frame[:6]!=gateway or frame[6:12]!=GUEST_MAC:raise ValueError('startup Echo SYN bypassed resolved on-link peer')
     startup+=1
     if startup>MAX_STARTUP_SYN:raise ValueError('startup Echo retransmission bound exceeded')
     continue
    if segment['source']!=LOCAL or segment['destination']!=REMOTE or segment['sport']!=PORT:raise ValueError('guest IPv4 endpoint differs from routed remote tuple')
    if gateway is None or pending_query or frame[:6]!=gateway or frame[6:12]!=GUEST_MAC or frame[:6]==REMOTE_MAC:raise ValueError('guest off-subnet TCP bypassed resolved gateway MAC')
    if segment['dport']==SILENT_PORT:silent_tcp+=1
    tcp_packets+=1
   elif segment['source']!=REMOTE or segment['destination']!=LOCAL or segment['dport']!=PORT or frame[:6]!=GUEST_MAC or frame[6:12]!=REMOTE_MAC:raise ValueError('explicit off-subnet peer fixture tuple differs')
  if not queries or not replies or not tcp_packets:raise ValueError('missing gateway discovery or actual routed TCP')
  if mode=='recover'and(renewal_query<0 or renewal_reply<renewal_query or gateway!=RENEWED_MAC):raise ValueError('missing fresh ARP/new gateway MAC recovery')
  if mode=='silence'and silent_tcp:raise ValueError('unresolved/expired silent SYN emitted guest TCP')
 except(ValueError,KeyError,TypeError,struct.error)as error:errors.append(str(error))
 return dict(ok=not errors,errors=errors,gateway_queries=queries,gateway_replies=replies,routed_tcp_packets=tcp_packets,silent_tcp_packets=silent_tcp,startup_echo_syn=startup,renewal_query_capture=renewal_query,renewal_reply_capture=renewal_reply,scope='Actual byte oracle requires remote IPv4 destination with configured on-link gateway Ethernet destination. Only the exact bounded normal Browser startup Echo SYN tuple is accounted separately; all bytes remain captured. This diagnostic listener fixture grants no app endpoint authority.')

class Peer:
 def __init__(self,connection,health):
  self.connection=connection;self.health=health;self.buffer=bytearray();self.captures=[];self.total=0;self.started=False;self.gateway=None;self.answer_mac=None;self.queries=0;self.new_query=False;self.startup=0
 def capture(self,direction,data):
  self.total+=len(data)
  if len(self.captures)>=MAX_PACKETS or self.total>MAX_CAPTURE:raise RuntimeError('packet/memory qualification bound exceeded')
  self.captures.append(dict(direction=direction,host_time=time.time(),monotonic_time=time.monotonic(),hex=data.hex(),sha256=digest(data)))
 def send(self,data):
  if not 14<=len(data)<=MAX_FRAME:raise ValueError('bounded Ethernet fixture required')
  self.connection.sendall(len(data).to_bytes(4,'big')+data);self.capture('peer_fixture',data)
 def receive(self,timeout=.1):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   self.health()
   if len(self.buffer)>=4:
    length=int.from_bytes(self.buffer[:4],'big')
    if not 14<=length<=MAX_FRAME:raise RuntimeError('invalid QEMU backend Ethernet length')
    if len(self.buffer)>=4+length:
     data=bytes(self.buffer[4:4+length]);del self.buffer[:4+length];self.capture('guest_tx',data);return data
   readable,_,_=select.select([self.connection],[],[],min(.05,max(0,deadline-time.monotonic())))
   if readable:
    data=self.connection.recv(4096)
    if not data:raise RuntimeError('QEMU Ethernet backend closed')
    self.buffer.extend(data)
    if len(self.buffer)>8192:raise RuntimeError('QEMU stream buffering bound exceeded')
  return None
 def process(self,frame):
  arp=decode_arp(frame)
  if arp:
   if arp['opcode']!=1 or arp['sender']!=LOCAL or arp['target']!=GATEWAY or arp['mac']!=GUEST_MAC:raise RuntimeError('guest ARP targeted non-gateway or invalid holder')
   self.queries+=1;self.new_query=True
   if self.answer_mac is not None:self.send(arp_reply(self.answer_mac));self.gateway=self.answer_mac
   return None
  segment=decode_tcp(frame)
  if segment is None:raise RuntimeError('unexpected guest Ethernet output')
  if startup_syn(segment):
   if self.gateway is None or frame[:6]!=self.gateway or frame[6:12]!=GUEST_MAC:raise RuntimeError('startup Echo SYN before neighbor resolution')
   self.startup+=1
   if self.startup>MAX_STARTUP_SYN:raise RuntimeError('startup Echo retransmission bound exceeded')
   return None
  if self.gateway is None or segment['source']!=LOCAL or segment['destination']!=REMOTE or segment['sport']!=PORT or frame[:6]!=self.gateway or frame[6:12]!=GUEST_MAC:raise RuntimeError('guest off-link IP/MAC route mismatch or IP before gateway resolution')
  return segment
 def pump(self,seconds):
  deadline=time.monotonic()+seconds
  while time.monotonic()<deadline:
   frame=self.receive(min(.1,deadline-time.monotonic()))
   if frame is not None:self.process(frame)
 def flow(self,port,payload,aborted=False,fresh=False,mac=GATEWAY_MAC):
  started=time.monotonic();client=0xfffffff0 if port==62001 else 100000+port;isn=client;self.answer_mac=mac;self.new_query=False
  if fresh:self.gateway=None
  self.send(peer_tcp(port,client,0,2));client=(client+1)&0xffffffff;server=None;stream={};fin=False;fin_ack=False
  def receive(deadline):
   nonlocal server,fin,fin_ack
   while time.monotonic()<deadline:
    frame=self.receive(.1)
    if frame is None:continue
    segment=self.process(frame)
    if segment is None:continue
    if fresh and not self.new_query:raise RuntimeError('gateway expiry not observed: TCP preceded fresh ARP')
    if segment['dport']!=port:continue
    if segment['flags']&4:raise RuntimeError('unexpected guest RST')
    if segment['flags']&2:
     if not segment['flags']&0x10 or segment['ack']!=(isn+1)&0xffffffff:raise RuntimeError('bad actual SYN/ACK correlation')
     value=(segment['seq']+1)&0xffffffff
     if server not in(None,value):raise RuntimeError('server ISN changed')
     server=value;return segment
    if server is None:raise RuntimeError('guest TCP preceded SYN/ACK')
    if segment['flags']&0x10:
     acknowledged=(segment['ack']-isn-1)&0xffffffff;admitted=(client-isn-1)&0xffffffff
     if acknowledged>admitted:raise RuntimeError('guest ACK acknowledged unsent bytes')
     fin_ack=fin_ack or acknowledged==len(payload)+1
    offset=(segment['seq']-server)&0xffffffff
    if segment['body']:
     if offset+len(segment['body'])>len(payload):raise RuntimeError('extra/out-of-range echoed bytes')
     for n,byte in enumerate(segment['body'],offset):
      if byte!=payload[n]or n in stream and stream[n]!=byte:raise RuntimeError('actual echoed byte/retransmission differs')
      stream[n]=byte
    if segment['flags']&1:
     if offset+len(segment['body'])!=len(payload):raise RuntimeError('early/mispositioned guest FIN')
     fin=True
    contiguous=0
    while contiguous in stream:contiguous+=1
    if segment['body']or segment['flags']&1:self.send(peer_tcp(port,client,(server+contiguous+int(fin))&0xffffffff,0x10))
    return segment
   raise TimeoutError('bounded actual TCP exchange')
  deadline=started+25
  receive(deadline)
  self.send(peer_tcp(port,client,server,0x10))
  sent=0;wanted=payload[:256]if aborted else payload
  while sent<len(wanted):
   part=wanted[sent:sent+192];self.send(peer_tcp(port,client,(server+len(stream))&0xffffffff,0x18,part));client=(client+len(part))&0xffffffff;sent+=len(part)
   while len(stream)<sent:receive(deadline)
  if aborted:self.send(peer_tcp(port,client,(server+len(stream))&0xffffffff,0x14))
  else:
   self.send(peer_tcp(port,client,(server+len(stream))&0xffffffff,0x11));client=(client+1)&0xffffffff
   while not(fin and fin_ack):receive(deadline)
  response=bytes(stream[n]for n in range(len(stream)))
  return dict(peer_port=port,length=len(payload),aborted=aborted,payload_hex=payload.hex(),response_hex=response.hex(),payload_sha256=digest(payload),response_sha256=digest(response),gateway_mac=mac.hex(),fresh_gateway_query=self.new_query,seconds=time.monotonic()-started)

def custody(text,captures):
 errors=[]
 for direction,labels in [('guest_tx',['tx.submit','tx.done']),('peer_fixture',['rx'])]:
  frames=[bytes.fromhex(x['hex'])for x in captures if x['direction']==direction]
  wanted=[(n,(n-1)%8,len(frame),fingerprint(frame))for n,frame in enumerate(frames,1)if n<=64 or n&(n-1)==0]
  for label in labels:
   actual=[(int(m[1]),int(m[2]),int(m[3]),int(m[4],16))for m in re.finditer(r'\[sexnet\.ether\.'+re.escape(label)+r'\] sequence=(\d+) index=(\d+) length=(\d+) hash=(0x[0-9a-f]+)',text)]
   if actual!=wanted:errors.append('sampled actual driver '+label+' differs from captured full frame/sequence/index/hash')
 if '[sexnet.ether.rx.drop]'in text or '[sexnet.ether.send.failed]'in text:errors.append('normal bounded fixture lost hardware custody')
 return dict(ok=not errors,errors=errors,scope='Sampled descriptor admissions/completions and ingress join actual captured frame hashes; completion is device custody, not endpoint durability.')

def expiry_receipt(text,captures):
 """A post-RX raw tick upper-bounds fill time; displayed time is irrelevant."""
 sequence=0;gateway=None
 for item in captures:
  if item['direction']!='peer_fixture':continue
  sequence+=1;frame=bytes.fromhex(item['hex']);arp=decode_arp(frame)
  if arp and arp['opcode']==2 and arp['sender']==GATEWAY and arp['mac']==GATEWAY_MAC:
   gateway=(sequence,frame);break
 if gateway is None:return None
 sequence,frame=gateway
 marker=f'[sexnet.ether.rx] sequence={sequence} index={(sequence-1)%8} length={len(frame)} hash={fingerprint(frame):#x}'
 at=text.find(marker)
 if at<0:return None
 waits=list(re.finditer(r'\[sexnet.receive.wait\] before=(\d+) after=(\d+) ticks=2 count=\d+ kind=timer_assisted_poll',text[at+len(marker):]))
 if not waits:return None
 baseline=int(waits[0][2]);target=baseline+60001
 if any(int(m[2])<int(m[1]) for m in waits):raise ValueError('raw guest tick receipt regressed')
 observed=next((m for m in waits if int(m[2])>=target),None)
 if observed is None:return None
 return dict(gateway_rx_sequence=sequence,gateway_rx_marker=marker,fill_upper_tick=baseline,target_tick=target,observed_tick=int(observed[2]),raw_tick_marker=observed[0],proof='Exact gateway RX precedes raw own-task wait sample; next transport advance processes ARP before that wait. A later raw tick exceeds the fill upper bound plus60000. Fresh ARP and a new gateway MAC remain separately mandatory.')

def bound_source(iso,path):
 binding=json.loads(path.read_text());root=Path(__file__).resolve().parents[1]
 if digest(iso.read_bytes())!=binding['iso_sha256']:raise RuntimeError('ISO differs from frozen binding')
 for collection in ['sources','runner_sha256']:
  for name,value in binding[collection].items():
   if digest((root/name).read_bytes())!=value:raise RuntimeError('frozen source/runner drift: '+name)
 if 'scripts/routed_transport_gate.py'not in binding['runner_sha256']:raise RuntimeError('new routed runner absent from frozen binding')
 return binding
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--iso',required=True,type=Path);parser.add_argument('--source-disk',required=True,type=Path);parser.add_argument('--out',required=True,type=Path);parser.add_argument('--model',required=True,choices=['e1000','e1000e']);parser.add_argument('--mode',choices=['normal','recover','silence'],default='normal');parser.add_argument('--source-binding',type=Path);args=parser.parse_args()
 iso=args.iso.resolve(strict=True);seed=args.source_disk.resolve(strict=True);binding_path=(args.source_binding or iso.parent/'source-binding.json').resolve(strict=True);binding=bound_source(iso,binding_path)
 out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh output required')
 shutil.copyfile(binding_path,out/'source-binding.json');disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log'
 listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(1);listener.settimeout(30);port=listener.getsockname()[1]
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev',f'socket,id=peer,connect=127.0.0.1:{port}','-device',f'{args.model},netdev=peer,mac=52:54:00:12:34:56']
 manifest=dict(iso_sha256=binding['iso_sha256'],source_binding_sha256=digest(binding_path.read_bytes()),input_disk_sha256=digest(seed.read_bytes()),command=cmd,model=args.model,mode=args.mode,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),limits=dict(packets=MAX_PACKETS,capture_bytes=MAX_CAPTURE,host_seconds=HOST_BOUND,neighbor_expiry_guest_ticks=60000,expiry_wait_host_bound=EXPIRY_BOUND),fixture_scope='Explicit raw off-subnet IPv4/TCP peer and on-link ARP gateway fixtures. All guest packets come from the selected real QEMU NIC. No host route/root/TAP or app endpoint grant.')
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);peer=None;connection=None;rows=[];early=[];evidence={};failure=None;started=time.monotonic()
 def text():
  if serial.exists()and serial.stat().st_size>16*1024*1024:raise RuntimeError('serial evidence bound exceeded')
  return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if time.monotonic()-started>HOST_BOUND:raise TimeoutError('whole native qualification bound')
  if re.search(FAULT,text()):raise RuntimeError('guest/device/transport failure')
 def wait(pattern,timeout=15,pump=True):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text())
   if match:return match
   if pump and peer:peer.pump(.05)
   else:time.sleep(.02)
  raise TimeoutError(pattern)
 def finish_flow(port,length,aborted,number,mac,fresh=False):
  payload=bytes((n*17+number*23)&255 for n in range(length));rows.append(peer.flow(port,payload,aborted,fresh,mac))
  wait(r'\[sexnet.transport.state\] from=\w+ to=Listen flow='+str(number)+r' ')
 try:
  connection,_=listener.accept();connection.settimeout(2);peer=Peer(connection,health)
  wait(ROUTE_MARKER,pump=False);wait(r'\[sexnet.transport.ready\]',pump=False)
  # Begin driver custody at its exact real startup ARP, retaining earlier frames
  # separately so firmware traffic cannot substitute for runtime evidence.
  for _ in range(64):
   frame=peer.receive(1)
   if frame==arp_query():peer.started=True;break
   if frame is not None:early.append(peer.captures.pop())
  else:raise RuntimeError('missing actual startup gateway ARP')
  (out/'pre-driver-capture.json').write_text(json.dumps(early,indent=2)+'\n')
  if args.mode=='silence':
   silent_start=len(peer.captures);peer.send(peer_tcp(SILENT_PORT,70000,0,2));wait(r'\[sexnet.transport.state\] from=Listen to=SynReceived flow=0 ')
   wait(r'\[sexnet.transport.state\] from=\w+ to=Listen flow=0 ',timeout=16)
   evidence['silence']=dict(capture_start=silent_start,capture_end=len(peer.captures),gateway_unanswered=True,listener_timeout_observed=True,queries=peer.queries)
   if not peer.queries:raise RuntimeError('no actual discovery attempt while silent')
  finish_flow(62001,8193,False,1,GATEWAY_MAC,True)
  if args.mode=='recover':
   peer.answer_mac=None;idle_start=time.monotonic()
   while (receipt:=expiry_receipt(text(),peer.captures))is None:
    if time.monotonic()-idle_start>EXPIRY_BOUND:raise TimeoutError('raw guest tick expiry progress not observed')
    peer.pump(.1)
   evidence['expiry']={**receipt,'host_idle_seconds':time.monotonic()-idle_start}
   finish_flow(62002,4097,False,2,RENEWED_MAC,True);mac=RENEWED_MAC;number=3
  else:mac=GATEWAY_MAC;number=2
  finish_flow(62003,1025,True,number,mac);finish_flow(62004,4097,False,number+1,mac)
  wait(r'\[app.surface.presented\] target=15 surface=300');peer.pump(.25)
  deadline=time.monotonic()+2
  while not custody(text(),peer.captures)['ok']and time.monotonic()<deadline:peer.pump(.05)
 except Exception as error:failure=f'{type(error).__name__}: {error}'
 finally:
  if connection:connection.close()
  listener.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  captures=peer.captures if peer else[]
  (out/'frame-capture.json').write_text(json.dumps(dict(pre_driver=early,captures=captures,observations=evidence),indent=2)+'\n');(out/'endpoint-exchanges.json').write_text(json.dumps(rows,indent=2)+'\n')
  for direction,name in [(None,'wire'),('guest_tx','guest_tx'),('peer_fixture','peer_fixture')]:
   # Retain even malformed frames on failure; PCAP export itself must not hide
   # the original exception by trying to parse corrupted packets again.
   raw=bytearray(struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,MAX_FRAME,1))
   for item in early+captures:
    if direction is not None and item['direction']!=direction:continue
    data=bytes.fromhex(item['hex']);seconds=int(item['host_time']);micros=int((item['host_time']-seconds)*1e6);raw.extend(struct.pack('<IIII',seconds,micros,len(data),len(data)));raw.extend(data)
   (out/(name+'.pcap')).write_bytes(raw)
 result=dict(ok=False,error=failure,observations=evidence,completed_flows=len(rows),scope='Normal immutable BOOT config with actual diagnostic listener off-subnet TCP through its on-link gateway. Full captures/sequence checks plus NIC/ownership receipts; no off-link app DNS/HTTP policy expansion, public routing, TLS or physical/SMP/hostile containment proof.')
 if failure is None:
  result['routing']=inspect_routing(captures,args.mode);result['capture']=tcp_check(pcap(captures,exclude_port=SILENT_PORT),rows);result['authority']=nic_check(text(),expected=1,preparation=True,ethernet=True,network_config=True,routing=True);result['self_wait']=wait_check(text());result['custody']=custody(text(),captures)
  result['ok']=all(result[key]['ok']for key in ['routing','capture','authority','self_wait','custody'])
  if args.mode=='silence':result['ok']=result['ok']and evidence.get('silence',{}).get('listener_timeout_observed',False)
  try:bound_source(iso,binding_path)
  except Exception as error:result['ok']=False;result['error']='post-run frozen binding: '+str(error)
 result['seconds']=time.monotonic()-started;(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(result,indent=2))

if __name__=='__main__':main()
