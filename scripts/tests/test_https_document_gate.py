"""Public independent HTTPS checker fixtures, never native acceptance evidence."""
import copy,hashlib,json,os,re,select,shutil,socket,ssl,struct,subprocess,sys,threading,time,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import https_document_gate as gate
import tls_exchange_gate as tls
import test_tls_exchange_gate as fixture
ROOT=Path(__file__).resolve().parents[2]

def capture(mode='ready',plaintext=False):
 if mode=='wrong-name':return fixture.capture('wrong-name')
 incoming=fixture.hello(False)+fixture.record(20,b'\1')+fixture.record(23,b'S'*(len(gate.response(mode))+128));outgoing=fixture.hello()+fixture.record(23,b'C'*(len(gate.REQUEST)+128))
 if plaintext:outgoing+=gate.REQUEST
 return fixture.capture('ready'if mode=='ready'else'custom',client=outgoing,server=incoming)

def peer(mode='ready'):
 if mode=='wrong-name':return fixture.peer(mode)
 return [dict(mode=mode,sni=tls.HOST,handshake_complete=True,tls_version='TLSv1.3',cipher=tls.CIPHER,request_hex=gate.REQUEST.hex(),response_hex=gate.response(mode).hex(),application_bytes_received=len(gate.REQUEST),extra_application_bytes=0,unwrap_completed=mode=='ready')]

class HTTPSDocumentOracle(unittest.TestCase):
 def test_source_contract_binds_shared_codec_pipe_and_authenticated_reader(self):
  contract=gate.load_contract(ROOT);self.assertEqual(contract['pipe_bytes'],8248);self.assertEqual(contract['reader_max_body'],12288);self.assertEqual(contract['expected_body_sha256'],hashlib.sha256(gate.BODY).hexdigest())
  for name in('crates/sex-http/src/codec.rs','crates/sex-tls/src/lib.rs','apps/kaleidoscope/src/document_reader.rs','apps/kaleidoscope/src/tls_exchange.rs'):self.assertIn(name,contract['source_sha256'])
 def test_public_guide_is_exact_bounded_ascii_and_last_byte_is_unique(self):
  self.assertEqual(len(gate.BODY),12288);self.assertTrue(gate.BODY.endswith(gate.TAIL));self.assertEqual(gate.BODY.count(gate.TAIL),1);self.assertTrue(all(b==10 or 32<=b<=126 for b in gate.BODY));self.assertIn(b'It is not Unix.',gate.BODY);self.assertEqual(len(gate.REQUEST),94)
 def test_exact_ready_wire_and_authenticated_http_negative_modes(self):
  # Synthetic ready flow needs genuine bilateral FIN in the parser fixture.
  for mode in gate.MODES:
   raw=capture(mode)
   if mode=='ready':raw=fixture.capture('ready',client=fixture.hello()+fixture.record(23,b'C'*(len(gate.REQUEST)+128)),server=fixture.hello(False)+fixture.record(23,b'S'*(len(gate.response(mode))+128)))
   result=gate.inspect_wire(raw,peer(mode),mode);self.assertTrue(result['ok'],(mode,result));self.assertEqual(result['joins'][0]['ticket'],2)
 def test_endpoint_application_identity_and_response_extent_cannot_be_invented(self):
  raw=fixture.capture('ready',client=fixture.hello()+fixture.record(23,b'C'*(len(gate.REQUEST)+128)),server=fixture.hello(False)+fixture.record(23,b'S'*(len(gate.response('ready'))+128)))
  for field,value in [('request_hex',tls.REQUEST.hex()),('response_hex',gate.response('ready')[:-1].hex()),('application_bytes_received',38),('handshake_complete',False),('unwrap_completed',False),('cipher','wrong'),('extra_application_bytes',1)]:
   rows=peer();rows[0][field]=value;self.assertFalse(gate.inspect_wire(raw,rows,'ready')['ok'],field)
  self.assertFalse(gate.inspect_wire(fixture.capture(),peer(),'ready')['ok'])
 def test_http_plaintext_on_wire_and_foreign_sni_fail(self):
  self.assertFalse(gate.inspect_wire(capture('partial-cancel',True),peer('partial-cancel'),'partial-cancel')['ok'])
  rows=peer('partial-close');rows[0]['sni']='outside.example';self.assertFalse(gate.inspect_wire(capture('partial-close'),rows,'partial-close')['ok'])
 def test_bad_certificate_requires_full_flight_zero_http_and_bound_public_der(self):
  raw=capture('wrong-name')
  for field,value in [('application_bytes_received',94),('request_hex',gate.REQUEST.hex()),('fixture',{}),('guest_bytes_after_flight',1),('server_flight_complete',False),('sent_wire_sha256','0'*64)]:
   rows=peer('wrong-name');rows[0][field]=value;self.assertFalse(gate.inspect_wire(raw,rows,'wrong-name')['ok'],field)
 def test_independent_http_fixtures_distinguish_parser_and_partial_cases(self):
  self.assertEqual(gate.response('duplicate-length').count(b'Content-Length:'),2);self.assertIn(b'Content-Length: 12289\r\n',gate.response('oversize'));self.assertIn(b'Content-Type: image/png\r\n',gate.response('unsupported-mime'));head,body=gate.response('partial-cancel').split(b'\r\n\r\n',1);self.assertEqual(len(body),1024);self.assertIn(b'Content-Length: 12288',head);self.assertNotIn(gate.TAIL,body)

def reader_pixels(frame,contract,width=1280,height=800):
 raw=bytearray(width*height*3);tile=frame['tile'];factor=frame['scale'];color=contract['color'].to_bytes(3,'big')
 for row,line in enumerate(frame['lines']):
  for col,ch in enumerate(line):
   for gy,bits in enumerate(gate.glyph(contract['font'],chr(ch))):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      for dy in range(factor):
       for dx in range(factor):
        x=tile['x']+contract['inset']+(col*contract['advance']+gx)*factor+dx;y=tile['y']+contract['top']+(row*contract['line']+gy)*factor+dy;at=(y*width+x)*3;raw[at:at+3]=color
 return b'P6\n'+f'{width} {height}\n255\n'.encode()+raw

class HTTPSReaderPixels(unittest.TestCase):
 def test_entire_https_reader_chrome_body_and_end_marker(self):
  contract=gate.http.load_pixel_contract(ROOT);contract.update(request=gate.REQUEST,reader_header=gate.HEADER)
  for width,height in((900,650),(504,600)):
   frame=gate.http.reader_frame(gate.BODY,width,height,contract,0,b'/welcome');frame.update(tile=dict(x=20,y=100,w=width,h=height));result=gate.frame_pixels(reader_pixels(frame,contract),frame,contract,(1280,800));self.assertTrue(result['ok']);self.assertTrue(frame['lines'][0].startswith(gate.HEADER));self.assertTrue(frame['lines'][1].startswith(b'probe.sexos.test:46080'))
   anchor=gate.http.reader_move(frame,'end');end=gate.http.reader_frame(gate.BODY,width,height,contract,anchor,b'/welcome');end.update(tile=frame['tile']);self.assertEqual(end['end'],end['total_lines']);self.assertIn(gate.TAIL,b''.join(end['lines'][2:-1]));self.assertTrue(gate.frame_pixels(reader_pixels(end,contract),end,contract)['ok']);self.assertEqual(gate.http.reader_move(end,'home'),0)
 def test_stale_body_ink_wrong_banner_and_wrong_origin_pixels_fail(self):
  contract=gate.http.load_pixel_contract(ROOT);contract.update(request=gate.REQUEST,reader_header=gate.HEADER);frame=gate.http.reader_frame(gate.BODY,900,650,contract,0,b'/welcome');frame.update(tile=dict(x=20,y=100,w=900,h=650));image=reader_pixels(frame,contract)
  for index,replacement in((0,b'Plain HTTP / no TLS'),(1,b'foreign.example /welcome'),(2,b'wrong body')):
   bad=copy.deepcopy(frame);bad['lines'][index]=replacement.ljust(frame['cols'],b' ')
   with self.assertRaises(ValueError):gate.frame_pixels(image,bad,contract)

def metadata(mode='ready'):
 base=fixture.metadata(fixture.capture());base=base[:base.index('[browser.tls.peer.authenticated]')].replace('browser.tls.peer.','browser.https.');base=re.sub(r'task_bytes=\d+','task_bytes=25000',base);base=re.sub(r'private_bytes=\d+','private_bytes=45536',base);initial=tls.tls_records(fixture.hello())[0]['length']+5
 marker='[browser.tls.prepared]';at=base.index('\n',base.index(marker));base=base[:at+1]+f'[browser.https.prepared] task_bytes=25000 private_bytes=45536 tx_bytes={initial} utc_snapshot={fixture.UTC} request_bytes=94 document_limit=12288 network=0\n'+base[at+1:]
 if mode!='wrong-name':base+='[browser.https.authenticated] ticket=2 profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1\n'
 if mode=='ready':
  wire=gate.inspect_wire(capture(mode),peer(mode),mode);assert wire['ok'],wire;flow=wire['joins'][0];digest=hex(tls.fingerprint(gate.BODY));base+=f'[browser.https.progress] ticket=2 body_received=12288 authenticated=1 body_visible=0\n[browser.https.body] ticket=2 bytes=12288 hash={digest} parsed=1 authenticated=1 visible=0\n[browser.https.released] ticket=2 tx={flow["outgoing_bytes"]} rx={flow["incoming_bytes"]} request_bytes=94 body_bytes=12288 body_hash={digest} positive_release=1 authenticated=1 public_test_root=1\n[browser.https.view] state=Verified controlled_profile=1 public_internet=0\n'
 elif mode in gate.HTTP_FAILURE or mode=='wrong-name':
  failure='Certificate'if mode=='wrong-name'else'Application';reason='None'if mode=='wrong-name'else'Some('+gate.HTTP_FAILURE[mode]+')';base+=f'[browser.https.rejected] ticket=2 positive_release=1 authenticated={0 if mode=="wrong-name"else 1} failure=Some({failure}) http={reason}\n[browser.https.view] state=Unavailable controlled_profile=1 public_internet=0\n'
 else:
  base+='[browser.https.progress] ticket=2 body_received=1024 authenticated=1 body_visible=0\n'
  if mode=='partial-cancel':base+='[browser.https.cancelled] ticket=2 positive_release=1 authenticated=1\n[browser.https.view] state=Cancelled controlled_profile=1 public_internet=0\n'
  else:
   base+='[browser.https.closed] pending=1 terminal=1 owned_storage=erased body_visible=0\n[trust.policy.retired] target=15 task=15 before_cleanup_ack=1\n[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased\n[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1\n'
   for role in(1,2,4,8,16):base+=f'[app.service.ack] target=15 task=15 holder=13 role={role} ok=1\n'
   base+='[net.dns.records.retired] target=15 task=15 remaining=0 before_cleanup_ack=1\n[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1\n[sext.app.pool.ready] target=15 task=15 bytes=1 ok=1\n'
 return base

class HTTPSAuthorityReceipts(unittest.TestCase):
 def check(self,text,mode='ready'):
  raw=capture(mode);wire=gate.inspect_wire(raw,peer(mode),mode);dns=gate.dns_capture(raw,'echo',expected=[dict(ticket=1,response_hex=tls.dns_response(1).hex())]);return gate.inspect_document(text,mode,wire,dns,raw,fixture.UTC-5,fixture.UTC+5,{'pipe_bytes':8248})
 def test_exact_document_negative_and_partial_authority_modes(self):
  for mode in gate.MODES:self.assertTrue(self.check(metadata(mode),mode)['ok'],(mode,self.check(metadata(mode),mode)))
 def test_missing_foreign_boot_policy_and_prerequisites_fail(self):
  text=metadata()
  for old,new in [('consumer_task=15 profile=1','consumer_task=99 profile=1'),('origin=held_boot_grant','origin=ambient'),('[browser.clock.source.denied]','[absent.clock.denial]'),('produced=32','produced=24'),('service_payload=erased','service_payload=retained'),('slot=62','slot=18')]:self.assertFalse(self.check(text.replace(old,new))['ok'],old)
 def test_source_bound_document_allocation_and_shared_metadata_must_agree(self):
  text=metadata()
  for old,new in [('private_bytes=45536','private_bytes=45535'),('document_limit=12288','document_limit=16384'),('request_bytes=94','request_bytes=38'),('utc_snapshot='+str(fixture.UTC),'utc_snapshot='+str(fixture.UTC+1)),('task_bytes=25000','task_bytes=40000')]:self.assertFalse(self.check(text.replace(old,new))['ok'],old)
  self.assertFalse(self.check(text.replace('private_bytes=45536','private_bytes=45535',1))['ok'])
 def test_dns_positive_release_and_exact_admitted_lease_precede_http(self):
  text=metadata()
  for old,new in [('ticket=1 view=Answer([10, 0, 2, 2])','ticket=1 view=Cancelled'),('dns_generation=1','dns_generation=2'),('expires=60000 now=101','expires=101 now=101'),('authority=held_boot_route','authority=ambient'),('owner=15 task=15 ticket=2 kind=10','owner=15 task=99 ticket=2 kind=10')]:self.assertFalse(self.check(text.replace(old,new))['ok'],old)
 def test_local_fixture_check_cannot_replace_real_authentication_or_release(self):
  text=metadata()
  for prefix in('[browser.https.authenticated]','[browser.https.body]','[browser.https.released]'):
   removed='\n'.join(line for line in text.splitlines()if not line.startswith(prefix))+'\n';self.assertFalse(self.check(removed)['ok'],prefix)
 def test_visible_partial_body_false_hash_and_early_ready_fail(self):
  text=metadata()
  for old,new in [('parsed=1 authenticated=1 visible=0','parsed=1 authenticated=1 visible=1'),('body_visible=0','body_visible=1'),('bytes=12288 hash=','bytes=1024 hash='),('body_hash=0x','body_hash=0x1'),('positive_release=1 authenticated=1','positive_release=0 authenticated=1')]:self.assertFalse(self.check(text.replace(old,new))['ok'],old)
  lines=text.splitlines();lines[-1],lines[-2]=lines[-2],lines[-1];self.assertFalse(self.check('\n'.join(lines))['ok'])
 def test_exact_http_parser_rejection_cannot_be_relabelled_certificate_or_timeout(self):
  for mode in gate.HTTP_FAILURE:
   text=metadata(mode)
   for old,new in [('failure=Some(Application)','failure=None'),('http=Some('+gate.HTTP_FAILURE[mode]+')','http=None'),('authenticated=1 failure=','authenticated=0 failure='),('positive_release=1 authenticated=1 failure=','positive_release=0 authenticated=1 failure=')]:self.assertFalse(self.check(text.replace(old,new),mode)['ok'],(mode,old))
 def test_wrong_name_is_not_an_authenticated_http_document(self):
  text=metadata('wrong-name');self.assertFalse(self.check(text.replace('failure=Some(Certificate)','failure=None'),'wrong-name')['ok']);self.assertFalse(self.check(text+'[browser.https.authenticated] ticket=2 profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1\n','wrong-name')['ok'])
 def test_partial_q_requires_auth_and_cleanup_and_no_reader(self):
  text=metadata('partial-cancel')
  for old,new in [('body_received=1024','body_received=0'),('positive_release=1 authenticated=1','positive_release=0 authenticated=1'),('positive_release=1 authenticated=1','positive_release=1 authenticated=0')]:self.assertFalse(self.check(text.replace(old,new),'partial-cancel')['ok'])
  self.assertFalse(self.check(text+'[browser.reader.layout] tls=1\n','partial-cancel')['ok'])
 def test_duplicate_and_misordered_authority_or_body_receipts_fail(self):
  text=metadata()
  for prefix in('[browser.https.intent]','[browser.https.prepared]','[browser.tls.prepared]','[browser.https.admitted]','[browser.https.authenticated]','[browser.https.body]','[browser.https.released]'):
   line=next(line for line in text.splitlines()if line.startswith(prefix));self.assertFalse(self.check(text+line+'\n')['ok'],prefix)
  lines=text.splitlines();auth=next(i for i,line in enumerate(lines)if line.startswith('[browser.https.authenticated]'));admit=next(i for i,line in enumerate(lines)if line.startswith('[browser.https.admitted]'));lines[auth],lines[admit]=lines[admit],lines[auth];self.assertFalse(self.check('\n'.join(lines))['ok'])
 def test_partial_close_requires_all_service_retirements_before_ack(self):
  text=metadata('partial-close')
  for old,new in [('pending=1 terminal=1','pending=0 terminal=1'),('[trust.policy.retired]','[missing.policy.retired]'),('role=16','role=99'),('remaining=0 before_cleanup_ack=1','remaining=1 before_cleanup_ack=1')]:self.assertFalse(self.check(text.replace(old,new),'partial-close')['ok'],old)

def reader_trace(contract,anchor=0):
 text=metadata()+fixture.trace('Verified',contract);frame=gate.http.reader_frame(gate.BODY,900,650,contract,anchor,b'/welcome');frame.update(tile=dict(x=20,y=100,w=900,h=650),viewport=7)
 values=' '.join(str(frame[k])for k in('first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'));first,end,body_rows,rows,cols,total,source,sequence=values.split();text+=f'[browser.reader.layout] first={first} end={end} body_rows={body_rows} rows={rows} cols={cols} total_lines={total} source_bytes={source} viewport={sequence} origin=authenticated_body tls=1 public_test_root=1 pixels=unconfirmed\n[browser.view.applied] bytes={frame["bytes"]} http=770 body_bytes=12288 pixels=unconfirmed\n';return text,frame

class HTTPSBoundReader(unittest.TestCase):
 def contract(self):
  c=gate.http.load_pixel_contract(ROOT);c.update(request=gate.REQUEST,reader_header=gate.HEADER);return c
 def test_actual_released_document_layout_binds_full_reader_pixels(self):
  c=self.contract();text,frame=reader_trace(c);r=gate.verify_reader_pixels(text,reader_pixels(frame,c),c,0,(1280,800));self.assertTrue(r['ok'],r)
 def test_reader_cannot_open_partial_cleared_or_unreleased_document(self):
  c=self.contract();text,frame=reader_trace(c);image=reader_pixels(frame,c)
  for old,new in [('[browser.https.released]','[missing.release]'),('positive_release=1 authenticated=1','positive_release=0 authenticated=1'),('origin=authenticated_body tls=1','origin=validated_body tls=0'),('http=770','http=258'),('body_bytes=12288','body_bytes=1024'),('public_test_root=1 pixels=unconfirmed','public_test_root=0 pixels=unconfirmed')]:self.assertFalse(gate.verify_reader_pixels(text.replace(old,new),image,c)['ok'],old)
  for late in('[browser.https.cleared] body_visible=0 owned_storage=erased network=0','[browser.https.view] state=Pending controlled_profile=1 public_internet=0','[browser.https.view] state=Unavailable controlled_profile=1 public_internet=0'):self.assertFalse(gate.verify_reader_pixels(text+late+'\n',image,c)['ok'],late)
 def test_geometry_navigation_and_atomic_applied_are_not_serial_proof(self):
  c=self.contract();text,frame=reader_trace(c);image=reader_pixels(frame,c)
  for old,new in [('first=0 end=','first=1 end='),('origin=held_shell','origin=untrusted'),('[browser.view.applied] bytes='+str(frame['bytes']),'[absent.applied] bytes='+str(frame['bytes'])),('sequence=7 w=900','sequence=8 w=900')]:self.assertFalse(gate.verify_reader_pixels(text.replace(old,new),image,c)['ok'],old)
 def test_https_status_panels_have_exact_full_glyphs_and_erasure(self):
  c=self.contract()
  for mode,state in(('ready','Verified'),('partial-cancel','Pending'),('partial-cancel','Cancelled'),('wrong-name','Unavailable'),('duplicate-length','Unavailable')):
   text=fixture.trace(state,c).replace('browser.tls.peer.layout','browser.https.layout');g=gate.status_geometry(text,state,mode,c);width=1280;height=800;raw=bytearray(width*height*3);color=c['color'].to_bytes(3,'big');factor=g['scale']
   for row in g['rows']:
    for col,ch in enumerate(row['text']):
     for gy,bits in enumerate(gate.glyph(c['font'],ch)):
      for gx in range(5):
       if bits&(1<<(4-gx)):
        for dy in range(factor):
         for dx in range(factor):
          x=row['x']+(col*c['advance']+gx)*factor+dx;y=row['y']+gy*factor+dy;at=(y*width+x)*3;raw[at:at+3]=color
   image=b'P6\n1280 800\n255\n'+raw;result=gate.verify_status_pixels(text,image,state,mode,c,(1280,800));self.assertTrue(result['ok'],(state,mode,result))
   self.assertFalse(gate.verify_status_pixels(text.replace('[browser.view.applied]','[missing.applied]'),image,state,mode,c)['ok'])

class HTTPSActualEndpoint(unittest.TestCase):
 def exercise(self,mode):
  endpoint=gate.ControlledHTTPSPeer(ROOT,mode);listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(1);listener.settimeout(.1);endpoint.sockets.append(listener);errors=[]
  def serve():
   try:endpoint.serve_tls(listener,endpoint.context())
   except Exception as exc:
    if not endpoint.stop.is_set():errors.append(repr(exc))
  thread=threading.Thread(target=serve);thread.start();executable=shutil.which('openssl');self.assertIsNotNone(executable)
  # Python SSLContext cannot restrict TLS1.3 ciphersuites; real OpenSSL CLI
  # does so explicitly. This is a local endpoint unit test, not guest proof.
  proc=subprocess.Popen([executable,'s_client','-connect','127.0.0.1:'+str(listener.getsockname()[1]),'-servername',tls.HOST,'-verify_return_error','-CAfile',str(ROOT/'tests/fixtures/tls-controlled/root.pem'),'-tls1_3','-groups','P-256','-ciphersuites',tls.CIPHER,'-quiet','-no_ign_eof'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  try:
   proc.stdin.write(gate.REQUEST);proc.stdin.flush();expected=gate.response(mode);actual=bytearray();deadline=time.monotonic()+5
   while len(actual)<len(expected):
    ready,_,_=select.select([proc.stdout],[],[],max(0,deadline-time.monotonic()));self.assertTrue(ready,errors);chunk=os.read(proc.stdout.fileno(),len(expected)-len(actual));self.assertTrue(chunk,errors);actual.extend(chunk)
   self.assertEqual(bytes(actual),expected);self.assertTrue(endpoint.response_sent.wait(3));proc.stdin.close();self.assertEqual(proc.wait(timeout=3),0)
   if mode=='ready':self.assertTrue(endpoint.completed.wait(3))
   else:self.assertFalse(endpoint.rows[0]['unwrap_completed']);self.assertFalse(endpoint.completed.is_set())
  finally:
   if proc.poll()is None:proc.kill();proc.wait()
   endpoint.close();thread.join(1)
   for stream in(proc.stdin,proc.stdout,proc.stderr):
    if stream:stream.close()
  self.assertFalse(thread.is_alive());self.assertFalse(errors,errors);row=endpoint.rows[0];self.assertEqual(row['request_hex'],gate.REQUEST.hex());self.assertEqual(row['response_hex'],gate.response(mode).hex());self.assertEqual(row['application_bytes_received'],94);self.assertEqual(row['extra_application_bytes'],0);return row
 def test_real_openssl_exact_https_application_and_close_notify(self):
  row=self.exercise('ready');self.assertTrue(row['unwrap_completed']);self.assertTrue(row['handshake_complete']);self.assertEqual(row['tls_version'],'TLSv1.3')
 def test_real_openssl_sends_malformed_and_partial_http_without_completion_claim(self):
  for mode in('duplicate-length','oversize','unsupported-mime','partial-cancel','partial-close'):
   with self.subTest(mode=mode):self.assertFalse(self.exercise(mode)['unwrap_completed'])

if __name__=='__main__':unittest.main()
