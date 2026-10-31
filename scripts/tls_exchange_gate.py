#!/usr/bin/env python3
"""Actual controlled TLS peer, guest wire and exact held-authority exchange.

The host is the named endpoint itself. It neither forwards guest traffic nor
provides a substitute verifier. No TLS key log or ephemeral secrets are saved.
"""
from clock_source_evidence import browser_clock_sources
import argparse,ast,hashlib,json,math,os,re,shutil,socket,ssl,struct,subprocess,tempfile,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from entropy_seed_gate import ECHO
from certificate_time_gate import RTC_NORMAL
from tls_prepare_gate import command_profile as preparation_profile
from check_nonunix_target import inspect as target_check
from check_tcp_capture import frames,checksum,fingerprint
from check_dns_capture import query as dns_query,response as dns_response,inspect as dns_capture
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check

CERTIFICATE_MODES={'wrong-name':'WRONG_NAME_DER','expired':'EXPIRED_DER','future':'FUTURE_DER','client-only':'CLIENT_ONLY_DER','foreign-root':'FOREIGN_DER','corrupt-signature':'LEAF_DER'}
TRANSPORT_MODES=('truncated-handshake','stall')
REJECTION_MODES=tuple(CERTIFICATE_MODES)+('unsupported-version',)+TRANSPORT_MODES
MODES=('ready','cancel','close')+REJECTION_MODES
HOST='probe.sexos.test';LOCAL=bytes((10,0,2,15));PEER=bytes((10,0,2,2))
REQUEST=b'SexOS guest encrypted request round93\n'
RESPONSE=b'SexOS controlled encrypted reply round93\n'
CIPHER='TLS_AES_128_GCM_SHA256'
LABELS={'Pending':b'checking','Verified':b'peer verified','Unavailable':b'unavailable','Cancelled':b'cancelled','Uncertain':b'waiting receipt'}
SCOPE='Controlled TLS1.3/P256/AES128 peer exchange through exact BOOT policy/seed/time, positively released fixed-name DNS and admitted duplex lease. Independent hostSSL receives the exact public request, sends the exact public response and unwrap verifies guest close_notify; PCAP independently checks actual TCP/TLS records and absence of those plaintext messages on46080. No host proxy, publicInternet/arbitrary endpoint, publicCA, physical/SMP/hostile-containment or general web-app claim.'

def rejection_reason(mode):
 return 'Some(Certificate)'if mode in CERTIFICATE_MODES else'Some(Handshake)'if mode=='unsupported-version'else'None'

def fixture_der(root,mode):
 symbol=CERTIFICATE_MODES.get(mode,'LEAF_DER');source=(Path(root)/'crates/sex-tls/src/probe_fixture.rs').read_text();match=re.search(r'pub const '+symbol+r'[^=]*=\s*&\[(.*?)\];',source,re.S)
 if not match:raise ValueError('missing source-bound public DER fixture '+symbol)
 data=bytes(int(x,16)for x in re.findall(r'0x([0-9a-fA-F]{2})',match[1]))
 if mode=='corrupt-signature':data=data[:-1]+bytes([data[-1]^1])
 return data

def fixture_descriptor(root,mode):
 data=fixture_der(root,mode)
 return dict(symbol=CERTIFICATE_MODES.get(mode,'LEAF_DER'),der_sha256=hashlib.sha256(data).hexdigest(),signature_last_byte_flipped=mode=='corrupt-signature',public_test_key=True)

def scope(mode):
 if mode not in REJECTION_MODES:return SCOPE
 evidence='Actual server certificate flight and guest Certificate failure'if mode in CERTIFICATE_MODES else'Actual unsupported-version fatal alert and guest Handshake failure'if mode=='unsupported-version'else'Bounded transport/timeout cleanup, without parser-EOF or certificate-rejection claim'
 return evidence+'; exact BOOT/held-route policy, DNS, fresh seed/UTC and positive RELEASE before Unavailable. No guest authenticated peer, application request, publicInternet, publicCA or general browser claim.'

def command_profile(command):
 return preparation_profile(command,'ready')

def tls_records(data):
 records=[];offset=0
 while offset<len(data):
  if len(data)-offset<5:raise ValueError('truncated TLS record header')
  kind,version,length=struct.unpack_from('!BHH',data,offset)
  if kind not in(20,21,22,23)or version not in(0x0301,0x0303)or not 0<length<=16640:raise ValueError('invalid TLS record kind/version/extent')
  end=offset+5+length
  if end>len(data):raise ValueError('truncated TLS record payload')
  payload=data[offset+5:end]
  if kind==20 and payload!=b'\x01':raise ValueError('noncanonical compatibility CCS')
  if kind==23 and len(payload)<17:raise ValueError('encrypted TLS record lacks tag/inner-content extent')
  records.append(dict(type=kind,version=version,length=length,payload=payload));offset=end
 return records

def hello(record,client):
 data=record['payload'];wanted=1 if client else 2
 if record['type']!=22 or len(data)<4 or data[0]!=wanted or int.from_bytes(data[1:4],'big')!=len(data)-4:raise ValueError('missing exact complete ClientHello/ServerHello')
 body=data[4:];offset=34
 if len(body)<35 or body[:2]!=b'\x03\x03':raise ValueError('wrong Hello legacy version/extent')
 session=body[offset];offset+=1
 if session>32 or offset+session>len(body):raise ValueError('invalid Hello session identifier')
 offset+=session
 if client:
  if offset+2>len(body):raise ValueError('missing offered cipher extent')
  size=int.from_bytes(body[offset:offset+2],'big');offset+=2
  if size<2 or size%2 or offset+size>len(body):raise ValueError('invalid offered cipher extent')
  suites=[int.from_bytes(body[n:n+2],'big')for n in range(offset,offset+size,2)];offset+=size
  if suites!=[0x1301]:raise ValueError('guest offered unexpected TLS cipher')
  if offset>=len(body):raise ValueError('missing compression extent')
  size=body[offset];offset+=1
  if size!=1 or body[offset:offset+size]!=b'\0':raise ValueError('unexpected Hello compression')
  offset+=size
 else:
  if body[offset:offset+3]!=b'\x13\x01\0':raise ValueError('server selected unexpected cipher/compression')
  offset+=3
 if offset+2>len(body):raise ValueError('missing Hello extension extent')
 size=int.from_bytes(body[offset:offset+2],'big');offset+=2
 if offset+size!=len(body):raise ValueError('Hello extension extent mismatch')
 extensions={}
 while offset<len(body):
  if len(body)-offset<4:raise ValueError('truncated Hello extension')
  kind,size=struct.unpack_from('!HH',body,offset);offset+=4
  if kind in extensions or offset+size>len(body):raise ValueError('duplicate/truncated Hello extension')
  extensions[kind]=body[offset:offset+size];offset+=size
 if client:
  if extensions.get(43)!=b'\x02\x03\x04':raise ValueError('guest did not offer exact TLS1.3')
  sni=extensions.get(0,b'');name=HOST.encode()
  if sni!=struct.pack('!HBH',len(name)+3,0,len(name))+name:raise ValueError('actual ClientHello SNI differs from sanctioned name')
  share=extensions.get(51,b'')
  if len(share)<2 or int.from_bytes(share[:2],'big')!=len(share)-2:raise ValueError('invalid client keyshare vector')
  share=share[2:]
 else:
  if extensions.get(43)!=b'\x03\x04':raise ValueError('peer did not select TLS1.3')
  share=extensions.get(51,b'')
 if len(share)!=69 or share[:4]!=b'\0\x17\0A'or share[4]!=4:raise ValueError('Hello key share is not uncompressed P256')
 return dict(tls_version='TLSv1.3',cipher=CIPHER,group='P-256',sni=HOST if client else None)

def reassemble(raw):
 connections={};packets=0
 for packet,frame in enumerate(frames(raw),1):
  if len(frame)<14 or frame[12:14]!=b'\x08\0':continue
  ip=frame[14:]
  if len(ip)<20:raise ValueError('short actual IPv4')
  ihl=(ip[0]&15)*4;length=int.from_bytes(ip[2:4],'big')
  if ip[0]>>4!=4 or ihl<20 or length<ihl or len(ip)<length or checksum(ip[:ihl]):raise ValueError('invalid actual IPv4 checksum/extent')
  if ip[9]!=6:continue
  tcp=ip[ihl:length]
  if int.from_bytes(ip[6:8],'big')&0x3fff or len(tcp)<20 or checksum(ip[12:20]+b'\0\6'+len(tcp).to_bytes(2,'big')+tcp):raise ValueError('fragmented/bad actual TCP checksum')
  sport,dport=struct.unpack_from('!HH',tcp);outbound=ip[12:16]==LOCAL;inbound=ip[16:20]==LOCAL
  if not(outbound or inbound):continue
  if(outbound and ip[16:20]!=PEER)or(inbound and ip[12:16]!=PEER):raise ValueError('guest escaped sanctioned peer address')
  remote=dport if outbound else sport;guest=sport if outbound else dport
  if remote not in(46078,46080)or not 48001<=guest<=65535:raise ValueError('unexpected guest TCP endpoint')
  key=(remote,guest);item=connections.setdefault(key,dict(peer_port=remote,guest_port=guest,outgoing={},incoming={},guest_isn=None,peer_isn=None,handshake=False,fin_out=0,fin_in=0,rst_out=0,rst_in=0,syn=None));packets+=1
  side='outgoing'if outbound else'incoming';own='guest_isn'if outbound else'peer_isn';other='peer_isn'if outbound else'guest_isn';seq,ack=struct.unpack_from('!II',tcp,4);flags=tcp[13];header=(tcp[12]>>4)*4
  if header<20 or header>len(tcp):raise ValueError('invalid actual TCP header extent')
  if flags&2:
   if item[own]not in(None,seq):raise ValueError('actual TCP tuple/initial sequence reused')
   item[own]=seq
   if outbound:
    if flags&16:raise ValueError('guest initiator sent SYN/ACK')
    if item['syn']is None:item['syn']=dict(packet=packet,length=len(frame),hash=hex(fingerprint(frame)))
   elif item[other]is None or not flags&16 or ack!=(item[other]+1)&0xffffffff:raise ValueError('wrong actual peer SYN/ACK')
  if outbound and flags&16 and not flags&2 and item[own]is not None and item[other]is not None and seq==(item[own]+1)&0xffffffff and ack==(item[other]+1)&0xffffffff:item['handshake']=True
  payload=tcp[header:]
  if payload:
   if item[own]is None:raise ValueError('actual payload precedes SYN')
   start=(seq-item[own]-1)&0xffffffff
   if start+len(payload)>65536:raise ValueError('TLS qualification stream exceeded64KiB')
   for offset,value in enumerate(payload,start):
    if offset in item[side]and item[side][offset]!=value:raise ValueError('conflicting actual TCP retransmission')
    item[side][offset]=value
  for flag,label in((1,'fin'),(4,'rst')):
   if flags&flag:item[label+('_out'if outbound else'_in')]+=1
 for item in connections.values():
  if item['guest_isn']is None or item['peer_isn']is None or not item['handshake']:raise ValueError('missing actual TCP three-way handshake')
  for side in('outgoing','incoming'):
   data=item[side]
   if set(data)!=set(range(len(data))):raise ValueError('hole in actual captured stream')
   item[side]=bytes(data[n]for n in range(len(data)))
 return list(connections.values()),packets

def inspect_wire(raw,peer_rows,mode):
 errors=[];joined=[]
 try:
  if mode not in MODES:raise ValueError('unknown controlled TLS mode')
  connections,packets=reassemble(raw);echo=[x for x in connections if x['peer_port']==46078];tls=[x for x in connections if x['peer_port']==46080]
  if len(connections)!=2 or len(echo)!=1 or len(tls)!=1:raise ValueError('expected sole startup Echo and one controlled TLS TCP flow')
  baseline=echo[0]
  if baseline['outgoing']!=ECHO or baseline['incoming']!=ECHO or not baseline['fin_out']or not baseline['fin_in']:raise ValueError('actual baseline Echo/FIN differs')
  if len(peer_rows)!=1:raise ValueError('missing/extra independently observed TLS peer connection')
  row=peer_rows[0];flow=tls[0]
  if row.get('mode')!=mode or row.get('sni')not in(None,HOST):raise ValueError('peer mode/SNI differs')
  if REQUEST in flow['outgoing']or RESPONSE in flow['incoming']or REQUEST in flow['incoming']or RESPONSE in flow['outgoing']:raise ValueError('public application plaintext escaped on TLS wire')
  outgoing=tls_records(flow['outgoing']);incoming=tls_records(flow['incoming'])if mode!='truncated-handshake'else[];client=hello(outgoing[0],True)
  if mode=='ready':
   if row.get('tls_version')!='TLSv1.3'or row.get('cipher')!=CIPHER or row.get('sni')!=HOST or row.get('request_hex')!=REQUEST.hex()or row.get('response_hex')!=RESPONSE.hex()or row.get('unwrap_completed')is not True:raise ValueError('independent actual SSL/application/close_notify proof absent')
   server=hello(incoming[0],False)
   if not any(r['type']==23 for r in outgoing)or not any(r['type']==23 for r in incoming):raise ValueError('missing encrypted TLS records in either direction')
   if not flow['fin_out']or not flow['fin_in']or flow['rst_out']or flow['rst_in']:raise ValueError('ready TLS flow lacks clean bilateral FIN')
  elif mode in REJECTION_MODES:
   if any(r['type']not in(20,21,22)for r in outgoing)or sum(r['type']==22 for r in outgoing)!=1:raise ValueError('rejected peer received guest encrypted/application or extra handshake records')
   if any(r['type']==21 and(len(r['payload'])!=2 or r['payload'][0]!=2)for r in outgoing):raise ValueError('rejected peer sent malformed/nonfatal plaintext TLS alert')
   if row.get('guest_bytes_after_flight')!=0:raise ValueError('negative endpoint observed extra guest bytes after flight')
   if row.get('fixture')!=fixture_descriptor(Path(__file__).resolve().parents[1],mode):raise ValueError('wrong source-bound public rejection fixture')
   if row.get('handshake_complete')is not False or row.get('application_bytes_received')!=0 or row.get('request_hex')or row.get('response_hex')or row.get('unwrap_completed')or row.get('tls_version'):raise ValueError('rejected peer fabricated completed handshake/application')
   if row.get('sent_wire_bytes')!=len(flow['incoming'])or row.get('sent_wire_sha256')!=hashlib.sha256(flow['incoming']).hexdigest():raise ValueError('actual endpoint flight does not match independent PCAP')
   if mode in CERTIFICATE_MODES:
    server=hello(incoming[0],False)
    if row.get('sni')!=HOST or row.get('handshake_started')is not True or not any(r['type']==23 for r in incoming)or row.get('server_flight_complete')is not True:raise ValueError('no actual controlled certificate handshake flight')
   elif mode=='unsupported-version':
    if len(incoming)!=1 or incoming[0]['type']!=21 or incoming[0]['payload']!=b'\x02\x46'or row.get('ssl_error_reason')!='UNSUPPORTED_PROTOCOL':raise ValueError('no actual unsupported-version fatal alert')
    server=None
   elif mode=='truncated-handshake':
    prefix=flow['incoming']
    if len(prefix)!=8 or prefix[:3]!=b'\x16\x03\x03'or int.from_bytes(prefix[3:5],'big')<=len(prefix)-5 or prefix[5]!=2 or row.get('truncated_bytes')!=8 or not isinstance(row.get('full_flight_bytes'),int)or row['full_flight_bytes']<=8:raise ValueError('missing exact intentionally incomplete actual ServerHello flight')
    server=None
   else:
    if incoming or row.get('handshake_stalled')is not True:raise ValueError('timeout stall unexpectedly sent TLS data')
    server=None
  else:
   if incoming or any(r['type']not in(20,22)for r in outgoing):raise ValueError('pending cancellation unexpectedly exchanged peer TLS/application records')
   if row.get('handshake_stalled')is not True or row.get('tls_version')or row.get('request_hex')or row.get('response_hex')or row.get('unwrap_completed'):raise ValueError('pending peer fabricated completed handshake/application')
   server=None
  joined.append(dict(ticket=flow['guest_port']-48000,guest_port=flow['guest_port'],peer_port=46080,syn=flow['syn'],client_hello=client,server_hello=server,outgoing_bytes=len(flow['outgoing']),incoming_bytes=len(flow['incoming']),outgoing_records=[{k:r[k]for k in('type','version','length')}for r in outgoing],incoming_records=[{k:r[k]for k in('type','version','length')}for r in incoming],fin_out=flow['fin_out'],fin_in=flow['fin_in'],rst_out=flow['rst_out'],rst_in=flow['rst_in']))
 except(ValueError,KeyError,IndexError,TypeError,struct.error)as exc:errors.append(str(exc));packets=locals().get('packets',0)
 return dict(ok=not errors,errors=errors,tcp_packets=packets,joins=joined,scope='Independent checksum/TCP reassembly and TLS Hello/outer-record bounds. Ready application identity/close_notify use actual endpoint OpenSSL. Negative modes join actual endpoint flight to PCAP and require guest exact failure/cleanup separately; truncated/stall prove bounded transport cleanup only.')

class ControlledPeer:
 """Explicit loopback endpoint fixtures; no traffic forwarding or key logs."""
 def __init__(self,root,mode):
  self.root=Path(root);self.mode=mode;self.stop=threading.Event();self.pending_hello=threading.Event();self.flight_sent=threading.Event();self.completed=threading.Event();self.errors=[];self.rows=[];self.dns=[];self.attempts=[];self.sockets=[];self.threads=[];self.live=[]
 def context(self):
  ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);ctx.minimum_version=ctx.maximum_version=ssl.TLSVersion.TLSv1_3;ctx.set_ecdh_curve('prime256v1');ctx.num_tickets=0;ctx.keylog_filename=None
  fixture=self.root/'tests/fixtures/tls-controlled'
  if self.mode=='unsupported-version':ctx.minimum_version=ctx.maximum_version=ssl.TLSVersion.TLSv1_2
  if self.mode in CERTIFICATE_MODES:
   # Public DER only; reuse the deliberately public scalar2 fixture key.
   with tempfile.TemporaryDirectory(prefix='sexos-public-tls-')as directory:
    pem=Path(directory)/'public-cert.pem';pem.write_text(ssl.DER_cert_to_PEM_cert(fixture_der(self.root,self.mode)));ctx.load_cert_chain(str(pem),str(fixture/'server-PUBLIC-TEST-key.pem'))
  else:ctx.load_cert_chain(str(fixture/'server.pem'),str(fixture/'server-PUBLIC-TEST-key.pem'))
  def sni(sock,name,context):
   if name!=HOST:raise ssl.SSLError('unexpected controlled SNI')
   sock.controlled_sni=name
  ctx.set_servername_callback(sni);return ctx
 def listener(self,port,udp=False):
  sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM if udp else socket.SOCK_STREAM);sock.bind(('127.0.0.1',port));sock.settimeout(.1)
  if not udp:sock.listen(2)
  self.sockets.append(sock);return sock
 def start(self):
  echo=self.listener(46078);dns=self.listener(46079,True);peer=self.listener(46080);ctx=self.context()
  def guarded(fn,*args):
   try:fn(*args)
   except Exception as exc:
    if not self.stop.is_set():self.errors.append(type(exc).__name__+': '+str(exc))
  for fn,args in((self.serve_echo,(echo,)),(self.serve_dns,(dns,)),(self.serve_tls,(peer,ctx))):
   t=threading.Thread(target=guarded,args=(fn,*args),daemon=True);self.threads.append(t);t.start()
 def accept(self,listener):
  while not self.stop.is_set():
   try:sock,_=listener.accept();sock.settimeout(30);self.live.append(sock);return sock
   except socket.timeout:pass
  return None
 def serve_echo(self,listener):
  sock=self.accept(listener)
  if sock is None:return
  data=bytearray()
  while True:
   chunk=sock.recv(1026)
   if not chunk:break
   data.extend(chunk)
   if len(data)>len(ECHO):raise ValueError('unexpected public startup payload')
  if bytes(data)!=ECHO:raise ValueError('wrong startup Echo bytes')
  sock.sendall(ECHO);sock.shutdown(socket.SHUT_WR);sock.close()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('unexpected extra startup connection')
   except socket.timeout:pass
 def serve_dns(self,sock):
  while not self.stop.is_set():
   try:data,remote=sock.recvfrom(513)
   except socket.timeout:continue
   if len(data)<2:raise ValueError('short actual DNS request')
   ticket=int.from_bytes(data[:2],'big')
   if data!=dns_query(ticket)or self.dns:raise ValueError('unexpected/repeated controlled DNS request')
   response=dns_response(ticket);sent=sock.sendto(response,remote);self.dns.append(dict(ticket=ticket,response_hex=response.hex()));self.attempts.append(dict(ticket=ticket,kind='answer',response_hex=response.hex(),sent_bytes=sent))
 def serve_tls(self,listener,ctx):
  sock=self.accept(listener)
  if sock is None:return
  row=dict(mode=self.mode,sni=None,handshake_stalled=False,tls_version=None,cipher=None,request_hex=None,response_hex=None,unwrap_completed=False);self.rows.append(row)
  if self.mode in('cancel','close','stall'):
   header=sock.recv(5,socket.MSG_PEEK|socket.MSG_WAITALL)
   if len(header)!=5 or header[0]!=22:raise ValueError('pending fixture did not observe a real guest ClientHello')
   row.update(handshake_stalled=True,fixture=fixture_descriptor(self.root,self.mode),handshake_complete=False,application_bytes_received=0,guest_bytes_after_flight=0,sent_wire_bytes=0,sent_wire_sha256=hashlib.sha256(b'').hexdigest());self.pending_hello.set()
   self.stop.wait(160);sock.close();return
  if self.mode in REJECTION_MODES:self.serve_rejected(sock,ctx,row);return
  wrapped=ctx.wrap_socket(sock,server_side=True,do_handshake_on_connect=False);self.live.append(wrapped);wrapped.do_handshake();row.update(sni=getattr(wrapped,'controlled_sni',None),tls_version=wrapped.version(),cipher=wrapped.cipher()[0])
  if row['tls_version']!='TLSv1.3'or row['cipher']!=CIPHER or row['sni']!=HOST:raise ValueError('wrong actual negotiated TLS profile')
  data=bytearray()
  while len(data)<len(REQUEST):
   chunk=wrapped.recv(1024)
   if not chunk:raise ValueError('TLS guest closed before exact request')
   data.extend(chunk)
   if len(data)>len(REQUEST):raise ValueError('extra guest application bytes')
  if bytes(data)!=REQUEST:raise ValueError('wrong actual decrypted request')
  row['request_hex']=bytes(data).hex();wrapped.sendall(RESPONSE);row['response_hex']=RESPONSE.hex();plain=wrapped.unwrap();row['unwrap_completed']=True;plain.close();self.completed.set()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('unexpected extra TLS connection')
   except socket.timeout:pass
 def serve_rejected(self,sock,ctx,row):
  # MemoryBIO exposes the actual endpoint's public handshake flight for a
  # byte-for-byte independent PCAP join. It never forwards guest traffic.
  inbound=ssl.MemoryBIO();outbound=ssl.MemoryBIO();server=ctx.wrap_bio(inbound,outbound,server_side=True);sent=bytearray();sock.settimeout(.1)
  row.update(fixture=fixture_descriptor(self.root,self.mode),handshake_started=False,handshake_complete=False,application_bytes_received=0,server_flight_complete=False,ssl_error_reason=None,guest_bytes_after_flight=0,sent_wire_bytes=0,sent_wire_sha256=hashlib.sha256(b'').hexdigest())
  while not self.stop.is_set():
   try:data=sock.recv(4096)
   except socket.timeout:continue
   except ConnectionResetError:row['transport_reset_observed']=True;break
   if not data:row['transport_eof_observed']=True;break
   if sent:
    row['guest_bytes_after_flight']+=len(data);raise ValueError('negative endpoint received extra guest bytes after rejection flight')
   inbound.write(data);row['handshake_started']=True
   rejected=False
   try:server.do_handshake();row['handshake_complete']=True;raise ValueError('negative endpoint unexpectedly completed TLS handshake')
   except ssl.SSLWantReadError:pass
   except ssl.SSLError as exc:row['ssl_error_reason']=exc.reason;rejected=True
   flight=outbound.read()
   if flight:
    row['sni']=getattr(server,'controlled_sni',None)
    if self.mode=='truncated-handshake':row['full_flight_bytes']=len(flight);row['truncated_bytes']=8;flight=flight[:8]
    sock.sendall(flight);sent.extend(flight);row.update(sent_wire_bytes=len(sent),sent_wire_sha256=hashlib.sha256(sent).hexdigest(),server_flight_complete=self.mode!='truncated-handshake')
    self.flight_sent.set()
    if self.mode=='truncated-handshake':sock.shutdown(socket.SHUT_WR)
   if rejected:break
  sock.close();self.completed.set()
 def close(self):
  self.stop.set()
  for sock in self.sockets+self.live:
   try:sock.close()
   except OSError:pass
  for thread in self.threads:thread.join(timeout=1)

def load_contract(root):
 from tls_prepare_gate import tls_contract
 contract=tls_contract(root);backend=(root/'crates/sex-tls/src/lib.rs').read_text();view=(root/'apps/kaleidoscope/src/browser_view.rs').read_text();app=(root/'apps/kaleidoscope/src/main.rs').read_text()
 for name,expected in(('EXCHANGE_REQUEST',REQUEST),('EXCHANGE_RESPONSE',RESPONSE)):
  match=re.search(r'pub const '+name+r'[^=]*=\s*(b"[^"\n]*");',backend)
  if not match or ast.literal_eval(match[1])!=expected:raise ValueError('frozen public TLS exchange bytes differ: '+name)
 if 'pub fn with_peer('not in view or 'browser_view::with_peer(text,'not in app:raise ValueError('source-bound controlled peer panel changed')
 for literal in ('b"\\n\\nControlled TLS: "','b"\\nY peer B HTTPS Q clear"'):
  if literal not in view:raise ValueError('source-bound controlled peer chrome changed')
 for label in LABELS.values():
  if ('b"'+label.decode()+'"')not in app and ('b"'+label.decode()+'"')not in view:raise ValueError('source-bound controlled peer label changed')
 for name in('apps/kaleidoscope/src/tls_exchange.rs','crates/sex-pdx/src/network_job.rs','servers/sexnet/src/jobs.rs','servers/sexnet/src/main.rs','kernel/src/boot_network_state.rs','kernel/src/init.rs','crates/sex-net-config/src/lib.rs','tests/fixtures/tls-controlled/server.pem','tests/fixtures/tls-controlled/server-PUBLIC-TEST-key.pem','crates/sex-tls/src/probe_fixture.rs'):
  contract['source_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 return contract

def peer_geometry(trace,expected,contract):
 # Independently compose the actual preceding public statuses and panel.
 layouts=list(re.finditer(r'\[browser.tls.peer.layout\] base=(\d+) bytes=64 viewport=(\d+) state=(\w+) pixels=unconfirmed',trace))
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
 status=(b'\n\nControlled TLS: '+LABELS[expected]+b'\nY peer B HTTPS Q clear').ljust(64,b' ')
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
 if not any('Controlled TLS:'in row['text']for row in rows):raise ValueError('controlled TLS label missing')
 return dict(tile=dict(x=x,y=y,w=w,h=h),base=base,viewport=sequence,scale=scale,columns=cols,rows=rows,status_text=LABELS[expected].decode())

def verify_peer_pixels(trace,ppm,expected,contract,resolution=None):
 from check_disk_persistence import ppm_rgb
 from check_desktop_pixels import glyph
 try:
  g=peer_geometry(trace,expected,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=g['tile'];scale=g['scale'];color=contract['color'].to_bytes(3,'big');checked=ink=mismatch=0
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

def inspect_exchange(trace,mode,wire,dns,raw,host_start,host_end):
 from http_content_gate import resolution_joins
 errors=[];joins=[]
 try:
  assert mode in MODES and wire['ok']and dns['ok'],'missing independent actual TLS/DNS wire proof'
  for marker in('[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1'):
   assert trace.count(marker)==1 and trace.count(marker.split(']')[0]+']')==1,'missing/foreign/duplicate exact BOOT grant or raw-source denial'
  private_lines=[line for line in trace.splitlines()if re.search(r'\[(?:browser\.tls|entropy\.)',line)]
  assert not any(re.search(r'\b(?:key|keylog|traffic_secret|master_secret|seed_bytes|seed_value|secret)=',line)for line in private_lines),'private TLS/seed material in metadata'
  intent=re.search(r'\[browser.tls.peer.intent\] accepted=1 explicit=1 public_internet=0',trace);assert intent and trace.count('[browser.tls.peer.intent]')==1,'missing exact actual Y intent'
  policy=re.search(r'\[trust.policy.snapshot\] owner=15 task=15 seq=1 profile=1 root_bytes=376 origin=held_boot_grant',trace);assert policy and trace.count('[trust.policy.snapshot]')==1 and intent.start()<policy.start(),'missing fresh exact policy query'
  sources=browser_clock_sources(trace);source=sources[0]if len(sources)==1 else None;service=re.search(r'\[certificate.time.snapshot\] owner=15 task=15 seq=1 seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace)
  assert source and service and len(sources)==trace.count('[certificate.time.snapshot]')==1 and source[1]==service[1]and policy.start()<source.start()<service.start(),'missing fresh exact UTC receipt'
  assert math.floor(host_start)-1<=int(source[1])<=math.ceil(host_end)+1,'UTC outside actual trusted host bracket'
  seed=re.search(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=1 bytes=32 source=held_boot_grant',trace);produced=re.search(r'\[entropy.seed.progress\] owner=Some\(Identity \{ pd: 15, task: 15 \}\) ticket=1 state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace);released=re.search(r'\[entropy.seed.released\] owner=15 task=15 ticket=1 service_payload=erased receipt=retained',trace)
  assert seed and produced and released and trace.count('[entropy.seed.admitted]')==trace.count('[entropy.seed.released]')==1 and policy.start()<seed.start()<produced.start()<released.start(),'missing fresh produced32/positive seed cleanup'
  check=re.search(r'\[browser.trust.self_check\] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',trace);prep=re.search(r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0',trace)
  assert check and prep and trace.count('[browser.trust.self_check]')==trace.count('[browser.tls.prepared]')==1 and max(released.start(),service.start())<check.start()<prep.start(),'local prerequisite preparation proof missing'
  task,private,initial,utc=map(int,prep.groups());assert 0<task<=32768 and private==task+8248 and private<49152 and 0<initial<=4096 and utc==int(source[1]),'private task/buffer/UTC bound differs'
  assert initial==wire['joins'][0]['outgoing_records'][0]['length']+5,'prepared ClientHello extent differs from first actual TLS record'
  peer_admit=re.search(r'\[browser.tls.peer.admitted\] ticket=(\d+) dns_generation=(\d+) seq=(\d+) task_bytes=(\d+) private_bytes=(\d+) profile=1 authority=held_route',trace);assert peer_admit and trace.count('[browser.tls.peer.admitted]')==1,'missing correlated nonzero duplex lease receipt'
  ticket,generation,seq,atask,aprivate=map(int,peer_admit.groups());assert ticket==wire['joins'][0]['ticket']and ticket==2 and generation==1 and 0<seq<=0xffffff and(atask,aprivate)==(task,private),'wrong peer/DNS/sequence/private join'
  resolution=resolution_joins(trace,raw,[(ticket,generation)],dns);assert resolution['ok'],resolution['errors']
  assert re.search(rf'\[net.job.state\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={ticket} ',trace),'duplex lease lacks actual authenticated owner/task state'
  admission=re.search(rf'\[net.job.duplex.admitted\] owner=15 task=15 ticket={ticket} kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route',trace)
  assert admission and trace.count('[net.job.duplex.admitted]')==1 and prep.start()<admission.start()<peer_admit.start(),'missing exact service-backed duplex admission'
  auth=list(re.finditer(rf'\[browser.tls.peer.authenticated\] ticket={ticket} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1',trace));responses=list(re.finditer(rf'\[browser.tls.peer.response\] ticket={ticket} bytes={len(RESPONSE)} hash=(0x[0-9a-f]+) authenticated=1 source=guest_aead',trace));positive=list(re.finditer(rf'\[browser.tls.peer.released\] ticket={ticket} tx=(\d+) rx=(\d+) request_bytes={len(REQUEST)} response_bytes={len(RESPONSE)} positive_release=1 authenticated=1 public_test_root=1',trace))
  assert len(auth)==trace.count('[browser.tls.peer.authenticated]')and len(responses)==trace.count('[browser.tls.peer.response]')and len(positive)==trace.count('[browser.tls.peer.released]'),'malformed/foreign peer success receipts'
  views=list(re.finditer(r'\[browser.tls.peer.view\] state=(Pending|Verified|Unavailable|Cancelled|Uncertain) controlled_profile=1 public_internet=0',trace));assert len(views)==trace.count('[browser.tls.peer.view]'),'unscoped/unknown peer status'
  assert any(v[1]=='Pending'for v in views),'no actual pending peer status'
  if mode not in REJECTION_MODES:assert '[browser.tls.peer.rejected]'not in trace,'unexpected rejected receipt in successful/pending workflow'
  if mode=='ready':
   assert len(auth)==len(responses)==len(positive)==1,'real peer lacks authentication/AEAD response/positive RELEASE'
   assert responses[0][1]==hex(fingerprint(RESPONSE)),'response receipt differs from exact actual public plaintext'
   assert peer_admit.start()<auth[0].start()<responses[0].start()<positive[0].start(),'peer success ordering differs'
   verified=[v for v in views if v[1]=='Verified'];assert len(verified)==1 and positive[0].start()<verified[0].start(),'Verified published before positive RELEASE'
   assert tuple(map(int,positive[0].groups()))==(wire['joins'][0]['outgoing_bytes'],wire['joins'][0]['incoming_bytes']),'leased byte cursors differ from actual complete TCP streams'
  else:
   assert not auth and not responses and not positive and not any(v[1]=='Verified'for v in views),'pending cancellation invented authenticated response'
   if mode=='cancel':
    cancelled=re.search(rf'\[browser.tls.peer.cancelled\] ticket={ticket} positive_release=1 authenticated=0',trace);view=next((v for v in views if v[1]=='Cancelled'),None)
    assert cancelled and view and peer_admit.start()<cancelled.start()<view.start(),'Q missing actual positive cancel cleanup before Cancelled'
   elif mode in REJECTION_MODES:
    reason=re.escape(rejection_reason(mode));rejected=list(re.finditer(rf'\[browser.tls.peer.rejected\] ticket={ticket} positive_release=1 authenticated=0 failure={reason}(?=\r?$)',trace,re.M));unavailable=[v for v in views if v[1]=='Unavailable']
    assert len(rejected)==trace.count('[browser.tls.peer.rejected]')==len(unavailable)==1,'missing/foreign/duplicate exact rejected reason/positive RELEASE/Unavailable'
    assert peer_admit.start()<rejected[0].start()<unavailable[0].start(),'rejection published before positive cleanup RELEASE'
    assert not any(v[1]in('Cancelled','Uncertain')for v in views),'negative qualification lacks certain terminal cleanup'
   else:
    close=re.search(r'\[browser.tls.peer.closed\] pending=1 terminal=1 owned_storage=erased',trace);ack=re.search(r'\[net.app.cleanup.ack\] target=15 task=15 client_leases=0 shared_cache=retained ok=1',trace)
    assert close and trace.count('[browser.tls.peer.closed]')==1 and ack and peer_admit.start()<close.start()<ack.start(),'Close missed actual pending TLS lease/cleanup'
    assert not re.search(rf'\[net.job.egress.submit\] ticket={ticket} |\[net.dns.egress.submit\] ticket={generation} ',trace[ack.end():]),'retired owner emitted new private egress after cleanupACK'
    for marker in('[trust.policy.retired] target=15 task=15 before_cleanup_ack=1','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1'):
     assert marker in trace and close.start()<trace.index(marker)<ack.start(),'prerequisite retirement did not precede exact cleanupACK'
  joins.append(dict(ticket=ticket,dns_generation=generation,sequence=seq,task_bytes=task,private_bytes=private,initial_tls_bytes=initial,utc_snapshot=utc,policy_offset=policy.start(),prepared_offset=prep.start(),duplex_admitted_offset=admission.start(),client_admitted_offset=peer_admit.start(),authenticated=mode=='ready',dns_resolution=resolution))
 except(AssertionError,ValueError,KeyError,IndexError,TypeError,AttributeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,scope=scope(mode))

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--iso',required=True,type=Path);parser.add_argument('--source-disk',required=True,type=Path);parser.add_argument('--binding',required=True,type=Path);parser.add_argument('--out',required=True,type=Path);parser.add_argument('--mode',choices=MODES,default='ready');parser.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');parser.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=parser.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));held=json.loads(a.binding.read_text());contract=load_contract(root);target=target_check(root)
 if held['runner_sha256'].get('scripts/tls_exchange_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('TLS exchange runner absent from frozen cohort')
 if not target['ok']:raise RuntimeError(str(target))
 peer=ControlledPeer(root,a.mode);peer.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-tls-peer-{os.getpid()}.sock'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',RTC_NORMAL,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}'];profile=command_profile(command)
 if not profile['ok']:raise RuntimeError(str(profile))
 host_start=time.time();manifest=dict(binding=binding,command=command,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),openssl_version=ssl.OPENSSL_VERSION,mode=a.mode,model=a.model,resolution=a.resolution,platform_profile=profile,target=target,host_utc_start=host_start,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),pixel_source_sha256=contract['source_sha256'],tls_peer=dict(endpoint='127.0.0.1:46080',host_forwards=False,minimum_version='TLSv1.2'if a.mode=='unsupported-version'else'TLSv1.3',maximum_version='TLSv1.2'if a.mode=='unsupported-version'else'TLSv1.3',group='P-256',num_tickets=0,keylog_enabled=False,fixture=fixture_descriptor(root,a.mode),public_test_root=True))
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;pixels=[];actions=[];cleanup=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>240:raise TimeoutError('bounded240-second TLS peer gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise RuntimeError('bounded raw evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.01)
  raise TimeoutError(pattern)
 def event(event,description,timeout=30):
  end=time.monotonic()+timeout
  while not event.wait(.01):
   health()
   if time.monotonic()>end:raise TimeoutError(description)
 def key(code,pattern):
  start=len(text());q.key(code);return wait(pattern,start)
 def picture(state,name):
  end=time.monotonic()+20;path=out/(name+'.ppm');result=dict(ok=False,errors=['no frame'])
  while time.monotonic()<end:
   health();q.cmd('screendump',{'filename':str(path)});result=verify_peer_pixels(text(),path.read_bytes(),state,contract,tuple(map(int,a.resolution.split('x'))))
   if result['ok']:break
   time.sleep(.1)
  result.update(screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256']);pixels.append(result);(out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n')
  if not result['ok']:raise RuntimeError(str(result['errors']))
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
  start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',start);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
  start=len(text());q.key('y');wait(r'\[shell.browser.input\] code=21 down=1 slot=43 accepted=1',start);wait(r'\[browser.tls.peer.intent\] accepted=1 explicit=1 public_internet=0',start)
  if a.mode=='ready':
   wait(r'\[browser.tls.peer.view\] state=Verified controlled_profile=1 public_internet=0',start);event(peer.completed,'actual server SSL unwrap/guest close_notify');picture('Verified','peer-verified');actions.append(dict(action='actual_Y_completed_SSL_exchange',application_request_bytes=len(REQUEST),application_response_bytes=len(RESPONSE),server_unwrap_completed=True))
  elif a.mode in REJECTION_MODES:
   wait(r'\[browser.tls.peer.view\] state=Unavailable controlled_profile=1 public_internet=0',start)
   if a.mode!='stall':event(peer.flight_sent,'actual negative endpoint public handshake flight sent')
   picture('Unavailable','peer-unavailable');actions.append(dict(action='actual_Y_negative_endpoint',expected_failure=rejection_reason(a.mode),authenticated=False,transport_timeout_only=a.mode in TRANSPORT_MODES))
  else:
   event(peer.pending_hello,'actual pending guest ClientHello');wait(r'\[browser.tls.peer.admitted\] ticket=2 ',start);picture('Pending','peer-pending');boundary=len(text())
   if a.mode=='cancel':
    q.key('q');wait(r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1',boundary);wait(r'\[browser.tls.peer.view\] state=Cancelled controlled_profile=1 public_internet=0',boundary);picture('Cancelled','peer-cancelled');actions.append(dict(action='actual_Q_pending_server_handshake',authenticated=False))
   else:
    q.key_sequence(['f11']);wait(r'\[browser.tls.peer.closed\] pending=1 terminal=1 owned_storage=erased',boundary);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',boundary);cleanup=cleanup_receipts(text(),15,15,boundary);q.cmd('screendump',{'filename':str(out/'peer-closed.ppm')});actions.append(dict(action='actual_F11_pending_server_handshake',authenticated=False))
  time.sleep(.25);health()
 except Exception as exc:failure=repr(exc)
 finally:
  host_end=time.time();peer.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 manifest['host_utc_end']=host_end;(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=inspect_wire(raw,peer.rows,a.mode);dns=dns_capture(raw,'echo',attempts=peer.attempts,expected=peer.dns)if peer.dns else dict(ok=False,errors=['missing actual fixed-name DNS fixture']);exchange=inspect_exchange(trace,a.mode,wire,dns,raw,host_start,host_end);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 (out/'peer-exchanges.json').write_text(json.dumps(dict(tls=peer.rows,dns=peer.dns,dns_send_attempts=peer.attempts,keylog_enabled=False,scope='Explicit actual endpoint public application observations only; no private/ephemeral TLS material captured.'),indent=2)+'\n')
 result=dict(ok=failure is None and not peer.errors and all(v['ok']for v in(wire,dns,exchange,authority,waiting)),error=failure,server_errors=peer.errors,mode=a.mode,binding=binding,wire=wire,dns=dns,exchange=exchange,authority=authority,self_wait=waiting,pixels=pixels,cleanup=cleanup,actions=actions,scope=scope(a.mode));(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,authenticated=a.mode=='ready',tls=exchange['joins'])))

if __name__=='__main__':main()
