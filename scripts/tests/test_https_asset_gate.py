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

def image_trace(w=512,h=219,ticket=2,viewport=7,x=20,y=100,paint=9,generation=4,mode='Fit',source=True):
 tile=dict(w=w,h=h,x=x,y=y);g=gate.image_geometry(tile,mode);frame=gate.asset_text(tile,mode)
 text=f'[shell.document.viewport] target=15 task=15 surface=300 sequence={viewport} x={x} y={y} w={w} h={h} enqueued=1\n[browser.viewport] target=15 task=15 sequence={viewport} w={w} h={h} origin=held_shell\n'
 if source:text+=f'[browser.asset.released] ticket={ticket} tx=200 rx=12000 body_bytes=11542 body_hash={hex(gate.fingerprint(gate.BODY))} positive_release=1 authenticated=1\n[browser.asset.decoded] ticket={ticket} width=48 height=48 pixels=2304 hash={hex(gate.PIXEL_HASH)} bounded_turn=128\n'
 text+=f'[display.text.committed] target=15 surface=300 paint={paint+100} bytes={frame["bytes"]} changed=1 pixels=unconfirmed\n[browser.asset.text.applied] viewport={viewport} sequence={paint+100} bytes={frame["bytes"]} rows={frame["rows"]} positive_ack=1 pixels=unconfirmed\n[browser.asset.presentation.text] generation={generation} viewport={viewport} sequence={paint+100} bytes={frame["bytes"]} rows={frame["rows"]} positive_ack=1\n'
 fields=f'x={g["x"]} y={g["y"]} width=48 height=48 scale={g["scale"]} hash={hex(gate.PIXEL_HASH)}'
 text+=f'[browser.asset.layout] ticket={ticket} viewport={viewport} {fields} pixels=unconfirmed\n[browser.asset.presentation.layout] generation={generation} mode={mode} viewport={viewport} ticket={ticket} sequence={paint} {fields}\n[display.bitmap.committed] target=15 surface=300 paint={paint} width=48 height=48 scale={g["scale"]} pixels=unconfirmed\n[browser.asset.applied] ticket={ticket} viewport={viewport} sequence={paint} {fields} positive_ack=1 pixels=unconfirmed\n[browser.asset.presentation.ready] generation={generation} mode={mode} viewport={viewport} ticket={ticket} sequence={paint} hash={hex(gate.PIXEL_HASH)} positive_ack=1 pixels=unconfirmed\n'
 return text

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
  frame=gate.asset_text(tile,g['mode']);font=gate.font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text());scale=frame['scale']
  for row,line in enumerate(frame['lines'][:frame['chrome_rows']]):
   for column,char in enumerate(line):
    bitmap=gate.glyph(font,char)
    for gy,bits in enumerate(bitmap):
     for gx in range(5):
      if bits&(1<<(4-gx)):
       for dy in range(scale):
        for dx in range(scale):
         px=tile['x']+8+(column*6+gx)*scale+dx;py=tile['y']+36+(row*9+gy)*scale+dy
         if 0<=px<width and 0<=py<height:raw[(py*width+px)*3:(py*width+px)*3+3]=b'\xcd\xd6\xf4'
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
  for old,new in [('scale=1','scale=3'),('hash='+hex(gate.PIXEL_HASH),'hash=0x123'),('positive_ack=1 pixels=unconfirmed','positive_ack=0 pixels=unconfirmed'),('[browser.asset.text.applied]','[absent.text.applied]'),('origin=held_shell','origin=foreign')]:
   with self.assertRaises((AssertionError,ValueError)):gate.verify_pixels(text.replace(old,new),image)
  bad=bytearray(image);g=gate.geometry(text);at=image.index(b'\n255\n')+5+((g['tile']['y']+g['y'])*1280+g['tile']['x']+g['x'])*3;bad[at]^=1
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
 trace='[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n';frames=[];viewports=set()
 def append(name,visible):frames.append(dict(name=name,file=name+'.ppm',visible=visible,serial_end=len(trace),serial_sha256=hashlib.sha256(trace.encode()).hexdigest()))
 def image(name,generation,paint,mode='Fit',viewport=7,w=512,h=219,ticket=2,source=False):
  nonlocal trace
  addition=image_trace(w,h,ticket=ticket,viewport=viewport,paint=paint,generation=generation,mode=mode,source=source)
  if viewport in viewports:addition=''.join(addition.splitlines(True)[2:])
  viewports.add(viewport);trace+=addition;append(name,True)
 def intent(generation,mode,viewport,ticket):
  nonlocal trace
  trace+=f'[sexinput.ps2.scancode] raw={hex(30 if mode=="Actual"else 33)}\n[shell.browser.input] code={30 if mode=="Actual"else 33} down=1 slot=43 accepted=1\n[browser.asset.presentation.intent] generation={generation} mode={mode} viewport={viewport} ticket={ticket} network=0\n'
 image('initial-image',4,9,source=True);intent(5,'Actual',7,2);image('actual-image',5,10,'Actual');intent(6,'Fit',7,2);image('refitted-image',6,11)
 trace+='[shell.browser.input] code=23 down=1 slot=43 accepted=1\n[browser.import.rejected] reason=asset_not_text effects=0\n[shell.browser.input] code=16 down=1 slot=43 accepted=1\n[display.bitmap.committed] target=15 surface=300 paint=12 width=0 height=0 scale=0 pixels=unconfirmed\n[browser.asset.cleared] viewport=7 sequence=12 positive_ack=1\n';append('cleared-image',False)
 trace+='[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n';image('fresh-image',9,13,ticket=3,source=True);image('zoomed-image',10,15,viewport=8,w=1200,h=680,ticket=3);intent(11,'Actual',8,3);image('zoomed-actual',11,16,'Actual',8,1200,680,3);image('restored-actual',12,18,'Actual',9,ticket=3);intent(13,'Fit',9,3);image('restored-fit',13,19,viewport=9,ticket=3)
 return trace,frames

class AssetStages(unittest.TestCase):
 def test_exact_frame_phases_visibility_and_fresh_ticket(self):
  trace,frames=staged_frames();gate.validate_frame_stages(trace,'ready',frames)
  for index in(0,1,2,4,5,6,7,8):
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
  for old,new in [('ticket=3','ticket=2'),('sequence=13','sequence=09')]:
   changed=trace.replace(old,new);wrong=copy.deepcopy(frames)
   # Equal-width replacements keep cut points; rehashes cannot fix provenance.
   for shot in wrong:shot['serial_sha256']=hashlib.sha256(changed[:shot['serial_end']].encode()).hexdigest()
   with self.assertRaises((AssertionError,ValueError)):gate.validate_frame_stages(changed,'ready',wrong)

def changed_stages(trace,frames,change):
 result='';out=[];previous=0
 for shot in frames:
  chunk=change(trace[previous:shot['serial_end']]);previous=shot['serial_end'];result+=chunk;row=dict(shot);row.update(serial_end=len(result),serial_sha256=hashlib.sha256(result.encode()).hexdigest());out.append(row)
 return result,out

class RetainedPresentation(unittest.TestCase):
 def test_centered_actual_and_fit_use_shared_chrome_at_both_scales(self):
  for tile in [dict(w=512,h=219),dict(w=1200,h=680),dict(w=192,h=310),dict(w=250,h=310)]:
   for mode in ('Fit','Actual'):
    g=gate.image_geometry(tile,mode);factor=2 if tile['w']>=256 else 1;top=44+g['chrome_rows']*9*factor
    self.assertEqual(g['x'],8+(tile['w']-16-48*g['scale'])//2);self.assertEqual(g['y'],top+(tile['h']-top-8-48*g['scale'])//2)
    self.assertEqual(g['scale'],1 if mode=='Actual'else min((tile['w']-16)//48,(tile['h']-top-8)//48,4))
    frame=gate.asset_text(tile,mode);self.assertLessEqual(frame['bytes'],512);self.assertEqual(frame['chrome_rows'],4 if g['cols']>=30 else 5)
    controls=' '.join(frame['lines'][3:frame['chrome_rows']]);self.assertIn('F Fit  A 1:1',controls);self.assertIn('G fetch  Q clear',controls)
 def test_each_mode_change_requires_new_generation_even_when_box_is_identical(self):
  trace,frames=staged_frames();a=gate.geometry(trace[:frames[0]['serial_end']]);b=gate.geometry(trace[:frames[1]['serial_end']]);self.assertEqual([a[k]for k in ('x','y','scale')],[b[k]for k in ('x','y','scale')]);self.assertNotEqual(a['generation'],b['generation']);self.assertNotEqual(a['sequence'],b['sequence'])
 def test_missing_foreign_stale_duplicate_ready_and_text_are_rejected(self):
  trace,frames=staged_frames()
  mutations=[lambda s:s.replace('[browser.asset.presentation.ready]','[absent.ready]'),lambda s:re.sub(r'(\[browser.asset.presentation.ready\] generation=)5',r'\g<1>4',s),lambda s:re.sub(r'(\[browser.asset.presentation.ready\][^\n]*viewport=)7',r'\g<1>99',s),lambda s:s.replace('[browser.asset.presentation.text]','[absent.text]'),lambda s:s+''.join(line for line in s.splitlines(True)if line.startswith('[browser.asset.presentation.ready]'))]
  for mutation in mutations:
   changed,shots=changed_stages(trace,frames,mutation)
   with self.assertRaises(ValueError):gate.validate_frame_stages(changed,'ready',shots)
 def test_missing_wrong_extra_and_misordered_control_key_fail(self):
  trace,frames=staged_frames()
  mutations=[lambda s:s.replace('code=30 down=1','code=31 down=1'),lambda s:s.replace('mode=Actual viewport=7 ticket=2 network=0','mode=Fit viewport=7 ticket=2 network=0'),lambda s:s.replace('[shell.browser.input] code=33 down=1 slot=43 accepted=1','[shell.browser.input] code=33 down=1 slot=43 accepted=1\n[shell.browser.input] code=33 down=1 slot=43 accepted=1')]
  for mutation in mutations:
   changed,shots=changed_stages(trace,frames,mutation)
   with self.assertRaises(ValueError):gate.validate_frame_stages(changed,'ready',shots)
  def reorder(s):
   keys=[line for line in s.splitlines(True)if line.startswith('[shell.browser.input] code=30 ')]
   for key in keys:s=s.replace(key,'')+key
   return s
  changed,shots=changed_stages(trace,frames,reorder)
  gate.validate_frame_stages(changed,'ready',shots) # App/Ready may precede the caller's post-call receipt.
 def test_current_generation_text_ack_must_precede_bitmap(self):
  trace=image_trace();line=next(line for line in trace.splitlines(True)if line.startswith('[browser.asset.presentation.text]'));trace=trace.replace(line,'')+line
  with self.assertRaises(ValueError):gate.geometry(trace)
 def test_full_chrome_help_and_metric_glyphs_are_mandatory(self):
  trace=image_trace(1200,680,y=60);image=ppm(trace);g=gate.geometry(trace);frame=gate.asset_text(g['tile'],g['mode']);offset=image.index(b'\n255\n')+5
  for row in (0,1,2,3):
   bad=bytearray(image);y=g['tile']['y']+36+row*9*frame['scale']
   for py in range(y,y+9*frame['scale']):
    start=offset+(py*1280+g['tile']['x']+8)*3;bad[start:start+frame['cols']*6*frame['scale']*3]=bytes(gate.BASE)*(frame['cols']*6*frame['scale'])
   with self.assertRaisesRegex(ValueError,'glyph'):gate.verify_pixels(trace,bytes(bad))
 def test_old_fit_rectangle_outside_actual_must_erase_every_pixel(self):
  old=image_trace(1200,680,y=60);trace=old+'[shell.browser.input] code=30 down=1 slot=43 accepted=1\n[browser.asset.presentation.intent] generation=5 mode=Actual viewport=7 ticket=2 network=0\n'+''.join(image_trace(1200,680,y=60,mode='Actual',generation=5,paint=10,source=False).splitlines(True)[2:]);image=ppm(trace);self.assertTrue(gate.verify_pixels(trace,image)['ok']);previous=ppm(old);g=gate.geometry(trace);width,_,before=gate.ppm_rgb(previous,min_width=1,min_height=1);offset=image.index(b'\n255\n')+5
  for i in range(0,len(before),3):
   x=(i//3)%width;y=(i//3)//width
   if tuple(before[i:i+3])not in (gate.BASE,(0xcd,0xd6,0xf4))and not(g['tile']['x']+g['x']<=x<g['tile']['x']+g['x']+48 and g['tile']['y']+g['y']<=y<g['tile']['y']+g['y']+48):
    bad=bytearray(image);bad[offset+i:offset+i+3]=before[i:i+3]
    with self.assertRaisesRegex(ValueError,'old rectangle'):gate.verify_pixels(trace,bytes(bad))
    break
  else:self.fail('negative control lacked old rectangle pixel')
 def test_retained_controls_do_not_add_fetches_or_source_cycles(self):
  trace,frames=staged_frames();self.assertEqual(trace.count('[browser.asset.intent]'),2);self.assertEqual(len(gate.frame_plan('ready')),9);gate.validate_frame_stages(trace,'ready',frames)
  extra=trace+'[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n'
  # Final screenshot cannot be reassigned to an extra (future) source intent.
  shots=copy.deepcopy(frames);shots[-1].update(serial_end=len(extra),serial_sha256=hashlib.sha256(extra.encode()).hexdigest())
  with self.assertRaises(ValueError):gate.validate_frame_stages(extra,'ready',shots)

def key_fixture(mode='ready',post=True,compact=False):
 trace='';actions=[];tick=1;menu_k=0
 if compact:
  for code in ('down','ret'):actions.append(dict(key=code,code=gate.KEY_CODES[code],kind='boot',transport='send-key',serial_start=0,serial_qmp_return=0,serial_end=0,host_start=tick,host_qmp_return=tick+.1,host_end=tick+.2));tick+=1
 keys=['g','a','f','i','q','g','grave_accent','k','k','ret','a','grave_accent','k','k','ret','f']if mode=='ready'else['g','q']if mode=='partial-clear'else['g','f11']if mode=='partial-close'else['g']
 for code in keys:
  start=len(trace);number=gate.KEY_CODES[code];trace+=f'[sexinput.ps2.scancode] raw={hex(number)}\n'
  if code in gate.BROWSER_EFFECTS:
   effects={'g':'[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n','a':'[browser.asset.presentation.intent] generation=5 mode=Actual viewport=7 ticket=2 network=0\n','f':'[browser.asset.presentation.intent] generation=6 mode=Fit viewport=7 ticket=2 network=0\n','i':'[browser.import.rejected] reason=asset_not_text effects=0\n','q':'[browser.asset.clear.intent] pending=1 owned_storage=erased\n'}
   held=f'[shell.browser.input] code={number} down=1 slot=43 accepted=1\n';trace+=effects[code]+held if post else held+effects[code]
  else:
   if code=='k':
    old,new=(0,9)if menu_k%2==0 else(9,8);trace+=f'[shell.palette.select] old={old} new={new}\n';menu_k+=1
   else:trace+=dict(grave_accent='[shell.palette.item] idx=0\n',k='[shell.palette.select] old=0 new=9\n',ret='[shell.window.action] action=ZoomToggle frame=9 sid=300 ok=1\n',f11='[sext.app.pool.ready] target=15 task=15 bytes=1 ok=1\n')[code]
  trace+=f'[sexinput.ps2.scancode] raw={hex(number|128)}\n';actions.append(dict(key=code,code=number,kind='browser'if code in gate.BROWSER_EFFECTS else'close'if code=='f11'else'menu',transport='input-send-event',serial_start=start,serial_qmp_return=start,serial_end=len(trace),host_start=tick,host_qmp_return=tick+.1,host_end=tick+.2));tick+=1
 return trace,[dict(serial_end=len(trace))],actions

def change_key_fixture(trace,actions,index,change):
 result='';rows=[];last=0
 for i,action in enumerate(actions):
  result+=trace[last:action['serial_start']];start=len(result);chunk=trace[action['serial_start']:action['serial_end']];result+=change(chunk)if i==index else chunk;row=dict(action);row.update(serial_start=start,serial_qmp_return=start,serial_end=len(result));rows.append(row);last=action['serial_end']
 result+=trace[last:];return result,[dict(serial_end=len(result))],rows

class PhysicalKeyCausality(unittest.TestCase):
 def test_app_before_or_after_shell_postcall_receipt_is_valid(self):
  for mode in gate.MODES:
   for post in (False,True):
    for compact in (False,True):
     trace,frames,actions=key_fixture(mode,post,compact);result=gate.verify_key_actions(trace,frames,actions,mode,'1024x768'if compact else'1280x800');self.assertTrue(result['ok']);self.assertEqual(result['checks'][0]['callee_before_caller_receipt'],post)
 def test_actual_hardware_helper_records_qmp_return_and_waits_for_both_receipts(self):
  order=[];actions=[]
  class Q:
   def cmd(self,op,args):order.append(('qmp',op,args))
  def wait(pattern,start):order.append(('wait',pattern,start));return 'receipt'
  with patch.object(gate.time,'sleep',side_effect=AssertionError('no arbitrary timer')):
   self.assertEqual(gate.capture_key_action(Q(),'g',gate.BROWSER_EFFECTS['g'],lambda:'prefix',wait,actions.append),'receipt')
  self.assertEqual(order[0][1],'input-send-event');self.assertEqual([event['data']['down']for event in order[0][2]['events']],[True,False]);self.assertIn('sexinput.ps2.scancode',order[1][1]);self.assertIn('browser.asset.intent',order[2][1]);self.assertIn('shell.browser.input',order[3][1]);self.assertEqual(actions[0]['serial_start'],6);self.assertLessEqual(actions[0]['host_start'],actions[0]['host_qmp_return']);self.assertIn('serial_end',actions[0])
 def test_missing_duplicate_foreign_or_reordered_physical_make_fails(self):
  trace,_,actions=key_fixture('invalid-qoi')
  mutations=[lambda s:s.replace('[sexinput.ps2.scancode] raw=0x22','[absent.scancode] raw=0x22'),lambda s:s.replace('raw=0x22','raw=0x21'),lambda s:s.replace('[sexinput.ps2.scancode] raw=0x22\n','[sexinput.ps2.scancode] raw=0x22\n'*2)]
  def late(s):
   physical='[sexinput.ps2.scancode] raw=0x22\n';return s.replace(physical,'')+physical
  for change in mutations+[late]:
   changed,frames,rows=change_key_fixture(trace,actions,0,change)
   with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,rows,'invalid-qoi','1280x800')
 def test_missing_duplicate_foreign_and_out_of_window_held_return_fails(self):
  trace,_,actions=key_fixture('invalid-qoi');held='[shell.browser.input] code=34 down=1 slot=43 accepted=1\n'
  for change in (lambda s:s.replace(held,''),lambda s:s.replace(held,held*2),lambda s:s.replace('code=34 down=1','code=33 down=1'),lambda s:held+s.replace(held,'')):
   changed,frames,rows=change_key_fixture(trace,actions,0,change)
   with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,rows,'invalid-qoi','1280x800')
  changed,frames,rows=change_key_fixture(trace,actions,0,lambda s:s.replace(held,''));changed+=held;frames[0]['serial_end']=len(changed)
  with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,rows,'invalid-qoi','1280x800')
 def test_handler_before_physical_or_qmp_start_and_wrong_focus_fail(self):
  trace,_,actions=key_fixture('invalid-qoi');event='[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n'
  changed,frames,rows=change_key_fixture(trace,actions,0,lambda s:event+s.replace(event,''))
  with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,rows,'invalid-qoi','1280x800')
  prefix=event;changed=prefix+trace;rows=copy.deepcopy(actions)
  for row in rows:
   for key in ('serial_start','serial_qmp_return','serial_end'):row[key]+=len(prefix)
  with self.assertRaises(ValueError):gate.verify_key_actions(changed,[dict(serial_end=len(changed))],rows,'invalid-qoi','1280x800')
  for focus in ('focused=201','mod=1 focused=300'):
   marker='[silk-shell.key.recv] code=34 down=1 '+('mod=0 '+focus if focus.startswith('focused')else focus)+'\n'
   changed,frames,rows=change_key_fixture(trace,actions,0,lambda s:s.replace(event,marker+event))
   with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,rows,'invalid-qoi','1280x800')
 def test_missing_qmp_window_reversed_or_foreign_request_fails(self):
  trace,frames,actions=key_fixture('invalid-qoi')
  for mutation in (lambda a:a.pop('serial_qmp_return'),lambda a:a.update(host_qmp_return=0),lambda a:a.update(serial_start=a['serial_end']),lambda a:a.update(key='a'),lambda a:a.update(transport='synthetic')):
   rows=copy.deepcopy(actions);mutation(rows[0])
   with self.assertRaises(ValueError):gate.verify_key_actions(trace,frames,rows,'invalid-qoi','1280x800')
  with self.assertRaises(ValueError):gate.verify_key_actions(trace,frames,[],'invalid-qoi','1280x800')
 def test_extra_physical_and_handler_events_cannot_hide_outside_windows(self):
  trace,frames,actions=key_fixture('invalid-qoi');tail='[sexinput.ps2.scancode] raw=0x21\n';changed=trace+tail;frames[0]['serial_end']=len(changed)
  with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,actions,'invalid-qoi','1280x800')
  changed=trace+'[browser.asset.intent] accepted=1 explicit=1 public_internet=0\n';frames[0]['serial_end']=len(changed)
  with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,actions,'invalid-qoi','1280x800')
 def test_menu_and_mode_physical_order_remains_finite_and_exact(self):
  trace,frames,actions=key_fixture();rows=copy.deepcopy(actions);rows[7],rows[8]=rows[8],rows[7]
  with self.assertRaises(ValueError):gate.verify_key_actions(trace,frames,rows,'ready','1280x800')
  changed,frames,rows=change_key_fixture(trace,actions,6,lambda s:s.replace('raw=0x29','raw=0x25'))
  with self.assertRaises(ValueError):gate.verify_key_actions(changed,frames,rows,'ready','1280x800')

class PointerParking(unittest.TestCase):
 def test_real_helper_uses_alternating_safe_intents_and_receipt_barriers_no_sleep(self):
  for resolution in ((1280,800),(1024,768)):
   order=[]
   class Q:
    def cmd(self,name,args):order.append(('cmd',name,args))
   def wait(pattern,start):order.append(('wait',pattern,start))
   with patch.object(gate.time,'sleep',side_effect=AssertionError('no timing substitute')):
    for phase in (0,1):gate.pointer_park(Q(),resolution,lambda:'prefix',wait,lambda r:order.append(('intent',r)),phase)
   self.assertEqual([r[0]for r in order],['intent','cmd','wait']*2);self.assertEqual((order[0][1]['x'],order[0][1]['y']),(resolution[0]-20,10));self.assertEqual(order[3][1]['x'],resolution[0]-21)
   self.assertIn('shell.abs.normalize',order[2][1]);self.assertIn('raw_y=',order[5][1]);self.assertNotEqual(order[1][2],order[4][2])
 def test_replay_requires_fresh_matching_normalization_for_each_capture(self):
  for resolution in ((1280,800),(1024,768)):
   trace='';frames=[];actions=[]
   for phase in range(2):
    start=len(trace);x=resolution[0]-20-phase%2;y=10;rx=round(x*32767/(resolution[0]-1));ry=round(y*32767/(resolution[1]-1));sx=rx*(resolution[0]-1)//32767;sy=ry*(resolution[1]-1)//32767
    trace+=f'[shell.abs.normalize] raw_x={rx} raw_y={ry} sx={sx} sy={sy} accepted=1 reason=ok\n';name='image-'+str(phase);actions.append(dict(kind='park',x=x,y=y,capture=name,serial_start=start,serial_end=len(trace)));trace+='frame barrier\n';frames.append(dict(name=name,serial_end=len(trace)))
   self.assertEqual(gate.verify_parking(trace,frames,actions,resolution)['captures'],2)
   for bad in (actions[:-1],[dict(a,kind='click')for a in actions],[dict(a,x=resolution[0]//2)for a in actions],[dict(a,serial_start=frames[0]['serial_end'])for a in actions]):
    with self.assertRaises(ValueError):gate.verify_parking(trace,frames,bad,resolution)
   for changed in (trace.replace('[shell.abs.normalize]','[absent.normalize]'),trace.replace('accepted=1 reason=ok','accepted=0 reason=edge_before_ready'),trace.replace('raw_y='+str(ry),'raw_y=0')):
    with self.assertRaises(ValueError):gate.verify_parking(changed,frames,actions,resolution)
 def test_later_foreign_pointer_sample_cannot_hide_behind_valid_park(self):
  resolution=(1280,800);x=1260;rx=round(x*32767/1279);ry=round(10*32767/799);trace=f'[shell.abs.normalize] raw_x={rx} raw_y={ry} sx={rx*1279//32767} sy={ry*799//32767} accepted=1 reason=ok\n';end=len(trace);trace+='[shell.abs.normalize] raw_x=16384 raw_y=16384 sx=639 sy=399 accepted=1 reason=ok\n'
  with self.assertRaises(ValueError):gate.verify_parking(trace,[dict(name='image',serial_end=len(trace))],[dict(kind='park',x=x,y=10,capture='image',serial_start=0,serial_end=end)],resolution)

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
    (out/'closed.ppm').write_bytes(image);manifest=dict(binding={},source_sha256={},keylog_enabled=False,frames=[frame],actions=[],key_actions=[],mode='partial-close',resolution='1280x800',command=command,host_utc_start=UTC-5,host_utc_end=UTC+5,raw_sha256={n:hashlib.sha256((out/n).read_bytes()).hexdigest()for n in('serial.log','wire.pcap','peer-exchanges.json','closed.ppm')});(out/'manifest.json').write_text(json.dumps(manifest))
    with patch.object(gate,'binding_check',return_value={}),patch.object(gate,'source_contract',return_value={'source_sha256':{}}),patch.object(gate,'verify_parking',return_value={'ok':True}),patch.object(gate,'verify_key_actions',return_value={'ok':True}):
     with self.assertRaises(ValueError):gate.verify_evidence(ROOT,iso,Path('unused'),out)

if __name__=='__main__':unittest.main()
