"""Public parser/pixel/disk fixtures; never native responsiveness evidence."""
import copy,re,struct,sys,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import tls_responsiveness_gate as gate
import test_https_document_gate as fixture
from test_tls_lifecycle_gate import snapshot
ROOT=Path(__file__).resolve().parents[2];UTC=fixture.fixture.UTC

def source_trace():
 text='';prepared=[]
 for index in range(1,4):
  kind='TlsDns'if index==3 else'TlsPrepare';doc=int(index==3);task=25000 if doc else 12000;private=task+8248+12288*doc;utc=UTC+index
  text+=f'[browser.command.queued] kind={kind} document={doc} retained=1 accepted_replay=0\n'+snapshot('command',queued='Some('+kind+')')+f'[browser.command.started] kind={kind} accepted=1\n'
  text+='[browser.https.intent] accepted=1 explicit=1 public_internet=0\n'if doc else'[browser.tls.intent] accepted=1 explicit=1 network=0\n'
  text+=f'[trust.policy.snapshot] owner=15 task=15 seq={index} profile=1 root_bytes=376 origin=held_boot_grant\n[clock.source.snapshot] pd=13 task=13 slot=61 seconds={utc} attempts=1 precision=1 profile=q35_cmos_utc ordinal={index}\n[certificate.time.snapshot] owner=15 task=15 seq={index} seconds={utc} source=q35_cmos_utc precision=1 receipt=retained\n[entropy.seed.admitted] owner=15 task=15 ticket={index} bytes=32 source=held_boot_grant\n[entropy.seed.progress] owner=Some(Identity {{ pd: 15, task: 15 }}) ticket={index} state=Some(Ready) produced=32 consumed=0 failure=0 private=1\n[entropy.seed.released] owner=15 task=15 ticket={index} service_payload=erased receipt=retained\n'
  for phase,name in enumerate(('FixtureCheck','ClientHello')):
   sequence=index*10+phase;turn=index*20+phase;begin=index*100+phase*5;payload=0 if phase==0 else task+8192+12288*doc
   text+=f'[browser.tls.work] kind={name} sequence={sequence} app_turn={turn} begin_ticks={begin} end_ticks={begin+3} elapsed_ticks=3 clock_valid=1 payload={payload} seed_ready={int(phase==0)}\n'
   if phase==0:text+='[browser.trust.self_check] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0\n'
  offset=len(text);text+=f'[browser.tls.prepared] task_bytes={task} private_bytes={private} read=4096 write=2048 tx_bytes=159 utc_snapshot={utc} authenticated=0 network=0\n';prepared.append(dict(document=bool(doc),task_bytes=task,offset=offset,started_offset=0,seed_release_offset=0))
  if doc:text+=f'[browser.https.prepared] task_bytes={task} private_bytes={private} tx_bytes=159 utc_snapshot={utc} request_bytes=94 document_limit=12288 network=0\n'
  if not doc:text+=f'[browser.tls.view] state=HelloPending authenticated=0 network=0\n[browser.tls.erased] bytes={task+8192} owned_storage=1\n[browser.tls.view] state=Cancelled authenticated=0 network=0\n'+snapshot('clear')
 return text,prepared

def metadata():
 base=fixture.metadata('partial-close');text='\n'.join(base.splitlines()[:7])+'\n'+source_trace()[0]+base[base.index('[browser.dns.released]'):]
 text+='[shell.browser.input] code=38 down=1 slot=43 accepted=1\n'*2+'[shell.browser.input] code=16 down=1 slot=43 accepted=1\n'*2+'[shell.browser.input] code=48 down=1 slot=43 accepted=1\n'
 text=text.replace('[browser.https.progress] ticket=2', '[net.job.stream.drain] owner=15 task=15 ticket=2 produced=512 consumed=4 buffered=508 ring=1024 state_before=Receiving receipt=retained not_delivery=1\n[browser.https.progress] ticket=2')
 first=gate.TOKEN+b'init';second=gate.TOKEN+gate.AFTER+b'init';save=lambda p:f'[quil.document.edit] revision={len(p)} pending=0 bytes={len(p)}\n[quil.persist.save.ok] bytes={len(p)} hash={hex(gate.fnv(p))} doc=2\n'
 text=text.replace('[browser.https.closed]',save(first)+'[browser.https.closed]');closed='[browser.https.closed] pending=1 terminal=1 owned_storage=erased body_visible=0\n';text=text.replace(closed,closed+snapshot('close'));return text+save(second)

def clock_fixture(contract,width=1280,stamp=(10,42,7)):
 d=contract['clock']['dimensions'];x=d['CHIP_X3']-max(0,d['CHIP_X3']+d['CLOCK_W']+16-width);y=d['CHIP_Y'];height=800 if width==1280 else 768;raw=bytearray(width*height*3);color=contract['clock']['color'].to_bytes(3,'big')
 def put(xx,yy):at=((y+1+yy)*width+x+xx)*3;raw[at:at+3]=color
 for col,ch in enumerate('DEMO '):
  for yy,bits in enumerate(gate.glyph(contract['font'],ch)):
   for xx in range(5):
    if bits&(1<<(4-xx)):put(col*6+xx,yy)
 digits=[int(c)for part in stamp for c in f'{part:02d}']
 for offset,digit in zip((0,7,17,24,34,41),digits):
  for yy,bits in enumerate(contract['clock']['digits'][digit]):
   for xx in range(5):
    if bits&(1<<(4-xx)):put(30+offset+xx,yy)
 for offset in(44,61):
  for yy in(1,5):put(offset,yy)
 trace=f'[sexdisplay.clock.presentation] label=DEMO wall_time=0 x={x} y={y} w=80 h=22 digits=30 width={width}\n';return trace,f'P6\n{width} {height}\n255\n'.encode()+raw,(width,height)

def document_disk(payload):
 disk=bytearray(4096*512);slot=2;identity=(3<<16)|1;manifest=2046*512;entry=manifest+16+slot*32;disk[entry]=1;struct.pack_into('<I',disk,entry+24,65536);struct.pack_into('<H',disk,entry+30,1);descriptor=(1925-slot)*512;disk[descriptor:descriptor+4]=b'SFEX';struct.pack_into('<H',disk,descriptor+6,1);struct.pack_into('<HH',disk,descriptor+12,2200,128);header=bytearray(512);header[:4]=b'QJ02';header[4]=2;struct.pack_into('<I',header,8,identity);struct.pack_into('<Q',header,16,1);struct.pack_into('<I',header,24,len(payload));struct.pack_into('<Q',header,32,gate.fnv(payload));struct.pack_into('<Q',header,504,gate.fnv(header[:504]));disk[2200*512:2201*512]=header;disk[2201*512:2201*512+len(payload)]=payload;return bytes(disk)

class HeavyWorkReceipts(unittest.TestCase):
 def test_three_preparations_have_six_actual_distinct_turns(self):
  text,prepared=source_trace();rows=gate.work_receipts(text,prepared);self.assertEqual(len(rows),6);self.assertEqual([r['kind']for r in rows],['FixtureCheck','ClientHello']*3)
 def test_missing_duplicate_unknown_and_malformed_work_fail(self):
  text,prepared=source_trace();line=next(line for line in text.splitlines(True)if line.startswith('[browser.tls.work]'))
  for changed in(text.replace(line,'',1),text+line,text.replace('kind=FixtureCheck','kind=Other',1),text.replace('clock_valid=1','clock_valid=2',1)):
   with self.assertRaises(AssertionError):gate.work_receipts(changed,prepared)
 def test_same_probe_or_app_turn_cannot_claim_separation(self):
  text,prepared=source_trace()
  for a,b in(('sequence=11','sequence=10'),('app_turn=21','app_turn=20')):
   with self.assertRaises(AssertionError):gate.work_receipts(text.replace(a,b),prepared)
 def test_invalid_clock_and_false_elapsed_fail(self):
  text,prepared=source_trace()
  for a,b in(('clock_valid=1','clock_valid=0'),('end_ticks=103','end_ticks=99'),('elapsed_ticks=3','elapsed_ticks=2')):
   with self.assertRaises(AssertionError):gate.work_receipts(text.replace(a,b,1),prepared)
 def test_fixture_seed_and_client_allocation_must_match_real_phase(self):
  text,prepared=source_trace()
  for a,b in(('payload=0 seed_ready=1','payload=1 seed_ready=1'),('payload=0 seed_ready=1','payload=0 seed_ready=0'),('payload=20192 seed_ready=0','payload=20191 seed_ready=0'),('payload=20192 seed_ready=0','payload=20192 seed_ready=1')):
   with self.assertRaises(AssertionError):gate.work_receipts(text.replace(a,b,1),prepared)
 def test_fresh_sources_positive_seed_cleanup_and_host_clock(self):
  from clock_source_evidence import BROWSER_GRANT
  text,_=source_trace();text=BROWSER_GRANT+'\n'+text;rows=gate.source_cycles(text,{'pipe_bytes':8248},UTC,UTC+5);self.assertEqual([r['document']for r in rows],[False,False,True])
  for a,b in(('seq=2 profile=1','seq=1 profile=1'),('service_payload=erased','service_payload=retained'),('ordinal=2','ordinal=1'),('private_bytes=33248','private_bytes=33247')):
   changed=text.replace(a,b,1)
   if changed==text:continue
   with self.assertRaises(AssertionError):gate.source_cycles(changed,{'pipe_bytes':8248},UTC,UTC+5)
  with self.assertRaises(AssertionError):gate.source_cycles(text,{'pipe_bytes':8248},UTC+100,UTC+110)

class ProgressAndCustody(unittest.TestCase):
 def check(self,text):
  raw=fixture.capture('partial-close');wire=gate.https.inspect_wire(raw,fixture.peer('partial-close'),'partial-close');dns=gate.dns_capture(raw,'echo',expected=[dict(ticket=1,response_hex=gate.https.tls.dns_response(1).hex())]);return gate.inspect_progress(text,wire,dns,raw,UTC-5,UTC+5,{'pipe_bytes':8248,'quil_initial':b'init'})
 def test_full_synthetic_join_is_control_only(self):self.assertTrue(self.check(metadata())['ok'],self.check(metadata()))
 def test_no_peer_proof_no_quil_save_or_partial_publication_pass(self):
  text=metadata()
  for a,b in(('[browser.https.authenticated]','[missing.auth]'),('body_received=1024','body_received=1023'),('body_visible=0','body_visible=1'),('[quil.persist.save.ok]','[missing.save]'),('pending=1 terminal=1','pending=0 terminal=1')):self.assertFalse(self.check(text.replace(a,b))['ok'],a)
  self.assertFalse(self.check(text+'[browser.reader.layout] tls=1\n')['ok'])
 def test_foreign_boot_duplex_drain_and_cleanup_fail(self):
  text=metadata()
  for a,b in(('consumer_task=15 profile=1','consumer_task=99 profile=1'),('kind=10 ring=1024','kind=9 ring=1024'),('owner=15 task=15 ticket=2 produced','owner=15 task=99 ticket=2 produced'),('ring=1024 state_before','ring=2048 state_before'),('buffered=508','buffered=507'),('role=16','role=99')):self.assertFalse(self.check(text.replace(a,b))['ok'],a)
 def test_quil_during_and_after_close_order_is_mandatory(self):
  text=metadata();lines=text.splitlines(True);saves=[line for line in lines if line.startswith('[quil.persist.save.ok]')];self.assertFalse(self.check(text.replace(saves[0],'')+saves[0])['ok']);self.assertFalse(self.check(saves[1]+text.replace(saves[1],''))['ok'])
 def test_all_shared_read_samples_validate_without_filtering(self):
  text=metadata();line=next(line for line in text.splitlines(True)if line.startswith('[net.job.stream.drain]'))
  for extra in(line,line.replace('ticket=2','ticket=99'),line.replace('consumed=4','consumed=3'),line.replace('produced=512','produced=99999')):self.assertFalse(self.check(text.replace(line,line+extra))['ok'])
  self.assertFalse(self.check(text.replace(line,'')+line)['ok'])
 def test_actual_input_preparation_edit_and_private_storage_cannot_be_omitted(self):
  text=metadata()
  for a,b in(('[shell.browser.input] code=48','[missing.input] code=48'),('[browser.https.prepared]','[missing.doc.prepared]'),('document_limit=12288','document_limit=16384'),('[quil.document.edit]','[missing.edit]'),('reason=close heap_used=0 heap_free=65536','reason=close heap_used=1 heap_free=65535')):self.assertFalse(self.check(text.replace(a,b))['ok'],a)
 def test_independent_disk_recovery_checks_exact_payload_and_hash(self):
  payload=b'finite public document';disk=document_disk(payload);self.assertEqual(gate.durable_snapshot(disk,payload)['bytes'],len(payload))
  with self.assertRaises(AssertionError):gate.durable_snapshot(disk,payload+b'z')
  changed=bytearray(disk);changed[2201*512]^=1
  with self.assertRaises(ValueError):gate.durable_snapshot(bytes(changed),payload)
 def test_raw_extent_evidence_reconstructs_both_actual_journal_banks(self):
  payload=b'first actual saved snapshot';disk=document_disk(payload);evidence=gate.document_evidence(disk);sparse=bytearray(evidence['image_bytes'])
  for segment in evidence['segments']:
   raw=bytes.fromhex(segment['hex']);self.assertEqual(len(raw),segment['bytes']);self.assertEqual(gate.hashlib.sha256(raw).hexdigest(),segment['sha256']);sparse[segment['offset']:segment['offset']+len(raw)]=raw
  self.assertEqual(gate.durable_snapshot(bytes(sparse),payload),gate.durable_snapshot(disk,payload));self.assertTrue(any(s['bytes']>=65536 for s in evidence['segments']))

class QuilFrameRetry(unittest.TestCase):
 def test_incomplete_admission_is_failed_frame_without_geometry_escape(self):
  with patch.object(gate,'quil_pixels',return_value=['latest Quil paint not fully admitted']),patch.object(gate,'quil_view',side_effect=ValueError('not fully admitted'))as geometry:
   check=gate.verify_quil_frame('immutable trace',b'pixels','token',{'font':[]},(1280,800));self.assertFalse(check['ok']);geometry.assert_not_called()
 def test_geometry_transient_is_retryable_but_never_accepted(self):
  with patch.object(gate,'quil_pixels',return_value=[]),patch.object(gate,'quil_view',side_effect=ValueError('latest Quil paint not fully admitted')):
   result=gate.verify_quil_frame('snapshot',b'pixels','token',{'font':[]},(1280,800));self.assertEqual(result,{'ok':False,'errors':['latest Quil paint not fully admitted']})
 def test_complete_glyphs_and_geometry_use_exact_same_snapshot(self):
  snapshot='one immutable serial prefix'
  with patch.object(gate,'quil_pixels',return_value=[])as pixels,patch.object(gate,'quil_view',return_value={'paint':7})as geometry:
   result=gate.verify_quil_frame(snapshot,b'pixels','token',{'font':[]},(1280,800));self.assertTrue(result['ok']);self.assertIs(pixels.call_args.args[0],geometry.call_args.args[0]);self.assertEqual(result['geometry'],{'paint':7})

class DemoClockPixels(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=gate.source_contract(ROOT)
 def test_source_binds_work_clock_quil_and_shared_layout(self):
  for name in('apps/kaleidoscope/src/tls_probe.rs','apps/kaleidoscope/src/main.rs','servers/sexdisplay/src/main.rs','servers/quil/src/main.rs','crates/silkbar-model/src/lib.rs','scripts/tls_responsiveness_gate.py'):self.assertIn(name,self.contract['source_sha256'])
 def test_full_demo_and_live_digits_wide_and_compact(self):
  for width in(1280,1024):
   trace,image,resolution=clock_fixture(self.contract,width);result=gate.verify_demo_clock(trace,image,self.contract,resolution);self.assertTrue(result['ok'],result);self.assertEqual(result['software_counter'],'10:42:07');self.assertEqual(result['geometry']['x'],1090 if width==1280 else 928);self.assertEqual(result['checked_pixels'],532)
 def test_missing_prefix_stale_digits_and_spacing_fail(self):
  trace,image,resolution=clock_fixture(self.contract);header=image.index(b'255\n')+4;changed=bytearray(image);x=1090;y=19
  for yy in range(7):
   for xx in range(30):at=header+((y+yy)*resolution[0]+x+xx)*3;changed[at:at+3]=b'\0\0\0'
  self.assertFalse(gate.verify_demo_clock(trace,bytes(changed),self.contract,resolution)['ok']);changed=bytearray(image);at=header+(y*resolution[0]+x+35)*3;changed[at:at+3]=self.contract['clock']['color'].to_bytes(3,'big');self.assertFalse(gate.verify_demo_clock(trace,bytes(changed),self.contract,resolution)['ok'])
 def test_wrong_identity_geometry_and_resolution_fail(self):
  trace,image,resolution=clock_fixture(self.contract,1024)
  for changed in(trace.replace('label=DEMO','label=UTC'),trace.replace('x=928','x=1090'),trace.replace('wall_time=0','wall_time=1'),trace+trace):self.assertFalse(gate.verify_demo_clock(changed,image,self.contract,resolution)['ok'])
  self.assertFalse(gate.verify_demo_clock(trace,image,self.contract,(1280,800))['ok'])
 def panel(self,width=1024):
  d=self.contract['clock']['dimensions'];x=d['CHIP_X3']-max(0,d['CHIP_X3']+d['CLOCK_W']+16-width);height=768 if width==1024 else 800;px=min(1000,width-240)
  trace=f'[shell.silkbar.click] target=clock x={x+40} y=29\n[shell.panel.geometry] surface=148 x={px} y=55 w=240 h=300 desktop_w={width} desktop_h={height} submitted=1\n[shell.clock.open.ok] id=0x94\n[shell.silkbar.click] target=clock x={x+40} y=29\n[shell.clock.close.ok] id=0x94\n[browser.command.started] kind=TlsPrepare accepted=1\n';return trace,(width,height)
 def test_actual_clock_hit_test_and_submitted_panel_fit_both_desktops(self):
  for width in(1024,1280):
   text,resolution=self.panel(width);result=gate.clock_panel(text,self.contract,resolution);self.assertEqual(result['submitted_geometry']['surface'],148);self.assertEqual(len(result['clicks']),2);self.assertIn('unconfirmed',result['pixels'])
 def test_wrong_click_clipped_panel_or_missing_close_cannot_pass(self):
  text,resolution=self.panel()
  for a,b in(('x=968 y=29','x=900 y=29'),('x=784 y=55','x=1000 y=55'),('surface=148','surface=149'),('submitted=1','submitted=0'),('[shell.clock.close.ok]','[missing.close]')):
   with self.assertRaises(AssertionError):gate.clock_panel(text.replace(a,b),self.contract,resolution)
 def test_invalid_counter_cannot_qualify_as_demo_progress(self):
  trace,image,resolution=clock_fixture(self.contract,stamp=(10,99,7));self.assertFalse(gate.verify_demo_clock(trace,image,self.contract,resolution)['ok'])

if __name__=='__main__':unittest.main()
