"""Independent public parser/checker fixtures, never guest or TLS-native proof."""
import copy,hashlib,json,socket,ssl,struct,sys,threading,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import tls_exchange_gate as gate
from check_dns_capture import inspect as dns_check
from http_content_gate import load_pixel_contract
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2];UTC=1793370192

def record(kind,payload):return struct.pack('!BHH',kind,0x0303,len(payload))+payload
def hello(client=True,name=gate.HOST):
 def ext(kind,data):return struct.pack('!HH',kind,len(data))+data
 share=b'\0\x17\0A\x04'+bytes(range(64));extensions=ext(43,b'\x02\x03\x04'if client else b'\x03\x04')+ext(51,struct.pack('!H',len(share))+share if client else share)
 if client:
  encoded=name.encode();extensions+=ext(0,struct.pack('!HBH',len(encoded)+3,0,len(encoded))+encoded)
 body=b'\x03\x03'+b'R'*32+b'\0'+(b'\0\x02\x13\x01\x01\0'if client else b'\x13\x01\0')+struct.pack('!H',len(extensions))+extensions
 return record(22,bytes([1 if client else 2])+len(body).to_bytes(3,'big')+body)
def packet(outbound,sport,dport,seq,ack,flags,payload=b'',udp=False):
 src,dst=(gate.LOCAL,gate.PEER)if outbound else(gate.PEER,gate.LOCAL);protocol=17 if udp else 6
 transport=bytearray(struct.pack('!HHHH',sport,dport,8+len(payload),0)+payload)if udp else bytearray(struct.pack('!HHIIBBHHH',sport,dport,seq,ack,5<<4,flags,65535,0,0)+payload)
 check=gate.checksum(src+dst+b'\0'+bytes([protocol])+len(transport).to_bytes(2,'big')+transport);struct.pack_into('!H',transport,6 if udp else 16,check)
 ip=bytearray(struct.pack('!BBHHHBBH4s4s',0x45,0,20+len(transport),1,0,64,protocol,0,src,dst));struct.pack_into('!H',ip,10,gate.checksum(ip));return b'\x52\x54\0\x12\x34\x56'*2+b'\x08\0'+ip+transport

def capture(mode='ready',client=None,server=None,plaintext=False):
 frames=[]
 def flow(port,guest,outgoing,incoming,finished=True):
  frames.extend([packet(True,guest,port,100,0,2),packet(False,port,guest,500,101,0x12),packet(True,guest,port,101,501,0x10),packet(True,guest,port,101,501,0x18,outgoing)])
  if incoming:frames.append(packet(False,port,guest,501,101+len(outgoing),0x18,incoming))
  if finished:frames.extend([packet(True,guest,port,101+len(outgoing),501+len(incoming),0x11),packet(False,port,guest,501+len(incoming),102+len(outgoing),0x11)])
 flow(46078,48001,gate.ECHO,gate.ECHO)
 frames.extend([packet(True,48001,46079,0,0,0,gate.dns_query(1),True),packet(False,46079,48001,0,0,0,gate.dns_response(1),True)])
 outgoing=hello()if client is None else client;incoming=server_wire(mode)if server is None else server
 if mode=='ready':outgoing+=record(23,b'C'*33)
 if plaintext:outgoing+=gate.REQUEST
 flow(46080,48002,outgoing,incoming,mode=='ready')
 raw=b'\xd4\xc3\xb2\xa1'+struct.pack('<HHIIII',2,4,0,0,65535,1)
 for n,frame in enumerate(frames):raw+=struct.pack('<IIII',1,n,len(frame),len(frame))+frame
 return raw

def server_wire(mode):
 if mode=='ready'or mode in gate.CERTIFICATE_MODES:return hello(False)+record(20,b'\1')+record(23,b'S'*33)
 if mode=='unsupported-version':return record(21,b'\x02\x46')
 if mode=='truncated-handshake':return hello(False)[:8]
 return b''
def peer(mode='ready'):
 row=dict(mode=mode,sni=gate.HOST if mode=='ready'else None,handshake_stalled=mode in('cancel','close','stall'),tls_version='TLSv1.3'if mode=='ready'else None,cipher=gate.CIPHER if mode=='ready'else None,request_hex=gate.REQUEST.hex()if mode=='ready'else None,response_hex=gate.RESPONSE.hex()if mode=='ready'else None,unwrap_completed=mode=='ready')
 if mode in gate.REJECTION_MODES:
  flight=server_wire(mode);row.update(fixture=gate.fixture_descriptor(ROOT,mode),handshake_complete=False,application_bytes_received=0,sent_wire_bytes=len(flight),sent_wire_sha256=hashlib.sha256(flight).hexdigest(),handshake_started=True,guest_bytes_after_flight=0,server_flight_complete=mode!='truncated-handshake',sni=gate.HOST if mode in gate.CERTIFICATE_MODES else None,ssl_error_reason='UNSUPPORTED_PROTOCOL'if mode=='unsupported-version'else None,truncated_bytes=8,full_flight_bytes=200)
 return [row]
def metadata(raw,mode='ready'):
 wire=gate.inspect_wire(raw,peer(mode),mode);assert wire['ok'],wire;flow=wire['joins'][0];syn=flow['syn'];rows=['[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.tls.peer.intent] accepted=1 explicit=1 public_internet=0','[trust.policy.snapshot] owner=15 task=15 seq=1 profile=1 root_bytes=376 origin=held_boot_grant',f'[clock.source.snapshot] pd=13 task=13 slot=61 seconds={UTC} attempts=1 precision=1 profile=q35_cmos_utc ordinal=1',f'[certificate.time.snapshot] owner=15 task=15 seq=1 seconds={UTC} source=q35_cmos_utc precision=1 receipt=retained','[entropy.seed.admitted] owner=15 task=15 ticket=1 bytes=32 source=held_boot_grant','[entropy.seed.progress] owner=Some(Identity { pd: 15, task: 15 }) ticket=1 state=Some(Ready) produced=32 consumed=0 failure=0 private=1','[entropy.seed.released] owner=15 task=15 ticket=1 service_payload=erased receipt=retained','[browser.trust.self_check] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',f"[browser.tls.prepared] task_bytes=11064 private_bytes=19312 read=4096 write=2048 tx_bytes={flow['outgoing_records'][0]['length']+5} utc_snapshot={UTC} authenticated=0 network=0",'[browser.dns.released] ticket=1 view=Answer([10, 0, 2, 2]) private_job=0','[net.job.resolved.admitted] owner=15 task=15 ticket=2 dns_generation=1 observed=100 expires=60000 now=101 fixed=1 at_begin=1','[net.job.duplex.admitted] owner=15 task=15 ticket=2 kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route','[browser.tls.peer.admitted] ticket=2 dns_generation=1 seq=40 task_bytes=11064 private_bytes=19312 profile=1 authority=held_route','[net.job.state] owner=Some(Identity { pd: 15, task: 15 }) ticket=2 state=Connecting',f"[net.job.egress.submit] ticket=2 sequence=12 length={syn['length']} hash={syn['hash']} scope=dma_admission_not_delivery",'[browser.tls.peer.view] state=Pending controlled_profile=1 public_internet=0']
 if mode=='ready':rows.extend(['[browser.tls.peer.authenticated] ticket=2 profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1',f'[browser.tls.peer.response] ticket=2 bytes={len(gate.RESPONSE)} hash={hex(gate.fingerprint(gate.RESPONSE))} authenticated=1 source=guest_aead',f"[browser.tls.peer.released] ticket=2 tx={flow['outgoing_bytes']} rx={flow['incoming_bytes']} request_bytes={len(gate.REQUEST)} response_bytes={len(gate.RESPONSE)} positive_release=1 authenticated=1 public_test_root=1",'[browser.tls.peer.view] state=Verified controlled_profile=1 public_internet=0'])
 elif mode=='cancel':rows.extend(['[browser.tls.peer.cancelled] ticket=2 positive_release=1 authenticated=0','[browser.tls.peer.view] state=Cancelled controlled_profile=1 public_internet=0'])
 elif mode in gate.REJECTION_MODES:rows.extend([f'[browser.tls.peer.rejected] ticket=2 positive_release=1 authenticated=0 failure={gate.rejection_reason(mode)}','[browser.tls.peer.view] state=Unavailable controlled_profile=1 public_internet=0'])
 else:rows.extend(['[browser.tls.peer.closed] pending=1 terminal=1 owned_storage=erased','[trust.policy.retired] target=15 task=15 before_cleanup_ack=1','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1','[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1'])
 return '\n'.join(rows)+'\n'

def trace(state,c,w=900,h=650):
 n=len(c['prefix'])+50;base=n+258;dns='Cancelled'if state=='Cancelled'else'Answer([10, 0, 2, 2])'
 return f'[browser.preview.layout] bytes={n} newline=1 cached_or_sample=1\n[browser.net.job.view] state=Cancelled web=0\n[browser.dns.view] state={dns} web=0\n[browser.http.view] state=Cancelled tls=0\n[shell.document.viewport] target=15 task=15 surface=300 sequence=7 x=20 y=100 w={w} h={h} enqueued=1\n[browser.viewport] target=15 task=15 sequence=7 w={w} h={h} origin=held_shell\n[browser.tls.peer.layout] base={base} bytes=64 viewport=7 state={state} pixels=unconfirmed\n[browser.view.applied] bytes={base+64} http=4 body_bytes=0 pixels=unconfirmed\n'
def pixels(text,state,c):
 g=gate.peer_geometry(text,state,c);raw=bytearray(1280*800*3);scale=g['scale'];color=c['color'].to_bytes(3,'big')
 for row in g['rows']:
  for col,ch in enumerate(row['text']):
   for gy,bits in enumerate(glyph(c['font'],ch)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      for dy in range(scale):
       for dx in range(scale):
        x=row['x']+(col*c['advance']+gx)*scale+dx;y=row['y']+gy*scale+dy;at=(y*1280+x)*3;raw[at:at+3]=color
 return b'P6\n1280 800\n255\n'+raw

class ControlledTlsOracle(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=load_pixel_contract(ROOT)
 def inspect(self,text,mode='ready',raw=None,rows=None):
  raw=capture(mode)if raw is None else raw;rows=peer(mode)if rows is None else rows;wire=gate.inspect_wire(raw,rows,mode);dns=dns_check(raw,'echo',expected=[dict(ticket=1,response_hex=gate.dns_response(1).hex())]);return gate.inspect_exchange(text,mode,wire,dns,raw,UTC-5,UTC+5)
 def test_actual_source_contract_and_explicit_no_keylog_server_context(self):
  c=gate.load_contract(ROOT);self.assertIn('apps/kaleidoscope/src/tls_exchange.rs',c['source_sha256']);self.assertIn('crates/sex-tls/src/verifier.rs',c['source_sha256']);ctx=gate.ControlledPeer(ROOT,'ready').context();self.assertEqual(ctx.minimum_version,ssl.TLSVersion.TLSv1_3);self.assertEqual(ctx.maximum_version,ssl.TLSVersion.TLSv1_3);self.assertEqual(ctx.num_tickets,0);self.assertIsNone(ctx.keylog_filename)
 def test_real_cipher_and_name_are_parsed_from_public_hello_bytes(self):
  self.assertEqual(gate.hello(gate.tls_records(hello())[0],True)['sni'],gate.HOST)
  for data in(hello(name='other.sexos.test'),hello().replace(b'\x13\x01',b'\x13\x02',1),hello().replace(b'\0\x17\0A',b'\0\x1d\0A',1)):
   with self.assertRaises(ValueError):gate.hello(gate.tls_records(data)[0],True)
 def test_exact_ready_and_pending_wire_modes(self):
  for mode in gate.MODES:
   r=gate.inspect_wire(capture(mode),peer(mode),mode);self.assertTrue(r['ok'],r);self.assertEqual(r['joins'][0]['ticket'],2)
 def test_truncated_oversized_records_and_bad_checksums_fail(self):
  for data in(b'\x17\x03',b'\x17\x03\x03\xff\xff',record(23,b'x'),record(20,b'wrong')):
   with self.assertRaises(ValueError):gate.tls_records(data)
  raw=bytearray(capture());raw[-1]^=1;self.assertFalse(gate.inspect_wire(bytes(raw),peer(),'ready')['ok'])
 def test_plaintext_wire_and_wrong_server_cipher_name_or_close_are_rejected(self):
  self.assertFalse(gate.inspect_wire(capture(plaintext=True),peer(),'ready')['ok'])
  for field,value in [('sni','outside.test'),('cipher','TLS_AES_256_GCM_SHA384'),('tls_version','TLSv1.2'),('unwrap_completed',False),('request_hex',b'fake'.hex()),('response_hex',b'fake'.hex())]:
   rows=peer();rows[0][field]=value;self.assertFalse(gate.inspect_wire(capture(),rows,'ready')['ok'])
 def test_pending_host_cannot_fabricate_authenticated_application(self):
  rows=peer('cancel');rows[0]['unwrap_completed']=True;self.assertFalse(gate.inspect_wire(capture('cancel'),rows,'cancel')['ok']);self.assertFalse(gate.inspect_wire(capture('ready'),peer('cancel'),'cancel')['ok'])
 def test_exact_actual_authority_flow_and_positive_cleanup_modes(self):
  for mode in gate.MODES:
   r=self.inspect(metadata(capture(mode),mode),mode);self.assertTrue(r['ok'],r)
 def test_missing_foreign_or_ambient_boot_and_policy_fail(self):
  text=metadata(capture())
  for old,new in [('consumer_task=15 profile=1','consumer_task=99 profile=1'),('peer_port=46080 origin=immutable_boot_profile','peer_port=443 origin=immutable_boot_profile'),('origin=held_boot_grant','origin=ambient'),('[browser.trust.source.denied]','[absent.denial]')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'])
 def test_seed_utc_and_prepared_correlation_cannot_be_skipped(self):
  text=metadata(capture())
  for old,new in [('produced=32','produced=24'),('service_payload=erased','service_payload=retained'),('ordinal=1','ordinal=2'),('utc_snapshot='+str(UTC),'utc_snapshot='+str(UTC+1)),('task_bytes=11064','task_bytes=32769')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'])
 def test_dns_positive_release_freshness_and_real_syn_join_are_required(self):
  text=metadata(capture())
  for old,new in [('ticket=1 view=Answer([10, 0, 2, 2])','ticket=1 view=Cancelled'),('dns_generation=1','dns_generation=2'),('expires=60000 now=101','expires=101 now=101'),('hash=0x','hash=0x1'),('owner=15 task=15 ticket=2 kind=10','owner=15 task=99 ticket=2 kind=10')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'])
 def test_fixture_selfcheck_alone_never_proves_peer(self):
  text=metadata(capture())
  for label in('authenticated','response','released'):
   bad='\n'.join(line for line in text.splitlines()if not line.startswith('[browser.tls.peer.'+label+']'))+'\n';self.assertFalse(self.inspect(bad)['ok'])
  self.assertFalse(self.inspect(text.replace('server_finished=1','server_finished=0'))['ok'])
 def test_verified_requires_exact_response_and_actual_byte_cursors(self):
  text=metadata(capture())
  for old,new in [('bytes=41 hash=','bytes=40 hash='),('authenticated=1 source=guest_aead','authenticated=0 source=guest_aead'),('positive_release=1 authenticated=1','positive_release=0 authenticated=1'),('tx='+str(gate.inspect_wire(capture(),peer(),'ready')['joins'][0]['outgoing_bytes']),'tx=1')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'])
  lines=text.splitlines();release=next(i for i,line in enumerate(lines)if line.startswith('[browser.tls.peer.released]'));lines[release],lines[release+1]=lines[release+1],lines[release];self.assertFalse(self.inspect('\n'.join(lines))['ok'])
 def test_pending_cancel_requires_real_cancel_release_before_ui(self):
  text=metadata(capture('cancel'),'cancel');self.assertFalse(self.inspect(text.replace('positive_release=1 authenticated=0','positive_release=0 authenticated=0'),'cancel')['ok']);self.assertFalse(self.inspect(text+'[browser.tls.peer.authenticated] ticket=2 profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1\n','cancel')['ok'])
 def test_pending_close_requires_retirement_before_ack(self):
  text=metadata(capture('close'),'close')
  for old,new in [('pending=1 terminal=1','pending=0 terminal=1'),('owned_storage=erased','owned_storage=retained'),('[trust.policy.retired]','[missing.policy.retired]')]:self.assertFalse(self.inspect(text.replace(old,new),'close')['ok'])
 def test_all_public_bad_certificates_load_same_existing_public_key(self):
  hashes=set()
  for mode in gate.CERTIFICATE_MODES:
   ctx=gate.ControlledPeer(ROOT,mode).context();self.assertIsNone(ctx.keylog_filename);self.assertEqual(ctx.num_tickets,0);hashes.add(gate.fixture_descriptor(ROOT,mode)['der_sha256'])
  self.assertEqual(len(hashes),6);original=gate.fixture_der(ROOT,'ready');corrupt=gate.fixture_der(ROOT,'corrupt-signature');self.assertEqual(original[:-1],corrupt[:-1]);self.assertEqual(original[-1]^1,corrupt[-1])
 def test_negative_receipts_require_exact_reason_cleanup_and_order(self):
  for mode in gate.REJECTION_MODES:
   text=metadata(capture(mode),mode);reason=gate.rejection_reason(mode)
   for old,new in [('ticket=2 positive_release=1 authenticated=0 failure=','ticket=99 positive_release=1 authenticated=0 failure='),('positive_release=1 authenticated=0 failure=','positive_release=0 authenticated=0 failure='),('failure='+reason,'failure=Some(Protocol)'),('[browser.tls.peer.rejected]','[missing.rejection]')]:self.assertFalse(self.inspect(text.replace(old,new),mode)['ok'],(mode,old))
   lines=text.splitlines();lines[-1],lines[-2]=lines[-2],lines[-1];self.assertFalse(self.inspect('\n'.join(lines),mode)['ok'])
   self.assertFalse(self.inspect(text+text.splitlines()[-2]+'\n',mode)['ok'])
   self.assertFalse(self.inspect(text+'[browser.tls.peer.view] state=Verified controlled_profile=1 public_internet=0\n',mode)['ok'])
  text=metadata(capture('wrong-name'),'wrong-name');self.assertFalse(self.inspect(text.replace('failure=Some(Certificate)','failure=None'),'wrong-name')['ok'])
 def test_rejection_wire_requires_actual_fixture_flight_and_no_application(self):
  for mode in gate.CERTIFICATE_MODES:
   for key,value in [('fixture',{}),('sent_wire_sha256','0'*64),('handshake_started',False),('server_flight_complete',False),('handshake_complete',True),('application_bytes_received',38),('guest_bytes_after_flight',1),('request_hex',gate.REQUEST.hex())]:
    rows=peer(mode);rows[0][key]=value;self.assertFalse(gate.inspect_wire(capture(mode),rows,mode)['ok'],(mode,key))
   self.assertFalse(gate.inspect_wire(capture(mode,server=b''),peer(mode),mode)['ok'])
   self.assertFalse(gate.inspect_wire(capture(mode,client=hello()+record(23,b'opaque encrypted app')),peer(mode),mode)['ok'])
 def test_truncation_and_stall_are_transport_scope_not_certificate_claims(self):
  for mode in gate.TRANSPORT_MODES:self.assertEqual(gate.rejection_reason(mode),'None');self.assertIn('without parser-EOF',gate.scope(mode));self.assertFalse(self.inspect(metadata(capture(mode),mode).replace('failure=None','failure=Some(Certificate)'),mode)['ok'])
  for incoming in(b'',hello(False),hello(False)[:7],b'badbytes'):
   rows=peer('truncated-handshake');rows[0].update(sent_wire_bytes=len(incoming),sent_wire_sha256=hashlib.sha256(incoming).hexdigest());self.assertFalse(gate.inspect_wire(capture('truncated-handshake',server=incoming),rows,'truncated-handshake')['ok'])
 def test_unsupported_version_requires_actual_fatal_protocol_alert(self):
  rows=peer('unsupported-version');rows[0]['ssl_error_reason']='UNEXPECTED_EOF_WHILE_READING';self.assertFalse(gate.inspect_wire(capture('unsupported-version'),rows,'unsupported-version')['ok'])
  self.assertEqual(gate.ControlledPeer(ROOT,'unsupported-version').context().maximum_version,ssl.TLSVersion.TLSv1_2)
 def test_actual_openssl_endpoint_observes_flight_without_guest_fin(self):
  # Host endpoint mechanics only; real public flight, socket stays open.
  endpoint=gate.ControlledPeer(ROOT,'wrong-name');server_sock,client_sock=socket.socketpair();client_sock.settimeout(3);row=peer('wrong-name')[0];errors=[]
  def serve():
   try:endpoint.serve_rejected(server_sock,endpoint.context(),row)
   except Exception as exc:errors.append(repr(exc))
  thread=threading.Thread(target=serve);thread.start();ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT);ctx.load_verify_locations(cafile=str(ROOT/'tests/fixtures/tls-controlled/root.pem'));ctx.minimum_version=ctx.maximum_version=ssl.TLSVersion.TLSv1_3;ctx.set_ecdh_curve('prime256v1');incoming=ssl.MemoryBIO();outgoing=ssl.MemoryBIO();client=ctx.wrap_bio(incoming,outgoing,server_side=False,server_hostname=gate.HOST)
  try:
   with self.assertRaises(ssl.SSLWantReadError):client.do_handshake()
   client_sock.sendall(outgoing.read());self.assertTrue(endpoint.flight_sent.wait(3));flight=client_sock.recv(4096);self.assertEqual(len(flight),row['sent_wire_bytes']);self.assertEqual(hashlib.sha256(flight).hexdigest(),row['sent_wire_sha256']);self.assertFalse(endpoint.completed.wait(.35));self.assertTrue(thread.is_alive());self.assertEqual(row['guest_bytes_after_flight'],0);self.assertNotIn('transport_eof_observed',row)
   incoming.write(flight)
   with self.assertRaises(ssl.SSLCertVerificationError):client.do_handshake()
   # Deliberately do not send the host client's alert or FIN. Cancellation
   # qualification uses guest exact failure/RELEASE and the independent PCAP.
  finally:endpoint.stop.set();thread.join(1);client_sock.close();endpoint.close()
  self.assertFalse(thread.is_alive());self.assertFalse(errors,errors);self.assertGreater(row['sent_wire_bytes'],200);self.assertFalse(row['handshake_complete']);self.assertEqual(row['application_bytes_received'],0);self.assertEqual(row['sni'],gate.HOST)
 def test_actual_negative_endpoint_rejects_extra_guest_bytes(self):
  endpoint=gate.ControlledPeer(ROOT,'wrong-name');server_sock,client_sock=socket.socketpair();client_sock.settimeout(3);row=peer('wrong-name')[0];errors=[]
  def serve():
   try:endpoint.serve_rejected(server_sock,endpoint.context(),row)
   except Exception as exc:errors.append(str(exc))
  thread=threading.Thread(target=serve);thread.start();ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT);ctx.check_hostname=False;ctx.verify_mode=ssl.CERT_NONE;ctx.set_ecdh_curve('prime256v1');incoming=ssl.MemoryBIO();outgoing=ssl.MemoryBIO();client=ctx.wrap_bio(incoming,outgoing,server_side=False,server_hostname=gate.HOST)
  try:
   with self.assertRaises(ssl.SSLWantReadError):client.do_handshake()
   client_sock.sendall(outgoing.read());self.assertTrue(endpoint.flight_sent.wait(3));client_sock.recv(4096);client_sock.sendall(b'extra');thread.join(1);self.assertFalse(thread.is_alive());self.assertEqual(row['guest_bytes_after_flight'],5);self.assertTrue(any('extra guest bytes'in e for e in errors))
  finally:endpoint.stop.set();client_sock.close();server_sock.close();thread.join(1);endpoint.close()
 def test_actual_openssl_unsupported_version_emits_fatal_alert(self):
  endpoint=gate.ControlledPeer(ROOT,'unsupported-version');server_sock,client_sock=socket.socketpair();server_sock.settimeout(3);client_sock.settimeout(3);row=peer('unsupported-version')[0];errors=[]
  def serve():
   try:endpoint.serve_rejected(server_sock,endpoint.context(),row)
   except Exception as exc:errors.append(repr(exc))
  thread=threading.Thread(target=serve);thread.start();ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT);ctx.check_hostname=False;ctx.verify_mode=ssl.CERT_NONE;ctx.minimum_version=ctx.maximum_version=ssl.TLSVersion.TLSv1_3
  try:
   with self.assertRaises(ssl.SSLError):ctx.wrap_socket(client_sock,server_hostname=gate.HOST)
  finally:client_sock.close();thread.join(5);endpoint.close()
  self.assertFalse(thread.is_alive());self.assertFalse(errors,errors);self.assertEqual(row['ssl_error_reason'],'UNSUPPORTED_PROTOCOL');self.assertEqual(row['sent_wire_bytes'],7);self.assertFalse(row['handshake_complete']);self.assertEqual(row['application_bytes_received'],0)
 def test_certificate_rejection_cannot_be_relabelled_or_mixed_with_success(self):
  text=metadata(capture('expired'),'expired')
  for line in('[browser.tls.peer.authenticated] ticket=2 profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1','[browser.tls.peer.response] ticket=99 bytes=41 hash=0x1 authenticated=1 source=guest_aead','[browser.tls.peer.rejected] ticket=2 positive_release=1 authenticated=0 failure=None'):
   self.assertFalse(self.inspect(text+line+'\n','expired')['ok'])
  self.assertFalse(self.inspect(metadata(capture())+'[browser.tls.peer.rejected] ticket=2 positive_release=1 authenticated=0 failure=None\n')['ok'])
 def test_complete_status_glyphs_and_trailing_erasure(self):
  for state in gate.LABELS:
   t=trace(state,self.c);r=gate.verify_peer_pixels(t,pixels(t,state,self.c),state,self.c,(1280,800));self.assertTrue(r['ok'],r)
 def test_stale_verified_ink_clipped_or_unapplied_status_fails(self):
  t=trace('Verified',self.c);raw=pixels(t,'Verified',self.c);self.assertFalse(gate.verify_peer_pixels(trace('Cancelled',self.c),raw,'Cancelled',self.c)['ok'])
  for t in(trace('Verified',self.c,h=100),trace('Verified',self.c).replace('[browser.view.applied]','[missing.applied]'),trace('Verified',self.c).replace('viewport=7','viewport=8')):self.assertFalse(gate.verify_peer_pixels(t,b'','Verified',self.c)['ok'])

if __name__=='__main__':unittest.main()
