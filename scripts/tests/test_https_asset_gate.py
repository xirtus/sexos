"""Public checker fixtures only; synthetic PCAP/PPM never native acceptance."""
import copy,hashlib,json,os,re,select,shutil,socket,struct,subprocess,sys,tempfile,threading,time,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import https_asset_gate as gate
import test_tls_exchange_gate as fixture
ROOT=Path(__file__).resolve().parents[2]
UTC=fixture.UTC

def rows(mode='ready'):
 return [dict(attempt=i,handshake_complete=True,sni=gate.https.tls.HOST,tls_version='TLSv1.3',cipher=gate.https.tls.CIPHER,request_hex=gate.REQUEST.hex(),response_hex=gate.response(mode).hex(),application_bytes_received=len(gate.REQUEST),extra_application_bytes=0,unwrap_completed=not mode.startswith('partial-')and mode!='wrong-mime')for i in range(1,3 if mode=='ready'else 2)]

def capture(mode='ready',plaintext=False,extra=False,incoming_tail=b'',outgoing_tail=b''):
 frames=[]
 def flow(port,ticket,outgoing,incoming):
  guest=48000+ticket;isn=100+ticket*100;peer=500+ticket*100
  frames.extend([fixture.packet(True,guest,port,isn,0,2),fixture.packet(False,port,guest,peer,isn+1,0x12),fixture.packet(True,guest,port,isn+1,peer+1,0x10),fixture.packet(True,guest,port,isn+1,peer+1,0x18,outgoing),fixture.packet(False,port,guest,peer+1,isn+1+len(outgoing),0x18,incoming),fixture.packet(True,guest,port,isn+1+len(outgoing),peer+1+len(incoming),0x11),fixture.packet(False,port,guest,peer+1+len(incoming),isn+2+len(outgoing),0x11)])
 flow(46078,1,gate.https.tls.ECHO,gate.https.tls.ECHO)
 for i in range(1,3 if mode=='ready'else 2):
  frames.extend([fixture.packet(True,48000+i,46079,0,0,0,gate.https.tls.dns_query(i),True),fixture.packet(False,46079,48000+i,0,0,0,gate.https.tls.dns_response(i),True)])
  outgoing=fixture.hello()+fixture.record(23,b'C'*(len(gate.REQUEST)+128))+outgoing_tail;incoming=fixture.hello(False)+b''.join(fixture.record(23,b'S'*(len(gate.response(mode)[at:at+1024])+17))for at in range(0,len(gate.response(mode)),1024))+incoming_tail
  flow(46080,i+1,outgoing+(gate.REQUEST if plaintext else b''),incoming)
 if extra:flow(46080,4,fixture.hello(),fixture.hello(False))
 raw=b'\xd4\xc3\xb2\xa1'+struct.pack('<HHIIII',2,4,0,0,65535,1)
 for i,frame in enumerate(frames):raw+=struct.pack('<IIII',1,i,len(frame),len(frame))+frame
 return raw

def image_trace(w=512,h=219,ticket=2,viewport=7,x=20,y=100,paint=9):
 scale=min((w-16)//48,(h-(44+18*(2 if w>=256 else 1))-8)//48,4);gy=44+18*(2 if w>=256 else 1)
 return f'''[shell.document.viewport] target=15 task=15 surface=300 sequence={viewport} x={x} y={y} w={w} h={h} enqueued=1
[browser.viewport] target=15 task=15 sequence={viewport} w={w} h={h} origin=held_shell
[browser.asset.released] ticket={ticket} tx=200 rx=12000 body_bytes=11542 body_hash={hex(gate.fingerprint(gate.BODY))} positive_release=1 authenticated=1
[browser.asset.decoded] ticket={ticket} width=48 height=48 pixels=2304 hash={hex(gate.PIXEL_HASH)} bounded_turn=128
[display.text.committed] target=15 surface=300 paint=8 bytes=399 changed=1 pixels=unconfirmed
[browser.asset.text.applied] viewport={viewport} sequence=8 bytes=399 rows=5 positive_ack=1 pixels=unconfirmed
[browser.asset.layout] ticket={ticket} viewport={viewport} x=8 y={gy} width=48 height=48 scale={scale} hash={hex(gate.PIXEL_HASH)} pixels=unconfirmed
[display.bitmap.committed] target=15 surface=300 paint={paint} width=48 height=48 scale={scale} pixels=unconfirmed
[browser.asset.applied] ticket={ticket} viewport={viewport} sequence={paint} x=8 y={gy} width=48 height=48 scale={scale} hash={hex(gate.PIXEL_HASH)} positive_ack=1 pixels=unconfirmed
'''

def ppm(trace,visible=True,shift=(0,0),resolution=(1280,800)):
 g=gate.geometry(trace,False);width,height=resolution;raw=bytearray(bytes(gate.BASE)*(width*height));data=gate.decode_fixture(gate.BODY);tile=g['tile']
 if visible:
  for y in range(48):
   for x in range(48):
    r,green,b,a=data[(y*48+x)*4:(y*48+x+1)*4];color=bytes((c*a+base*(255-a)+127)//255 for c,base in zip((r,green,b),gate.BASE))
    for dy in range(g['scale']):
     for dx in range(g['scale']):
      px=tile['x']+g['x']+x*g['scale']+dx+shift[0];py=tile['y']+g['y']+y*g['scale']+dy+shift[1]
      if 0<=px<width and 0<=py<height:at=(py*width+px)*3;raw[at:at+3]=color
 return f'P6\n{width} {height}\n255\n'.encode()+raw

def metadata(mode='ready'):
 raw=capture(mode);wire=gate.inspect_wire(raw,rows(mode),mode);text='[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0\n[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0\n[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0\n[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0\n[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1\n[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1\n[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1\n'
 syns={}
 for f in gate.https.tls.frames(raw):
  if len(f)>=54 and f[23]==6 and f[26:30]==gate.https.tls.LOCAL and f[47]&0x12==2:syns[int.from_bytes(f[34:36],'big')-48000]=f
 for i,flow in enumerate(wire['joins'],1):
  ticket=i+1;syn=syns[ticket];text+=f'''[shell.browser.input] code=34 down=1 slot=43 accepted=1
[browser.asset.intent] accepted=1 explicit=1 public_internet=0
[trust.policy.snapshot] owner=15 task=15 seq={i} profile=1 root_bytes=376 origin=held_boot_grant
[clock.source.snapshot] pd=13 task=13 slot=61 seconds={UTC} attempts=1 precision=1 profile=q35_cmos_utc ordinal={i}
[certificate.time.snapshot] owner=15 task=15 seq={i} seconds={UTC} source=q35_cmos_utc precision=1 receipt=retained
[entropy.seed.admitted] owner=15 task=15 ticket={i} bytes=32 source=held_boot_grant
[entropy.seed.progress] owner=Some(Identity {{ pd: 15, task: 15 }}) ticket={i} state=Some(Ready) produced=32 consumed=0 failure=0 private=1
[entropy.seed.released] owner=15 task=15 ticket={i} service_payload=erased receipt=retained
[browser.trust.self_check] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0
[browser.tls.prepared] task_bytes=25000 private_bytes=45536 read=4096 write=2048 tx_bytes={flow['initial_tls_bytes']} utc_snapshot={UTC} authenticated=0 network=0
[browser.dns.released] ticket={i} view=Answer([10, 0, 2, 2]) private_job=0
[net.job.resolved.admitted] owner=15 task=15 ticket={ticket} dns_generation={i} observed=100 expires=60000 now=101 fixed=1 at_begin=1
[net.job.egress.submit] ticket={ticket} sequence=1 length={len(syn)} hash={hex(gate.fingerprint(syn))} scope=dma_admission_not_delivery
[net.job.duplex.admitted] owner=15 task=15 ticket={ticket} kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route
[browser.asset.admitted] ticket={ticket} dns_generation={i} seq={i*1000} task_bytes=25000 private_bytes=45536 profile=1 authority=held_route
[net.job.state] owner=Some(Identity {{ pd: 15, task: 15 }}) ticket={ticket} from=Connecting to=Sending staged={flow['initial_tls_bytes']} queued={flow['initial_tls_bytes']} received=0 scope=private_job
[browser.asset.authenticated] ticket={ticket} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1
'''
  if mode in('ready','invalid-qoi'):
   body=gate.response(mode).split(b'\r\n\r\n',1)[1];text+=f'[browser.asset.body] ticket={ticket} bytes={len(body)} hash={hex(gate.fingerprint(body))} parsed=1 authenticated=1 visible=0\n[browser.asset.released] ticket={ticket} tx={flow["outgoing_bytes"]} rx={flow["incoming_bytes"]} body_bytes={len(body)} body_hash={hex(gate.fingerprint(body))} positive_release=1 authenticated=1\n'
   if mode=='ready':
    text+=f'[browser.asset.decoded] ticket={ticket} width=48 height=48 pixels=2304 hash={hex(gate.PIXEL_HASH)} bounded_turn=128\n'
    if i==1:text+='[shell.browser.input] code=23 down=1 slot=43 accepted=1\n[browser.import.rejected] reason=asset_not_text effects=0\n[shell.browser.input] code=16 down=1 slot=43 accepted=1\n[display.bitmap.committed] target=15 surface=300 paint=10 width=0 height=0 scale=0 pixels=unconfirmed\n[browser.asset.cleared] viewport=7 sequence=10 positive_ack=1\n'
   else:text+='[browser.asset.rejected] reason=invalid_qoi effects=0 owned_storage=erased\n'
  elif mode=='wrong-mime':text+='[browser.tls.cleanup] ticket=2 outcome=Unavailable authenticated=1 document=0 tls_payload=0 positive_release=1\n[browser.asset.rejected] ticket=2 positive_release=1 authenticated=1 failure=Some(Application) http=Some(UnsupportedContentType)\n[browser.asset.view] state=Unavailable controlled_profile=1 public_internet=0\n'
  else:
   text+='[browser.asset.progress] ticket=2 body_received=1024 authenticated=1 body_visible=0\n'
   if mode=='partial-clear':text+='[browser.tls.cleanup] ticket=2 outcome=Cancelled authenticated=1 document=0 tls_payload=0 positive_release=1\n'
   else:
    text+='[browser.asset.closed] pending=1 terminal=1 owned_storage=erased\n'
    text+='[trust.policy.retired] target=15 task=15 before_cleanup_ack=1\n[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased\n[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1\n'
    mask=0
    for role,holder in[(1,3),(2,1),(4,11),(8,13),(16,10)]:mask|=role;text+=f'[app.service.ack] target=15 task=15 holder={holder} role={role} acked={mask} required=31\n'
    text+='[net.dns.records.retired] target=15 task=15 remaining=0 before_cleanup_ack=1\n[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1\n[app.private.retired] target=15 task=15 pages=1 bytes=4096 acked=31 required=31 all_flushed=1 pool_reusable=1\n[sext.app.pool.ready] target=15 task=15 bytes=1 ok=1\n'
 return text,wire,raw

class AssetFixture(unittest.TestCase):
 def test_fixed_qoi_and_mixed_alpha_source(self):
  self.assertEqual(len(gate.BODY),11542);self.assertEqual(len(gate.REQUEST),93);self.assertEqual(gate.decode_fixture(gate.BODY),gate.rgba());self.assertEqual(set(gate.rgba()[3::4]),{0,128,255});self.assertIn(b'/art.qoi',gate.REQUEST);self.assertIn(b'Accept: image/qoi',gate.REQUEST)
  with self.assertRaises(ValueError):gate.decode_fixture(gate.BODY[:-1]+b'\2')
 def test_contract_binds_actual_resource_decoder_compositor_sources(self):
  c=gate.source_contract(ROOT);self.assertEqual(set(c['source_sha256']),set(gate.SOURCE_FILES));self.assertEqual(c['pixel_hash'],hex(gate.PIXEL_HASH))
 def test_wire_profiles_bind_endpoint_body_and_no_plaintext(self):
  for mode in gate.MODES:self.assertTrue(gate.inspect_wire(capture(mode),rows(mode),mode)['ok'])
  for mutate in ('extra','plaintext'):
   with self.assertRaises(ValueError):gate.inspect_wire(capture(**{mutate:True}),rows(),'ready')
  for field,value in [('request_hex','00'),('response_hex','00'),('sni','foreign.test'),('unwrap_completed',False),('extra_application_bytes',1),('cipher','wrong')]:
   peer=rows();peer[0][field]=value
   with self.assertRaises(ValueError,msg=field):gate.inspect_wire(capture(),peer,'ready')
 def test_only_negative_incoming_streams_allow_bounded_final_record_prefix(self):
  prefix=fixture.hello(False)+fixture.record(20,b'\1')+fixture.record(23,b'S'*32)
  encrypted=fixture.record(23,b'S'*1041)
  for mode in('wrong-mime','partial-clear','partial-close'):
   for size in(1,2,3,4,5,9,1045):
    with self.subTest(mode=mode,size=size):
     records,tail=gate.incoming_records(prefix+encrypted[:size],mode);self.assertEqual(len(records),3);self.assertEqual(tail['available_bytes'],size);self.assertEqual(tail['declared_bytes'],1041 if size>=5 else None);self.assertFalse(tail['authenticated']);self.assertEqual(tail['sha256'],hashlib.sha256(encrypted[:size]).hexdigest())
   self.assertIsNotNone(gate.inspect_wire(capture(mode,incoming_tail=encrypted[:9]),rows(mode),mode)['joins'][0]['incoming_tail'])
   with self.assertRaises(ValueError):gate.inspect_wire(capture(mode,outgoing_tail=encrypted[:9]),rows(mode),mode)
  for mode in('ready','invalid-qoi'):
   for size in(1,4,5,9):
    with self.assertRaises(ValueError):gate.incoming_records(prefix+encrypted[:size],mode)
   with self.assertRaises(ValueError):gate.inspect_wire(capture(mode,incoming_tail=encrypted[:9]),rows(mode),mode)
 def test_negative_tail_rejects_every_known_bad_header_byte_or_extent(self):
  prefix=fixture.hello(False)+fixture.record(23,b'S'*32)
  for tail in(b'\x16',b'\x17\x02',b'\x17\x03\x01',b'\x17\x03\x03\x05',b'\x17\x03\x03\0\0',b'\x17\x03\x03\0\x10',b'\x17\x03\x03\4\x12'):
   with self.assertRaises(ValueError,msg=tail.hex()):gate.incoming_records(prefix+tail,'wrong-mime')
  for prefix in(b'',fixture.hello(False),fixture.hello(False)[:-4]):
   with self.assertRaises(ValueError):gate.incoming_records(prefix+b'\x17','wrong-mime')
  with self.assertRaises(ValueError):gate.incoming_records(fixture.hello(False)+fixture.record(23,b'S'*1042),'wrong-mime')

class AssetActualEndpoint(unittest.TestCase):
 def exercise_open_client(self,mode,extra=False):
  endpoint=gate.AssetPeer(ROOT,mode);listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(1);listener.settimeout(.1);endpoint.sockets.append(listener);errors=[]
  def serve():
   try:endpoint.serve_tls(listener,endpoint.context())
   except Exception as exc:
    if not endpoint.stop.is_set():errors.append(repr(exc))
  thread=threading.Thread(target=serve);endpoint.threads.append(thread);thread.start();executable=shutil.which('openssl');self.assertIsNotNone(executable)
  # Real local OpenSSL restricts the TLS1.3 ciphersuite and keeps stdin/socket
  # open after the request. Endpoint completion cannot depend on client EOF.
  proc=subprocess.Popen([executable,'s_client','-connect','127.0.0.1:'+str(listener.getsockname()[1]),'-servername',gate.https.tls.HOST,'-verify_return_error','-CAfile',str(ROOT/'tests/fixtures/tls-controlled/root.pem'),'-tls1_3','-groups','P-256','-ciphersuites',gate.https.tls.CIPHER,'-quiet','-no_ign_eof'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  try:
   proc.stdin.write(gate.REQUEST);proc.stdin.flush();expected=gate.response(mode);actual=bytearray();deadline=time.monotonic()+5
   while len(actual)<len(expected):
    ready,_,_=select.select([proc.stdout],[],[],max(0,deadline-time.monotonic()));self.assertTrue(ready,errors);chunk=os.read(proc.stdout.fileno(),len(expected)-len(actual));self.assertTrue(chunk,errors);actual.extend(chunk)
   self.assertEqual(bytes(actual),expected);self.assertTrue(endpoint.completed.wait(.5),errors);self.assertIsNone(proc.poll());self.assertTrue(thread.is_alive());row=endpoint.rows[0]
   self.assertFalse(row['unwrap_completed']);self.assertEqual(row['request_hex'],gate.REQUEST.hex());self.assertEqual(row['response_hex'],expected.hex());self.assertEqual(row['application_bytes_received'],len(gate.REQUEST))
   if extra:
    proc.stdin.write(b'forbidden extra application bytes');proc.stdin.flush();thread.join(2);self.assertFalse(thread.is_alive());self.assertGreater(row['extra_application_bytes'],0);self.assertTrue(any('extra decrypted asset application data'in error for error in errors),errors)
   else:
    time.sleep(.2);self.assertTrue(thread.is_alive());self.assertIsNone(proc.poll());self.assertEqual(row['extra_application_bytes'],0);self.assertFalse(errors,errors)
  finally:
   endpoint.close()
   if proc.poll()is None:proc.kill();proc.wait()
   for stream in(proc.stdin,proc.stdout,proc.stderr):
    if stream:stream.close()
  self.assertFalse(thread.is_alive())
 def test_negative_and_partial_observations_complete_while_actual_tls_client_stays_open(self):
  for mode in('wrong-mime','partial-clear','partial-close'):
   with self.subTest(mode=mode):self.exercise_open_client(mode)
 def test_negative_watch_rejects_extra_actual_decrypted_client_bytes(self):
  self.exercise_open_client('wrong-mime',extra=True)

class AssetPixels(unittest.TestCase):
 def test_complete_scaled_image_at_both_framebuffer_profiles_and_small_tile(self):
  for resolution,w,h in [((1280,800),512,219),((1280,800),1200,700),((1024,768),960,680),((1280,800),120,219)]:
   text=image_trace(w,h,y=60 if h==680 else 100);r=gate.verify_pixels(text,ppm(text,resolution=resolution),True,resolution);self.assertGreater(r['colored_pixels'],100);self.assertGreater(r['checked_pixels'],48*48)
 def test_old_inset_offset_bad_channels_scale_and_missing_text_ack_fail(self):
  text=image_trace();image=ppm(text)
  for dx,dy in [(8,0),(0,36),(-1,0),(0,-1)]:
   with self.assertRaises(ValueError):gate.verify_pixels(text,ppm(text,shift=(dx,dy)))
  for old,new in [('scale=2','scale=3'),('hash='+hex(gate.PIXEL_HASH),'hash=0x123'),('positive_ack=1 pixels=unconfirmed','positive_ack=0 pixels=unconfirmed'),('[browser.asset.text.applied]','[absent.text.applied]'),('origin=held_shell','origin=foreign')]:
   with self.assertRaises((AssertionError,ValueError)):gate.verify_pixels(text.replace(old,new),image)
  bad=bytearray(image);bad[-len(image)+image.index(b'\n255\n')+5+(100+80)*1280*3+(20+8+16)*3]^=1
  with self.assertRaises(ValueError):gate.verify_pixels(text,bytes(bad))
 def test_positive_clear_frame_must_have_no_old_image_pixels(self):
  text=image_trace()+'[browser.asset.cleared] viewport=7 sequence=10 positive_ack=1\n'
  self.assertTrue(gate.verify_pixels(text,ppm(text,False),False)['ok'])
  with self.assertRaises(ValueError):gate.verify_pixels(text,ppm(text),False)
  with self.assertRaises(ValueError):gate.verify_pixels(text,ppm(text),True)
 def test_offscreen_or_oversized_tile_cannot_pass_complete_image_claim(self):
  for changed in(image_trace(x=-1),image_trace(y=790),image_trace(w=1400),image_trace(h=900)):
   with self.assertRaises(ValueError):gate.verify_pixels(changed,ppm(changed))
 def test_applied_requires_matching_display_producer_and_text_erasure(self):
  text=image_trace();image=ppm(text)
  for prefix in('[display.bitmap.committed]','[display.text.committed]'):
   with self.assertRaises(ValueError):gate.verify_pixels(text.replace(prefix,'[absent.commit]'),image)

class AssetTrace(unittest.TestCase):
 def check(self,text,mode='ready',raw=None):
  if raw is None:_,wire,raw=metadata(mode)
  else:wire=gate.inspect_wire(raw,rows(mode),mode)
  dns=gate.dns_capture(raw,'echo',expected=[dict(ticket=i,response_hex=gate.https.tls.dns_response(i).hex())for i in range(1,3 if mode=='ready'else 2)]);return gate.inspect_trace(text,mode,wire,raw,UTC-5,UTC+5,{},dns)
 def test_ready_negative_partial_and_close_have_exact_authority_order(self):
  for mode in gate.MODES:self.assertTrue(self.check(metadata(mode)[0],mode)['ok'])
 def test_begin_receipt_precedes_start_state_and_peer_authentication(self):
  text=metadata()[0];self.assertTrue(self.check(text)['ok'])
  lines=text.splitlines(True)
  state=next(line for line in lines if line.startswith('[net.job.state]'))
  without=text.replace(state,'',1)
  for prefix in('[net.job.duplex.admitted]','[browser.asset.admitted]'):
   at=without.index(prefix)
   with self.assertRaises(ValueError,msg=prefix):self.check(without[:at]+state+without[at:])
  auth=next(line for line in lines if line.startswith('[browser.asset.authenticated]'))
  at=without.index(auth)+len(auth)
  with self.assertRaises(ValueError):self.check(without[:at]+state+without[at:])
 def test_final_encrypted_tail_still_requires_actual_correlated_negative_cleanup(self):
  for mode,prefix in [('wrong-mime','[browser.asset.rejected]'),('partial-clear','[browser.tls.cleanup]'),('partial-close','[net.app.cleanup.ack]')]:
   text=metadata(mode)[0];raw=capture(mode,incoming_tail=fixture.record(23,b'S'*1041)[:9]);self.assertTrue(self.check(text,mode,raw)['ok'])
   with self.assertRaises(ValueError,msg=mode):self.check(text.replace(prefix,'[absent.cleanup]'),mode,raw)
 def test_unreleased_body_decoding_foreign_lease_and_missing_clear_fail(self):
  text=metadata()[0]
  for old,new in [('positive_release=1 authenticated=1','positive_release=0 authenticated=1'),('visible=0','visible=1'),('dns_generation=2','dns_generation=1'),('seq=2 profile=1','seq=1 profile=1'),('service_payload=erased','service_payload=retained'),('[browser.asset.cleared]','[absent.clear]'),('reason=asset_not_text','reason=no_document')]:
   with self.assertRaises(ValueError,msg=old):self.check(text.replace(old,new))
  lines=text.splitlines();a=next(i for i,line in enumerate(lines)if line.startswith('[browser.asset.body]'));lines[a],lines[a+2]=lines[a+2],lines[a]
  with self.assertRaises(ValueError):self.check('\n'.join(lines))
 def test_any_binary_import_mutation_or_partial_publication_fails(self):
  with self.assertRaises(ValueError):self.check(metadata()[0]+'[browser.import.control] forged\n')
  for mode in('partial-clear','partial-close','wrong-mime','invalid-qoi'):
   with self.assertRaises(ValueError):self.check(metadata(mode)[0]+'[browser.asset.applied] forged\n',mode)
 def test_duplicate_intent_body_auth_and_missing_retirement_fail(self):
  text=metadata()[0]
  for prefix in('[browser.asset.intent]','[browser.asset.body]','[browser.asset.authenticated]'):
   line=next(line for line in text.splitlines()if line.startswith(prefix))
   with self.assertRaises(ValueError):self.check(text+line+'\n')
  with self.assertRaises(ValueError):self.check(metadata('partial-close')[0].replace('role=16','role=99'),'partial-close')
 def test_prerequisite_source_join_and_private_leak_mutations_fail(self):
  text=metadata()[0]
  for old,new in [('source=rdseed','source=ambient'),('[browser.clock.source.denied]','[absent.denied]'),('produced=32','produced=24'),('seconds='+str(UTC)+' attempts=1','seconds='+str(UTC+1)+' attempts=1'),('[browser.trust.self_check]','[absent.fixture]'),('[browser.tls.prepared]','[absent.prepared]'),('max_wire=32768','max_wire=65536'),('seq=1000','seq=16777216'),('tx_bytes='+str(gate.inspect_wire(capture(),rows(),'ready')['joins'][0]['initial_tls_bytes']),'tx_bytes=1')]:
   with self.assertRaises(ValueError,msg=old):self.check(text.replace(old,new))
  with self.assertRaises(ValueError):self.check(text+'[browser.asset.debug] traffic_secret=forbidden\n')
  text=metadata('partial-close')[0]
  for changed in(text.replace('[trust.policy.retired]','[absent.retired]'),text.replace('holder=11 role=4','holder=13 role=4'),text.replace('acked=31 required=31','acked=15 required=31'),text+'[net.job.egress.submit] ticket=2\n'):
   with self.assertRaises(ValueError):self.check(changed,'partial-close')
 def test_negative_parser_reason_and_decoder_release_order_are_exact(self):
  text=metadata('wrong-mime')[0]
  for old,new in [('Some(Application)','None'),('Some(UnsupportedContentType)','None'),('authenticated=1 failure','authenticated=0 failure')]:
   with self.assertRaises(ValueError):self.check(text.replace(old,new),'wrong-mime')
  text=metadata('invalid-qoi')[0];line=next(line for line in text.splitlines(True)if line.startswith('[browser.asset.rejected]'))
  with self.assertRaises(ValueError):self.check(line+text.replace(line,''),'invalid-qoi')

def staged_frames():
 trace='[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n'+image_trace();frames=[]
 def append(name,visible):frames.append(dict(name=name,file=name+'.ppm',visible=visible,serial_end=len(trace),serial_sha256=hashlib.sha256(trace.encode()).hexdigest()))
 append('initial-image',True);trace+='[shell.browser.input] code=23 down=1 slot=43 accepted=1\n[browser.import.rejected] reason=asset_not_text effects=0\n[shell.browser.input] code=16 down=1 slot=43 accepted=1\n[display.bitmap.committed] target=15 surface=300 paint=10 width=0 height=0 scale=0 pixels=unconfirmed\n[browser.asset.cleared] viewport=7 sequence=10 positive_ack=1\n';append('cleared-image',False)
 trace+='[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n'+''.join(image_trace(ticket=3,paint=11).splitlines(True)[2:]);append('fresh-image',True);trace+=image_trace(1200,680,ticket=3,viewport=8,y=60,paint=13);append('zoomed-image',True);return trace,frames

class AssetStages(unittest.TestCase):
 def test_exact_frame_phases_visibility_and_fresh_ticket(self):
  trace,frames=staged_frames();gate.validate_frame_stages(trace,'ready',frames)
  for index in(0,2,3):
   wrong=copy.deepcopy(frames);wrong[index]['visible']=False
   with self.assertRaises(ValueError):gate.validate_frame_stages(trace,'ready',wrong)
  for index in(1,2,3):
   wrong=copy.deepcopy(frames);wrong[index]['serial_end']=frames[0]['serial_end'];wrong[index]['serial_sha256']=frames[0]['serial_sha256']
   with self.assertRaises(ValueError):gate.validate_frame_stages(trace,'ready',wrong)
  with self.assertRaises(ValueError):gate.validate_frame_stages(trace,'ready',list(reversed(frames)))
  wrong=copy.deepcopy(frames);wrong[2]['name']='initial-image'
  with self.assertRaises(ValueError):gate.validate_frame_stages(trace,'ready',wrong)
 def test_fresh_frame_cannot_reuse_an_old_ticket_or_publication_sequence(self):
  trace,frames=staged_frames()
  for old,new in [('ticket=3','ticket=2'),('sequence=11 x=8','sequence=9 x=8')]:
   changed=trace.replace(old,new);wrong=copy.deepcopy(frames)
   # Equal-width replacements keep cut points; rehashes cannot fix provenance.
   for shot in wrong:shot['serial_sha256']=hashlib.sha256(changed[:shot['serial_end']].encode()).hexdigest()
   with self.assertRaises((AssertionError,ValueError)):gate.validate_frame_stages(changed,'ready',wrong)

class FrozenEvidence(unittest.TestCase):
 def test_freezer_classifies_certificate_fixtures_as_mandatory_runner_inputs(self):
  fixtures=('tests/fixtures/tls-controlled/server.pem','tests/fixtures/tls-controlled/server-PUBLIC-TEST-key.pem')
  self.assertTrue(all(name not in gate.SOURCE_FILES and name in gate.RUNNER_FILES for name in fixtures))
  with tempfile.TemporaryDirectory()as d:
   root=Path(d);iso=root/'image.iso';iso.write_bytes(b'frozen-source-image');sources={};runners={}
   # Mirror the actual freezer's independent classification, rather than
   # placing inputs in whichever map the checker itself happens to request.
   for name in set(gate.SOURCE_FILES)|set(gate.RUNNER_FILES):
    path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(name.encode())
    group=runners if name.startswith(('scripts/','tests/fixtures/'))else sources
    group[name]=hashlib.sha256(path.read_bytes()).hexdigest()
   sha=hashlib.sha256(iso.read_bytes()).hexdigest();binding=root/'binding.json';binding.write_text(json.dumps(dict(iso_sha256=sha,sources=sources,runner_sha256=runners,build_provenance=dict(build_gate='PASS',source_image_binding='sealed_build_this_run',sources_unchanged_during_build=True,iso_sha256=sha))))
   self.assertEqual(gate.binding_check(root,iso,binding)['iso_sha256'],sha)
   for fixture in fixtures:
    path=root/fixture;original=path.read_bytes();path.write_bytes(original+b'changed-public-fixture')
    with self.assertRaises(ValueError):gate.binding_check(root,iso,binding)
    path.write_bytes(original)
   held=json.loads(binding.read_text());del held['runner_sha256'][fixtures[0]];binding.write_text(json.dumps(held))
   with self.assertRaisesRegex(ValueError,'checker dependency absent'):gate.binding_check(root,iso,binding)
 def test_standalone_checker_rejects_changed_iso_source_and_checker(self):
  with tempfile.TemporaryDirectory()as d:
   root=Path(d);iso=root/'image.iso';iso.write_bytes(b'frozen-iso');sources={};runners={}
   for group,names in[(sources,gate.SOURCE_FILES),(runners,gate.RUNNER_FILES)]:
    for name in names:
     path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(name.encode());group[name]=hashlib.sha256(path.read_bytes()).hexdigest()
   sha=hashlib.sha256(iso.read_bytes()).hexdigest();binding=root/'binding.json';binding.write_text(json.dumps(dict(iso_sha256=sha,sources=sources,runner_sha256=runners,build_provenance=dict(build_gate='PASS',source_image_binding='sealed_build_this_run',sources_unchanged_during_build=True,iso_sha256=sha))))
   self.assertEqual(gate.binding_check(root,iso,binding)['iso_sha256'],sha)
   for path in(iso,root/gate.SOURCE_FILES[0],root/'scripts/check_https_asset.py'):
    old=path.read_bytes();path.write_bytes(old+b'changed')
    with self.assertRaises(ValueError):gate.binding_check(root,iso,binding)
    path.write_bytes(old)
 def test_result_json_is_not_evidence_and_raw_hash_closure_is_mandatory(self):
  with tempfile.TemporaryDirectory()as d:
   out=Path(d);(out/'result.json').write_text('{"ok":true}');(out/'manifest.json').write_text(json.dumps(dict(binding={},source_sha256={},keylog_enabled=False,frames=[],raw_sha256={})))
   with patch.object(gate,'binding_check',return_value={}),patch.object(gate,'source_contract',return_value={'source_sha256':{}}):
    with self.assertRaisesRegex(ValueError,'raw evidence closure'):gate.verify_evidence(ROOT,Path('unused'),Path('unused'),out)
 def test_close_snapshot_still_requires_valid_ppm_and_actual_resolution(self):
  with tempfile.TemporaryDirectory()as directory:
   out=Path(directory);trace,_,raw=metadata('partial-close');iso=out/'frozen.iso';iso.write_bytes(b'public checker fixture');frame=dict(name='closed',file='closed.ppm',visible=False,serial_end=len(trace),serial_sha256=hashlib.sha256(trace.encode()).hexdigest())
   (out/'serial.log').write_text(trace);(out/'wire.pcap').write_bytes(raw);(out/'peer-exchanges.json').write_text(json.dumps(dict(tls=rows('partial-close'),dns=[dict(ticket=1,response_hex=gate.https.tls.dns_response(1).hex())],dns_send_attempts=[],errors=[],keylog_enabled=False)))
   command=gate.desktop.native_command(iso,out/'disk',out/'serial.log',out/'qmp',out/'wire.pcap','e1000e','1280x800')
   for image in(b'',b'P6\n1 1\n255\n\0\0\0'):
    (out/'closed.ppm').write_bytes(image);manifest=dict(binding={},source_sha256={},keylog_enabled=False,frames=[frame],mode='partial-close',resolution='1280x800',command=command,host_utc_start=UTC-5,host_utc_end=UTC+5,raw_sha256={n:hashlib.sha256((out/n).read_bytes()).hexdigest()for n in('serial.log','wire.pcap','peer-exchanges.json','closed.ppm')});(out/'manifest.json').write_text(json.dumps(manifest))
    with patch.object(gate,'binding_check',return_value={}),patch.object(gate,'source_contract',return_value={'source_sha256':{}}):
     with self.assertRaises(ValueError):gate.verify_evidence(ROOT,iso,Path('unused'),out)

if __name__=='__main__':unittest.main()
