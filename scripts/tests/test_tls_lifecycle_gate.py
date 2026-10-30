"""Independent public lifecycle fixtures, never native guest acceptance proof."""
import copy,json,os,re,select,shutil,socket,ssl,struct,subprocess,sys,threading,time,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import tls_lifecycle_gate as gate
import test_tls_exchange_gate as fixture
ROOT=Path(__file__).resolve().parents[2]
PLAIN_REQUEST=b'GET /welcome HTTP/1.1\r\nHost: probe.sexos.test\r\nConnection: close\r\nAccept: text/plain\r\n\r\n'

def endpoint_rows():
 return [dict(attempt=n,kind=gate.PLAN[n-1],handshake_complete=True,sni=gate.tls.HOST,tls_version='TLSv1.3',cipher=gate.tls.CIPHER,request_hex=gate.https.REQUEST.hex(),response_hex=gate.response(n).hex(),application_bytes_received=94,extra_application_bytes=0,unwrap_completed=n in gate.FULL)for n in range(1,8)]
def public_rows():return [dict(payload_hex=gate.tls.ECHO.hex(),response_hex=gate.tls.ECHO.hex(),finished=True),dict(payload_hex=PLAIN_REQUEST.hex(),response_hex=gate.plain_response().hex(),finished=True)]
def capture(extra=False,plaintext=False,incoming_overrides=None):
 frames=[]
 def flow(port,ticket,outgoing,incoming,finished):
  incoming=(incoming_overrides or{}).get(ticket,incoming)
  if ticket>1:
   generation=ticket-1;frames.extend([fixture.packet(True,48000+generation,46079,0,0,0,gate.tls.dns_query(generation),True),fixture.packet(False,46079,48000+generation,0,0,0,gate.tls.dns_response(generation),True)])
  guest=48000+ticket;isn=100+ticket*100;peer=200+ticket*100
  frames.extend([fixture.packet(True,guest,port,isn,0,2),fixture.packet(False,port,guest,peer,isn+1,0x12),fixture.packet(True,guest,port,isn+1,peer+1,0x10),fixture.packet(True,guest,port,isn+1,peer+1,0x18,outgoing)])
  if incoming:frames.append(fixture.packet(False,port,guest,peer+1,isn+1+len(outgoing),0x18,incoming))
  if finished:frames.extend([fixture.packet(True,guest,port,isn+1+len(outgoing),peer+1+len(incoming),0x11),fixture.packet(False,port,guest,peer+1+len(incoming),isn+2+len(outgoing),0x11)])
 flow(46078,1,gate.tls.ECHO,gate.tls.ECHO,True)
 for attempt,ticket in enumerate(gate.TLS_TICKETS,1):
  if attempt==7:flow(46078,8,PLAIN_REQUEST,gate.plain_response(),True)
  outgoing=fixture.hello()+fixture.record(23,b'C'*(len(gate.https.REQUEST)+128));incoming=fixture.hello(False)+fixture.record(23,b'S'*(len(gate.response(attempt))+128))
  if plaintext and attempt==5:outgoing+=gate.https.REQUEST
  flow(46080,ticket,outgoing,incoming,attempt in gate.FULL)
 if extra:flow(46080,10,fixture.hello(),fixture.hello(False),False)
 raw=b'\xd4\xc3\xb2\xa1'+struct.pack('<HHIIII',2,4,0,0,65535,1)
 for n,frame in enumerate(frames):raw+=struct.pack('<IIII',1,n,len(frame),len(frame))+frame
 return raw

class LifecycleWireOracle(unittest.TestCase):
 def test_seven_actual_connections_have_distinct_public_bodies(self):
  self.assertEqual(len(set(gate.body(n)for n in range(1,8))),7)
  for n in range(1,8):self.assertEqual(len(gate.body(n)),3072);self.assertTrue(gate.body(n).endswith(f'FINAL LIFECYCLE HTTPS DOCUMENT {n:02d} BYTE Z'.encode()))
  result=gate.inspect_wire(capture(),endpoint_rows(),public_rows(),PLAIN_REQUEST);self.assertTrue(result['ok'],result);self.assertEqual([j['ticket']for j in result['joins']],[2,3,4,5,6,7,9])
 def test_stale_or_reordered_document_cannot_pass_current_connection(self):
  for attempt in gate.FULL:
   rows=endpoint_rows();rows[attempt-1]['response_hex']=gate.response(2 if attempt==1 else 1).hex();self.assertFalse(gate.inspect_wire(capture(),rows,public_rows(),PLAIN_REQUEST)['ok'],attempt)
  rows=endpoint_rows();rows[0],rows[1]=rows[1],rows[0];self.assertFalse(gate.inspect_wire(capture(),rows,public_rows(),PLAIN_REQUEST)['ok'])
 def test_extra_tls_connection_and_plaintext_escape_fail(self):
  for raw in(capture(extra=True),capture(plaintext=True)):self.assertFalse(gate.inspect_wire(raw,endpoint_rows(),public_rows(),PLAIN_REQUEST)['ok'])
 def test_fixture_authentication_application_and_close_notify_are_independent(self):
  for field,value in [('handshake_complete',False),('sni','outside.example'),('cipher','wrong'),('request_hex',gate.tls.REQUEST.hex()),('extra_application_bytes',1),('unwrap_completed',False)]:
   rows=endpoint_rows();rows[0][field]=value;self.assertFalse(gate.inspect_wire(capture(),rows,public_rows(),PLAIN_REQUEST)['ok'],field)
  rows=endpoint_rows();rows[3]['unwrap_completed']=True;self.assertFalse(gate.inspect_wire(capture(),rows,public_rows(),PLAIN_REQUEST)['ok'])
 def test_plain_http_is_exact_and_separate_from_tls(self):
  rows=public_rows();rows[1]['response_hex']=gate.tls.ECHO.hex();self.assertFalse(gate.inspect_wire(capture(),endpoint_rows(),rows,PLAIN_REQUEST)['ok'])

UTC=fixture.UTC
CONTRACT={'pipe_bytes':8248,'stream_ring':1024,'stream_max_wire':13312,'stream_total_ticks':30000,'stream_idle_ticks':5000}
def snapshot(reason,used=0,payload=0,tls=0,route=0,providers=0,queued='None',visible=0):return f'[browser.resources] reason={reason} heap_used={used} heap_free={65536-used} heap_total=65536 tls_payload={payload} tls_pending={tls} route_busy={route} providers_busy={providers} queued={queued} body_visible={visible}\n'
def metadata():
 raw=capture();wire=gate.inspect_wire(raw,endpoint_rows(),public_rows(),PLAIN_REQUEST);assert wire['ok'];flows,_=gate.tls.reassemble(raw);by_ticket={f['guest_port']-48000:f for f in flows};text=fixture.metadata(fixture.capture()).split('[browser.tls.peer.intent]',1)[0];cycle=0
 def queue(kind,document,old=0):
  nonlocal text
  text+=f'[browser.command.queued] kind={kind} document={document} retained=1 accepted_replay=0\n'+snapshot('command',route=int(old!=0),queued='Some('+kind+')')
  if old:text+=f'[browser.tls.cleanup] ticket={old} outcome=Cancelled authenticated=1 document=1 tls_payload=0 positive_release=1\n'+snapshot('change',queued='Some('+kind+')')
 def prepare(kind,document):
  nonlocal text,cycle
  cycle+=1;task=25000 if document else 12000;private=task+8248+(12288 if document else 0);initial=gate.tls.tls_records(fixture.hello())[0]['length']+5;utc=UTC+cycle
  text+=f'[browser.command.started] kind={kind} accepted=1\n[trust.policy.snapshot] owner=15 task=15 seq={cycle} profile=1 root_bytes=376 origin=held_boot_grant\n[clock.source.snapshot] pd=13 task=13 slot=61 seconds={utc} attempts=1 precision=1 profile=q35_cmos_utc ordinal={cycle}\n[certificate.time.snapshot] owner=15 task=15 seq={cycle} seconds={utc} source=q35_cmos_utc precision=1 receipt=retained\n[entropy.seed.admitted] owner=15 task=15 ticket={cycle} bytes=32 source=held_boot_grant\n[entropy.seed.progress] owner=Some(Identity {{ pd: 15, task: 15 }}) ticket={cycle} state=Some(Ready) produced=32 consumed=0 failure=0 private=1\n[entropy.seed.released] owner=15 task=15 ticket={cycle} service_payload=erased receipt=retained\n[browser.trust.self_check] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0\n'
  if document:text+=f'[browser.https.prepared] task_bytes={task} private_bytes={private} tx_bytes={initial} utc_snapshot={utc} request_bytes=94 document_limit=12288 network=0\n'
  text+=f'[browser.tls.prepared] task_bytes={task} private_bytes={private} read=4096 write=2048 tx_bytes={initial} utc_snapshot={utc} authenticated=0 network=0\n'
  return task,private
 def admit(ticket,duplex):
  nonlocal text
  syn=by_ticket[ticket]['syn'];text+=f'[browser.dns.released] ticket={ticket-1} view=Answer([10, 0, 2, 2]) private_job=0\n[net.job.resolved.admitted] owner=15 task=15 ticket={ticket} dns_generation={ticket-1} observed=100 expires=60000 now=101 fixed=1 at_begin=1\n'
  if duplex:text+=f'[net.job.duplex.admitted] owner=15 task=15 ticket={ticket} kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route\n'
  else:text+=f'[net.job.stream.admitted] owner=15 task=15 ticket={ticket} kind=9 ring=1024 max_wire=13312 total_ticks=30000 idle_ticks=5000 authority=held_route\n'
  text+=f'[net.job.egress.submit] ticket={ticket} sequence={ticket+12} length={syn["length"]} hash={syn["hash"]} scope=dma_admission_not_delivery\n[net.job.stream.drain] owner=15 task=15 ticket={ticket} produced={128 if not duplex else 512} consumed=4 buffered={124 if not duplex else 508} ring=1024 state_before=Receiving receipt=retained not_delivery=1\n'
 def fetch(attempt,old=0):
  nonlocal text
  queue('TlsDns',1,old);task,private=prepare('TlsDns',True);ticket=gate.TLS_TICKETS[attempt-1];admit(ticket,True);text+=f'[browser.https.admitted] ticket={ticket} dns_generation={ticket-1} seq={1000+attempt*20} task_bytes={task} private_bytes={private} profile=1 authority=held_route\n[browser.https.authenticated] ticket={ticket} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1\n[browser.https.progress] ticket={ticket} body_received={3072 if attempt in gate.FULL else 1024} authenticated=1 body_visible=0\n'
  if attempt in gate.FULL:
   digest=hex(gate.tls.fingerprint(gate.body(attempt)));flow=wire['joins'][attempt-1];text+=f'[browser.https.body] ticket={ticket} bytes=3072 hash={digest} parsed=1 authenticated=1 visible=0\n[browser.tls.cleanup] ticket={ticket} outcome=Verified authenticated=1 document=1 tls_payload=12288 positive_release=1\n'+snapshot('change',used=12304,payload=12288,visible=1)+f'[browser.https.released] ticket={ticket} tx={flow["tx_bytes"]} rx={flow["rx_bytes"]} request_bytes=94 body_bytes=3072 body_hash={digest} positive_release=1 authenticated=1 public_test_root=1\n'
  else:text+=snapshot('change',used=45584,payload=task+8192+12288,route=1)
 fetch(1);fetch(2);text+=snapshot('clear');fetch(3);fetch(4);fetch(5,old=5);fetch(6);queue('TlsPrepare',0,old=7);prepare('TlsPrepare',False);text+=snapshot('change',used=20304,payload=20192,tls=1);text+=snapshot('clear');queue('TlsPrepare',0);prepare('TlsPrepare',False);text+=snapshot('change',used=20304,payload=20192,tls=1);queue('HttpDns',0);admit(8,False);text+='[browser.http.released] ticket=8 view=Ready private_job=0\n';fetch(7)
 text+='[browser.https.closed] pending=1 terminal=1 owned_storage=erased body_visible=0\n'+snapshot('close')+'[trust.policy.retired] target=15 task=15 before_cleanup_ack=1\n[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased\n[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1\n'
 for role in(1,2,4,8,16):text+=f'[app.service.ack] target=15 task=15 holder=13 role={role} ok=1\n'
 return text+'[net.dns.records.retired] target=15 task=15 remaining=0 before_cleanup_ack=1\n[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1\n[sext.app.pool.ready] target=15 task=15 bytes=512000 ok=1\n'

class LifecycleOwnershipOracle(unittest.TestCase):
 def inspect(self,text):
  raw=capture();wire=gate.inspect_wire(raw,endpoint_rows(),public_rows(),PLAIN_REQUEST);dns=gate.dns_capture(raw,'echo',expected=[dict(ticket=n,response_hex=gate.tls.dns_response(n).hex())for n in range(1,9)]);return gate.inspect_lifecycle(text,wire,dns,raw,UTC-1,UTC+12,CONTRACT)
 def test_exact_same_incarnation_lifecycle_metadata(self):
  r=self.inspect(metadata());self.assertTrue(r['ok'],r);self.assertEqual(len(r['joins']['cycles']),9)
 def test_retained_heap_payload_or_visible_old_body_on_replacement_fails(self):
  text=metadata()
  for old,new in [('reason=command heap_used=0 heap_free=65536','reason=command heap_used=16 heap_free=65520'),('tls_payload=0 tls_pending=0','tls_payload=16 tls_pending=0'),('queued=Some(TlsDns) body_visible=0','queued=Some(TlsDns) body_visible=1'),('heap_total=65536','heap_total=131072')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'],old)
 def test_missing_wrong_or_nonpositive_old_partial_release_fails(self):
  text=metadata()
  for old,new in [('ticket=5 outcome=Cancelled','ticket=99 outcome=Cancelled'),('outcome=Cancelled authenticated=1 document=1 tls_payload=0','outcome=Cancelled authenticated=1 document=1 tls_payload=12288'),('positive_release=1','positive_release=0'),('[browser.tls.cleanup] ticket=7','[missing.cleanup] ticket=7')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'],old)
 def test_cleanup_must_precede_fresh_replacement_start(self):
  lines=metadata().splitlines();cleanup=next(i for i,l in enumerate(lines)if l.startswith('[browser.tls.cleanup] ticket=5'));started=next(i for i in range(cleanup+1,len(lines))if lines[i]=='[browser.command.started] kind=TlsDns accepted=1');lines[cleanup],lines[started]=lines[started],lines[cleanup];self.assertFalse(self.inspect('\n'.join(lines))['ok'])
 def test_fresh_source_sequence_grant_and_shared_preparation_cannot_repeat(self):
  text=metadata()
  for old,new in [('seq=7 profile=1','seq=6 profile=1'),('ordinal=8','ordinal=7'),('produced=32','produced=24'),('private_bytes=45536','private_bytes=45535'),('source=q35_cmos_utc precision=1','source=host precision=1'),('slot=62','slot=18')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'],old)
 def test_command_replay_and_route_fence_regression_fail(self):
  text=metadata()
  for old,new in [('accepted_replay=0','accepted_replay=1'),('seq=1100 task_bytes=','seq=1000 task_bytes='),('kind=TlsPrepare accepted=1','kind=TlsPrepare accepted=0'),('kind=HttpDns document=0','kind=Echo document=0')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'],old)
 def test_stale_full_document_hash_and_premature_publication_fail(self):
  text=metadata();digest=hex(gate.tls.fingerprint(gate.body(5)))
  self.assertFalse(self.inspect(text.replace(digest,hex(gate.tls.fingerprint(gate.body(1)))))['ok']);self.assertFalse(self.inspect(text.replace('visible=0','visible=1'))['ok']);self.assertFalse(self.inspect(text.replace('tls_payload=12288 positive_release=1','tls_payload=20480 positive_release=1'))['ok'])
 def test_l_and_h_and_close_have_no_retained_tls_heap(self):
  text=metadata()
  for old,new in [('tls_payload=20192 tls_pending=1','tls_payload=0 tls_pending=1'),('ticket=8 view=Ready','ticket=8 view=Cancelled'),('reason=close heap_used=0 heap_free=65536','reason=close heap_used=16 heap_free=65520'),('role=16','role=32'),('[trust.policy.retired]','[missing.retirement]')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'],old)
 def test_no_owned_work_after_cleanup_or_final_ack(self):
  text=metadata();self.assertFalse(self.inspect(text+'[net.job.egress.submit] ticket=5 sequence=99\n')['ok']);self.assertFalse(self.inspect(text+'[browser.tls.prepared] task_bytes=1\n')['ok'])
 def test_malformed_or_foreign_resource_snapshot_is_not_silently_ignored(self):
  text=metadata();self.assertFalse(self.inspect(text.replace('queued=None','queued=Some(Ambient)'))['ok']);self.assertFalse(self.inspect(text+'[browser.resources] reason=unknown heap_used=0\n')['ok'])

class MixedDrainOracle(unittest.TestCase):
 def check(self,text=None,raw=None):return gate.mixed_drains(metadata()if text is None else text,capture()if raw is None else raw)
 def test_all_mixed_samples_pass_and_http_only_negative_control_stays_strict(self):
  text=metadata();r=self.check(text);self.assertTrue(r['ok'],r);self.assertEqual({v['ticket']for v in r['samples']},set(range(2,10)));self.assertEqual({v['kind']for v in r['samples']},{9,10});self.assertFalse(gate.http.stream_receipts(text,[8],CONTRACT)['ok'])
 def test_unknown_or_foreign_admitted_kind_owner_and_endpoint_fail(self):
  text=metadata()
  for old,new in [('ticket=8 kind=9','ticket=8 kind=10'),('kind=10 ring=1024','kind=11 ring=1024'),('[net.job.duplex.admitted] owner=15','[net.job.duplex.admitted] owner=99'),('[net.job.stream.admitted]','[net.job.unknown.admitted]'),('peer_port=46080 trust_profile','peer_port=46078 trust_profile')]:self.assertFalse(self.check(text.replace(old,new))['ok'],old)
 def test_drain_unknown_ticket_owner_task_ring_and_receipt_fail(self):
  text=metadata()
  for old,new in [('[net.job.stream.drain] owner=15 task=15 ticket=2','[net.job.stream.drain] owner=15 task=15 ticket=99'),('[net.job.stream.drain] owner=15','[net.job.stream.drain] owner=99'),('[net.job.stream.drain] owner=15 task=15','[net.job.stream.drain] owner=15 task=99'),('ring=1024 state_before','ring=2048 state_before'),('receipt=retained not_delivery=1','receipt=retained not_delivery=0'),('receipt=retained not_delivery=1','receipt=unconfirmed not_delivery=1')]:self.assertFalse(self.check(text.replace(old,new))['ok'],old)
 def test_each_protocol_wire_limit_is_independent_of_capture_capacity(self):
  for ticket,maximum,size in((8,13312,14000),(2,32768,33000)):
   text=metadata();pattern=rf'\[net.job.stream.drain\] owner=15 task=15 ticket={ticket} [^\n]*';replacement=f'[net.job.stream.drain] owner=15 task=15 ticket={ticket} produced={maximum+1} consumed={maximum-1023} buffered=1024 ring=1024 state_before=Receiving receipt=retained not_delivery=1';text=re.sub(pattern,replacement,text);self.assertFalse(self.check(text,capture(incoming_overrides={ticket:b'x'*size}))['ok'],ticket)
 def test_captured_incoming_bounds_and_buffer_arithmetic_fail_closed(self):
  text=metadata()
  for old,new in [('produced=512 consumed=4 buffered=508','produced=5000 consumed=4000 buffered=1000'),('produced=512 consumed=4 buffered=508','produced=512 consumed=513 buffered=0'),('produced=512 consumed=4 buffered=508','produced=512 consumed=4 buffered=507'),('produced=512 consumed=4 buffered=508','produced=512 consumed=0 buffered=512')]:self.assertFalse(self.check(text.replace(old,new))['ok'],old)
 def test_duplicate_or_regressed_consumption_and_complete_state_regression_fail(self):
  text=metadata();line=re.search(r'\[net.job.stream.drain\] owner=15 task=15 ticket=2 [^\n]*',text)[0]
  for extra in(line,line.replace('produced=512 consumed=4 buffered=508','produced=511 consumed=8 buffered=503')):self.assertFalse(self.check(text.replace(line,line+'\n'+extra,1))['ok'])
  complete=line.replace('consumed=4 buffered=508','consumed=8 buffered=504').replace('state_before=Receiving','state_before=Complete');regressed=line.replace('consumed=4 buffered=508','consumed=12 buffered=500');self.assertFalse(self.check(text.replace(line,line+'\n'+complete+'\n'+regressed,1))['ok'])
 def test_no_sample_filtering_and_every_admitted_read_flow_required(self):
  text=metadata();without=re.sub(r'\[net.job.stream.drain\] owner=15 task=15 ticket=7 [^\n]*\n','',text);self.assertFalse(self.check(without)['ok']);self.assertFalse(self.check(text+'[net.job.stream.drain] malformed\n')['ok']);self.assertFalse(self.check(text+'[net.job.stream.drain malformed\n')['ok'])
 def test_drain_before_admission_or_after_retirement_fails(self):
  text=metadata();line=re.search(r'\[net.job.stream.drain\] owner=15 task=15 ticket=2 [^\n]*',text)[0];self.assertFalse(self.check(line+'\n'+text.replace(line+'\n','',1))['ok']);later=line.replace('consumed=4 buffered=508','consumed=8 buffered=504');self.assertFalse(self.check(text+later+'\n')['ok'])

class LifecycleReaderOracle(unittest.TestCase):
 def contract(self):return gate.load_contract(ROOT)
 def trace(self,attempt,c,anchor=0):
  text=metadata();release=re.search(r'\[browser.https.released\] ticket='+str(gate.TLS_TICKETS[attempt-1])+r'[^\n]*\n',text);assert release;boundaries=[n for token in('[browser.command.queued]','[browser.resources] reason=clear')if(n:=text.find(token,release.end()))!=-1];following=min(boundaries)if boundaries else -1;text=text[:following]if following!=-1 else text;text+=fixture.trace('Verified',c);frame=gate.http.reader_frame(gate.body(attempt),900,650,c,anchor,b'/welcome');frame.update(tile=dict(x=20,y=100,w=900,h=650),viewport=7)
  text+='[browser.reader.layout] '+ ' '.join(f'{key}={frame[key]}'for key in('first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'))+' origin=authenticated_body tls=1 public_test_root=1 pixels=unconfirmed\n'+f'[browser.view.applied] bytes={frame["bytes"]} http=770 body_bytes=3072 pixels=unconfirmed\n';return text,frame
 def image(self,frame,c):
  import test_https_document_gate as pixels
  return pixels.reader_pixels(frame,c)
 def test_source_contract_and_each_current_document_full_glyphs(self):
  c=self.contract();self.assertIn('sex-rt/src/heap.rs',c['source_sha256']);self.assertIn('apps/kaleidoscope/src/browser_command.rs',c['source_sha256'])
  for attempt in gate.FULL:
   text,frame=self.trace(attempt,c);r=gate.verify_reader_pixels(text,self.image(frame,c),attempt,c);self.assertTrue(r['ok'],(attempt,r));anchor=gate.http.reader_move(frame,'end');text,end=self.trace(attempt,c,anchor);self.assertTrue(gate.verify_reader_pixels(text,self.image(end,c),attempt,c,anchor)['ok']);self.assertIn(f'FINAL LIFECYCLE HTTPS DOCUMENT {attempt:02d} BYTE Z',''.join(line.decode()for line in end['lines'][2:-1]))
 def test_old_body_or_late_supersession_cannot_pass_new_reader(self):
  c=self.contract();text,frame=self.trace(2,c);_,old=self.trace(1,c);self.assertFalse(gate.verify_reader_pixels(text,self.image(old,c),2,c)['ok'])
  for late in('[browser.command.queued] kind=TlsDns document=1 retained=1 accepted_replay=0','[browser.https.cleared] body_visible=0 owned_storage=erased network=0'):
   self.assertFalse(gate.verify_reader_pixels(text+late+'\n',self.image(frame,c),2,c)['ok'])
 def test_warm_private_l_requires_actual_dynamic_http_status_applied(self):
  c=self.contract();text=fixture.trace('HelloPending',c).replace('browser.tls.peer.layout','browser.tls.layout');g=gate.private_geometry(text,'HelloPending',c);self.assertEqual(g['base'],len(c['prefix'])+50+258)
  with self.assertRaises(ValueError):gate.private_geometry(text.replace('http=4','http=0'),'HelloPending',c)

class LifecycleActualEndpoint(unittest.TestCase):
 def test_actual_ssl_replacements_while_old_partial_connection_is_custodied(self):
  endpoint=gate.LifecyclePeer(ROOT,PLAIN_REQUEST);listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(8);listener.settimeout(.1);endpoint.sockets.append(listener);errors=[];processes=[]
  def serve():
   try:endpoint.serve_tls(listener,endpoint.context())
   except Exception as exc:
    if not endpoint.stop.is_set():errors.append(repr(exc))
  thread=threading.Thread(target=serve);thread.start();executable=shutil.which('openssl');self.assertIsNotNone(executable)
  try:
   for attempt in range(1,8):
    proc=subprocess.Popen([executable,'s_client','-connect','127.0.0.1:'+str(listener.getsockname()[1]),'-servername',gate.tls.HOST,'-verify_return_error','-CAfile',str(ROOT/'tests/fixtures/tls-controlled/root.pem'),'-tls1_3','-groups','P-256','-ciphersuites',gate.tls.CIPHER,'-quiet','-no_ign_eof'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE);processes.append(proc);proc.stdin.write(gate.https.REQUEST);proc.stdin.flush();expected=gate.response(attempt);actual=bytearray();deadline=time.monotonic()+5
    while len(actual)<len(expected):
     ready,_,_=select.select([proc.stdout],[],[],max(0,deadline-time.monotonic()));self.assertTrue(ready,errors);chunk=os.read(proc.stdout.fileno(),len(expected)-len(actual));self.assertTrue(chunk,errors);actual.extend(chunk)
    self.assertEqual(bytes(actual),expected);self.assertTrue(endpoint.sent[attempt].wait(3))
    if attempt in gate.FULL:proc.stdin.close();self.assertEqual(proc.wait(timeout=3),0);self.assertTrue(endpoint.finished[attempt].wait(3))
    else:self.assertIsNone(proc.poll());self.assertFalse(endpoint.finished[attempt].is_set())
    if attempt==5:self.assertIsNone(processes[3].poll());self.assertFalse(endpoint.rows[3]['unwrap_completed']);self.assertTrue(endpoint.rows[4]['unwrap_completed'])
   self.assertEqual(len(endpoint.rows),7);self.assertFalse(errors);self.assertEqual([row['application_bytes_received']for row in endpoint.rows],[94]*7);self.assertEqual([row['unwrap_completed']for row in endpoint.rows],[n in gate.FULL for n in range(1,8)])
  finally:
   endpoint.close();thread.join(1)
   for proc in processes:
    if proc.poll()is None:proc.kill();proc.wait()
    for stream in(proc.stdin,proc.stdout,proc.stderr):
     if stream:stream.close()
  self.assertFalse(thread.is_alive());self.assertFalse(errors,errors)

if __name__=='__main__':unittest.main()
