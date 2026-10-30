"""Independent public import fixtures; never actual native acceptance evidence."""
import copy,hashlib,re,struct,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import https_import_gate as gate
import test_tls_exchange_gate as fixture
ROOT=Path(__file__).resolve().parents[2]

def peer_rows():return [dict(attempt=n,handshake_complete=True,sni=gate.https.tls.HOST,tls_version='TLSv1.3',cipher=gate.https.tls.CIPHER,request_hex=gate.https.REQUEST.hex(),response_hex=gate.response(n).hex(),application_bytes_received=94,extra_application_bytes=0,unwrap_completed=True)for n in(1,2)]
def capture(recovery=False,extra=False,plaintext=False):
 frames=[]
 def tcp(port,ticket,outgoing,incoming):
  guest=48000+ticket;isn=100+ticket*100;peer=200+ticket*100;frames.extend([fixture.packet(True,guest,port,isn,0,2),fixture.packet(False,port,guest,peer,isn+1,0x12),fixture.packet(True,guest,port,isn+1,peer+1,0x10),fixture.packet(True,guest,port,isn+1,peer+1,0x18,outgoing),fixture.packet(False,port,guest,peer+1,isn+1+len(outgoing),0x18,incoming),fixture.packet(True,guest,port,isn+1+len(outgoing),peer+1+len(incoming),0x11),fixture.packet(False,port,guest,peer+1+len(incoming),isn+2+len(outgoing),0x11)])
 tcp(46078,1,gate.https.tls.ECHO,gate.https.tls.ECHO)
 if not recovery:
  for n in(1,2):
   frames.extend([fixture.packet(True,48000+n,46079,0,0,0,gate.https.tls.dns_query(n),True),fixture.packet(False,46079,48000+n,0,0,0,gate.https.tls.dns_response(n),True)]);incoming=fixture.hello(False)+b''.join(fixture.record(23,b'S'*(len(gate.response(n)[i:i+1024])+17))for i in range(0,len(gate.response(n)),1024));outgoing=fixture.hello()+fixture.record(23,b'C'*(94+128))
   if plaintext and n==2:outgoing+=gate.https.REQUEST
   tcp(46080,n+1,outgoing,incoming)
 if extra:tcp(46080,4,fixture.hello(),fixture.hello(False))
 raw=b'\xd4\xc3\xb2\xa1'+struct.pack('<HHIIII',2,4,0,0,65535,1)
 for n,frame in enumerate(frames):raw+=struct.pack('<IIII',1,n,len(frame),len(frame))+frame
 return raw

def disk_object(disk,slot,name,payload,journal=True,generation=1):
 manifest=2046*512;disk[manifest:manifest+8]=b'SDISKMV1';struct.pack_into('<HHI',disk,manifest+8,4,15,1);entry=manifest+16+slot*32;size=65536 if journal else len(payload);data=bytearray(32);data[:len(name)]=name;struct.pack_into('<I',data,24,size);struct.pack_into('<H',data,30,1);h=0x811c9dc5
 for b in data[:28]+data[30:32]:h=((h^b)*0x01000193)&0xffffffff
 struct.pack_into('<H',data,28,(h^(h>>16))&65535);disk[entry:entry+32]=data;start=2200+slot*128;descriptor=(1925-slot)*512;disk[descriptor:descriptor+4]=b'SFEX';struct.pack_into('<H',disk,descriptor+6,1);struct.pack_into('<HH',disk,descriptor+12,start,128 if journal else (len(payload)+511)//512)
 if journal:
  header=bytearray(512);header[:4]=b'QJ02';header[4]=slot;struct.pack_into('<I',header,8,((slot+1)<<16)|1);struct.pack_into('<Q',header,16,generation);struct.pack_into('<I',header,24,len(payload));struct.pack_into('<Q',header,32,gate.fnv(payload));struct.pack_into('<Q',header,504,gate.fnv(header[:504]));disk[start*512:(start+1)*512]=header;disk[(start+1)*512:(start+1)*512+len(payload)]=payload
 else:disk[start*512:start*512+len(payload)]=payload
 return disk

def disks():
 before=disk_object(bytearray(4096*512),2,b'old-note',b'pre-existing content');after=disk_object(bytearray(before),5,b'web-note',gate.BODY);return bytes(before),bytes(after)

def control_trace():
 text='';spec=[(9,0,5),(1,5,(6<<16)|1),(6,5,0),(8,5,12288),(5,5,0),(9,0,10)]
 for index,(kind,obj,value)in enumerate(spec,1):
  seq=index if index<4 else index+768;text+=f'[browser.import.control] phase=Admitted kind={kind} sequence={seq} object={obj} ok=1 value=0\n[browser.import.control] phase=Reply kind={kind} sequence={seq} object={obj} ok={int(index!=6)} value={value}\n'
 return text

class ImportWire(unittest.TestCase):
 def test_two_distinct_real_endpoint_application_observations(self):
  result=gate.inspect_wire(capture(),peer_rows());self.assertTrue(result['ok'],result);self.assertEqual([r['ticket']for r in result['joins']],[2,3]);self.assertNotEqual(result['joins'][0]['body_hash'],result['joins'][1]['body_hash']);self.assertEqual(len(gate.BODY),12288)
 def test_wrong_peer_cipher_application_and_missing_close_notify_fail(self):
  for field,value in(('sni','foreign.test'),('cipher','wrong'),('request_hex','00'),('response_hex',gate.response(1)[:-1].hex()),('unwrap_completed',False),('handshake_complete',False),('extra_application_bytes',1)):
   rows=peer_rows();rows[0][field]=value;self.assertFalse(gate.inspect_wire(capture(),rows)['ok'],field)
 def test_missing_extra_reordered_flows_and_plaintext_fail(self):
  self.assertFalse(gate.inspect_wire(capture(extra=True),peer_rows())['ok']);self.assertFalse(gate.inspect_wire(capture(plaintext=True),peer_rows())['ok']);self.assertFalse(gate.inspect_wire(capture(),list(reversed(peer_rows())))['ok']);self.assertFalse(gate.inspect_wire(capture(),peer_rows()[:1])['ok'])
 def test_recovery_allows_only_actual_startup_echo(self):
  self.assertTrue(gate.inspect_wire(capture(recovery=True),[],True)['ok']);self.assertFalse(gate.inspect_wire(capture(),peer_rows(),True)['ok'])

class ImportControls(unittest.TestCase):
 def test_exact_admitted_control_replies_commit_end_and_collision(self):
  rows=gate.controls(control_trace(),5,(6<<16)|1);self.assertEqual([r['kind']for r in rows],[9,1,6,8,5,9]);self.assertEqual(rows[3]['value'],12288);self.assertEqual(rows[-1]['ok'],0)
 def test_unadmitted_duplicate_foreign_sequence_object_or_error_fail(self):
  text=control_trace();line=text.splitlines(True)[0]
  for changed in(text.replace(line,'',1),text+line,text.replace('sequence=2 object=5','sequence=1 object=5'),text.replace('object=5 ok=1 value=0','object=6 ok=1 value=0',1),text.replace('ok=0 value=10','ok=1 value=10'),text.replace('kind=8','kind=7')):
   with self.assertRaises(AssertionError):gate.controls(changed,5,(6<<16)|1)
 def test_commit_wrong_length_end_nonzero_and_early_end_cannot_claim_saved(self):
  text=control_trace()
  for changed in(text.replace('value=12288','value=12287'),text.replace('kind=5 sequence=773 object=5 ok=1 value=0','kind=5 sequence=773 object=5 ok=1 value=1'),text.replace('phase=Admitted kind=8','phase=Reply kind=8')):
   with self.assertRaises(AssertionError):gate.controls(changed,5,(6<<16)|1)

class ImportDisk(unittest.TestCase):
 def test_fresh_create_is_exact_and_preexisting_bytes_remain_unchanged(self):
  before,after=disks();r=gate.import_disk(before,after);self.assertEqual(r['object'],5);self.assertEqual(r['bytes'],12288);self.assertEqual(r['hash'],hex(gate.fnv(gate.BODY)))
 def test_name_collision_checks_entire_object_and_manifest_not_just_body(self):
  _,disk=disks();self.assertTrue(gate.collision_disk(disk,disk,5)['unchanged']);changed=bytearray(disk);changed[(2200+5*128)*512+600]^=1
  with self.assertRaises(AssertionError):gate.collision_disk(disk,bytes(changed),5)
  changed=bytearray(disk);changed[2046*512+12]^=1
  with self.assertRaises(AssertionError):gate.collision_disk(disk,bytes(changed),5)
 def test_wrong_body_preexisting_overwrite_or_duplicate_name_fail(self):
  before,after=disks()
  for changed in(disk_object(bytearray(after),5,b'web-note',gate.COLLISION_BODY),disk_object(bytearray(after),2,b'old-note',b'changed'),disk_object(bytearray(after),6,b'web-note',gate.BODY)):
   with self.assertRaises(AssertionError):gate.import_disk(before,bytes(changed))
 def test_manifest_checksum_and_identity_corruption_fail(self):
  before,after=disks();changed=bytearray(after);changed[2046*512+16+5*32+28]^=1
  with self.assertRaises(AssertionError):gate.import_disk(before,bytes(changed))
 def test_raw_saved_extents_reconstruct_real_recovery(self):
  _,disk=disks();evidence=gate.raw_document(disk,5);sparse=bytearray(evidence['image_bytes'])
  for segment in evidence['segments']:
   data=bytes.fromhex(segment['hex']);self.assertEqual(hashlib.sha256(data).hexdigest(),segment['sha256']);sparse[segment['offset']:segment['offset']+len(data)]=data
  self.assertEqual(gate.recover(bytes(sparse),5)['payload'],gate.BODY)

UTC=fixture.UTC
CONTRACT={'pipe_bytes':8248}
def metadata(recovery=False):
 before,after=disks();meta=gate.import_disk(before,after);identity=meta['identity'];raw=capture(recovery);wire=gate.inspect_wire(raw,[]if recovery else peer_rows(),recovery);flows={f['guest_port']-48000:f for f in gate.https.tls.reassemble(raw)[0]};text=''
 if not recovery:
  text='\n'.join(fixture.metadata(fixture.capture()).splitlines()[:7])+'\n'
  for index,flow in enumerate(wire['joins'],1):
   ticket=index+1;digest=hex(gate.fnv(gate.BODIES[index-1]));initial=flow['initial_tls_bytes'];utc=UTC+index
   text+=f'[browser.command.started] kind=TlsDns accepted=1\n[browser.https.intent] accepted=1 explicit=1 public_internet=0\n[trust.policy.snapshot] owner=15 task=15 seq={index} profile=1 root_bytes=376 origin=held_boot_grant\n[clock.source.snapshot] pd=13 task=13 slot=61 seconds={utc} attempts=1 precision=1 profile=q35_cmos_utc ordinal={index}\n[certificate.time.snapshot] owner=15 task=15 seq={index} seconds={utc} source=q35_cmos_utc precision=1 receipt=retained\n[entropy.seed.admitted] owner=15 task=15 ticket={index} bytes=32 source=held_boot_grant\n[entropy.seed.progress] owner=Some(Identity {{ pd: 15, task: 15 }}) ticket={index} state=Some(Ready) produced=32 consumed=0 failure=0 private=1\n[entropy.seed.released] owner=15 task=15 ticket={index} service_payload=erased receipt=retained\n[browser.trust.self_check] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0\n[browser.tls.prepared] task_bytes=25000 private_bytes=45536 read=4096 write=2048 tx_bytes={initial} utc_snapshot={utc} authenticated=0 network=0\n[browser.https.prepared] task_bytes=25000 private_bytes=45536 tx_bytes={initial} utc_snapshot={utc} request_bytes=94 document_limit=12288 network=0\n[browser.dns.released] ticket={index} view=Answer([10, 0, 2, 2]) private_job=0\n[net.job.resolved.admitted] owner=15 task=15 ticket={ticket} dns_generation={index} observed=100 expires=60000 now=101 fixed=1 at_begin=1\n[net.job.duplex.admitted] owner=15 task=15 ticket={ticket} kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route\n[browser.https.admitted] ticket={ticket} dns_generation={index} seq={index*100} task_bytes=25000 private_bytes=45536 profile=1 authority=held_route\n'
   syn=flows[ticket]['syn'];text+=f'[net.job.egress.submit] ticket={ticket} sequence={index*10} length={syn["length"]} hash={syn["hash"]} scope=dma_admission_not_delivery\n[net.job.stream.drain] owner=15 task=15 ticket={ticket} produced=512 consumed=4 buffered=508 ring=1024 state_before=Receiving receipt=retained not_delivery=1\n[browser.https.authenticated] ticket={ticket} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1\n[browser.https.body] ticket={ticket} bytes=12288 hash={digest} parsed=1 authenticated=1 visible=0\n[browser.tls.cleanup] ticket={ticket} outcome=Verified authenticated=1 document=1 tls_payload=12288 positive_release=1\n[browser.https.released] ticket={ticket} tx={flow["outgoing_bytes"]} rx={flow["incoming_bytes"]} request_bytes=94 body_bytes=12288 body_hash={digest} positive_release=1 authenticated=1 public_test_root=1\n[browser.import.intent] source_ticket={ticket} bytes=12288 hash={digest} authenticated=1 positive_release=1 create_only=1 accepted=1\n[browser.import.submit] name=web-note bytes=12288 hash={digest} source_ticket={ticket} accepted=1 create_only=1\n'
   lines=control_trace().splitlines(True)
   if index==1:
    for pair in range(5):
     text+=lines[pair*2]
     if pair==0:text+=f'[files.browser.import.created] target=15 object=5 identity={identity} create_only=1\n'
     if pair==3:text+=f'[files.document.durable] caller=15 object=5 identity={identity} generation=1 bank=0 bytes=12288 hash={digest} barriers=3 readback=1\n'
     text+=lines[pair*2+1]
     if pair==2:text+='[browser.command.blocked] kind=TlsDns import_pending=1 accepted_replay=0\n'
    text+=f'[browser.import.event] state=Saved object=5 identity={identity} bytes=12288 hash={digest} error=0 source_ticket=2\n[linen.workspace.changed] target=15 task=15 refresh=queued read_only=1\n[browser.import.files] target=15 task=15 accepted=1\n[shell.document.open] target=15 linen=200\n[linen.quil.disk_doc.intent] object_id=1005 path_id=5 ok=1\n[quil.persist.load.ok] bytes=12288 hash={digest} doc=5\n'
   else:text+=''.join(lines[10:])+f'[browser.import.event] state=Exists object=0 identity=0 bytes=12288 hash={digest} error=10 source_ticket=3\n'
 else:text+=f'[browser.import.files] target=15 task=15 accepted=1\n[shell.document.open] target=15 linen=200\n[linen.quil.disk_doc.intent] object_id=1005 path_id=5 ok=1\n[quil.persist.load.ok] bytes=12288 hash={hex(gate.fnv(gate.BODY))} doc=5\n'
 text+='[browser.import.closed] target=15 task=15 pending=0 accepted=0 terminal=1 owned_storage=erased\n[files.document.retire] target=15 staged=0 pending=0 terminal=1\n'
 for role in(1,2,4,8,16):text+=f'[app.service.ack] target=15 task=15 holder=13 role={role} ok=1\n'
 text+='[net.dns.records.retired] target=15 task=15 remaining=0 before_cleanup_ack=1\n[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1\n[sext.app.pool.ready] target=15 task=15 bytes=516096 ok=1\n';dns=gate.dns_capture(raw,'echo',expected=[dict(ticket=n,response_hex=gate.https.tls.dns_response(n).hex())for n in(1,2)])if not recovery else{'ok':True};return text,raw,wire,dns,meta

class FullImportJoins(unittest.TestCase):
 def check(self,text=None,recovery=False):
  original,raw,wire,dns,meta=metadata(recovery);return gate.inspect_import(original if text is None else text,wire,dns,raw,meta,CONTRACT,UTC-5,UTC+5,recovery)
 def test_combined_import_collision_and_cold_reopen_controls(self):
  for recovery in(False,True):self.assertTrue(self.check(recovery=recovery)['ok'],self.check(recovery=recovery))
 def test_missing_control_commit_end_or_early_saved_fail(self):
  text,*_=metadata()
  for a,b in(('[browser.import.control] phase=Reply kind=8','[missing.commit] phase=Reply kind=8'),('phase=Reply kind=5','phase=Admitted kind=5'),('bytes=12288 hash=','bytes=12287 hash='),('[files.document.durable]','[missing.durable]'),('[files.browser.import.created]','[missing.create]')):self.assertFalse(self.check(text.replace(a,b))['ok'],a)
  saved=next(l for l in text.splitlines(True)if l.startswith('[browser.import.event] state=Saved'));self.assertFalse(self.check(saved+text.replace(saved,''))['ok'])
 def test_fresh_sources_actual_positive_release_and_no_accepted_replay(self):
  text,*_=metadata()
  for a,b in(('seq=2 profile=1','seq=1 profile=1'),('origin=held_boot_grant','origin=ambient'),('service_payload=erased','service_payload=retained'),('[browser.tls.cleanup]','[missing.cleanup]'),('[browser.command.blocked]','[missing.blocked]'),('accepted_replay=0','accepted_replay=1')):self.assertFalse(self.check(text.replace(a,b))['ok'],a)
 def test_foreign_import_object_identity_hash_collision_and_load_fail(self):
  text,*_=metadata()
  for a,b in(('state=Saved object=5','state=Saved object=6'),('identity=393217','identity=393218'),('error=10 source_ticket=3','error=0 source_ticket=3'),('path_id=5','path_id=6'),('doc=5','doc=6'),('phase=Reply kind=9 sequence=774 object=0 ok=0 value=10','phase=Reply kind=9 sequence=774 object=0 ok=1 value=5')):self.assertFalse(self.check(text.replace(a,b))['ok'],a)
 def test_all_shared_drains_are_checked_and_terminal_custody_is_required(self):
  text,*_=metadata();line=next(l for l in text.splitlines(True)if l.startswith('[net.job.stream.drain]'))
  for changed in(text+line,text.replace(line,line+line),text.replace('ticket=2 produced=512','ticket=99 produced=512'),text.replace('buffered=508','buffered=507'),text.replace('role=16','role=99'),text.replace('[files.document.retire]','[missing.file.retire]')):self.assertFalse(self.check(changed)['ok'])
 def test_cold_reopen_may_not_refetch_reimport_or_rewrite(self):
  text,*_=metadata(True)
  for extra in('[browser.https.intent] accepted=1\n','[browser.import.control] phase=Admitted\n','[files.document.durable] caller=9 object=5\n','[files.document.durable] caller=15 object=5\n'):self.assertFalse(self.check(text+extra,True)['ok'])

class ImportFramePixels(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=gate.source_contract(ROOT)
 def paint(self,state='Naming',name='web-note',compact=False):
  width,height=(1024,768)if compact else(1280,800);tile=dict(x=0,y=50,w=width,h=height-50);f=gate.import_frame(tile,state,name,self.contract);raw=bytearray(width*height*3);scale=f['scale'];color=self.contract['color'].to_bytes(3,'big')
  for row,line in enumerate(f['lines']):
   for column,byte in enumerate(line):
    for gy,bits in enumerate(gate.https.glyph(self.contract['font'],chr(byte))):
     for gx in range(5):
      if bits&(1<<(4-gx)):
       for dy in range(scale):
        for dx in range(scale):
         x=tile['x']+self.contract['inset']+(column*6+gx)*scale+dx;y=tile['y']+self.contract['top']+(row*9+gy)*scale+dy;at=(y*width+x)*3;raw[at:at+3]=color
  code=gate.STATE_CODES[state];text=f'[shell.document.viewport] target=15 task=15 surface=300 sequence=7 x=0 y=50 w={width} h={height-50} enqueued=1\n[browser.viewport] target=15 task=15 sequence=7 w={width} h={height-50} origin=held_shell\n[browser.import.layout] state={state} name={name} bytes={f["bytes"]} cols={f["cols"]} rows={f["rows"]} viewport=7 hash={f["hash"]} pixels=unconfirmed\n[browser.import.applied] bytes={f["bytes"]} hash={f["hash"]} rows={f["rows"]} viewport=7 state_code={code} sequence=12 positive_ack=1\n[browser.view.applied] bytes={f["bytes"]} http={1024|(code<<11)} body_bytes=0 pixels=unconfirmed\n';return text,f'P6\n{width} {height}\n255\n'.encode()+raw,(width,height)
 def test_prompt_saved_collision_and_edited_name_have_full_glyphs(self):
  for compact in(False,True):
   for state,name in(('Naming','web-note'),('Naming','web-not'),('Saved','web-note'),('Exists','web-note')):
    text,image,resolution=self.paint(state,name,compact);result=gate.verify_import_pixels(text,image,state,name,self.contract,resolution);self.assertTrue(result['ok'],result);self.assertGreater(result['checked_pixels'],0)
 def test_actual_runner_end_navigation_reaches_full_unique_tail(self):
  for width,height in((1280,750),(1024,718)):
   tile=dict(x=0,y=50,w=width,h=height);anchor=gate.reader_end_anchor(tile,self.contract);frame=gate.https.http.reader_frame(gate.BODY,width,height,self.contract,anchor,b'/welcome');self.assertGreater(anchor,0);self.assertEqual(frame['end'],frame['total_lines']);self.assertIn(gate.https.TAIL,b''.join(frame['lines'][2:-1]));self.assertEqual(anchor,gate.https.http.reader_move(frame,'end'))
 def test_missing_ack_wrong_hash_state_viewport_and_stale_frame_fail(self):
  text,image,resolution=self.paint()
  for a,b in(('[browser.import.applied]','[missing.applied]'),('positive_ack=1','positive_ack=0'),('state_code=0','state_code=2'),('viewport=7 state_code','viewport=8 state_code'),('sequence=12 positive','sequence=0 positive'),('hash=0x','hash=0x1')):self.assertFalse(gate.verify_import_pixels(text.replace(a,b),image,'Naming','web-note',self.contract,resolution)['ok'],a)
  self.assertFalse(gate.verify_import_pixels(text,image,'Saved','web-note',self.contract,resolution)['ok']);self.assertFalse(gate.verify_import_pixels(text,image,'Naming','other',self.contract,resolution)['ok'])
 def test_one_glyph_or_padding_pixel_changed_fails(self):
  text,image,resolution=self.paint();changed=bytearray(image);header=image.index(b'255\n')+4;at=header+((50+self.contract['top'])*resolution[0]+self.contract['inset']+13)*3;color=self.contract['color'].to_bytes(3,'big');changed[at:at+3]=b'\0\0\0'if changed[at:at+3]==color else color;self.assertFalse(gate.verify_import_pixels(text,bytes(changed),'Naming','web-note',self.contract,resolution)['ok'])
 def test_short_tile_keeps_status_and_name_without_clipping(self):
  frame=gate.import_frame(dict(x=0,y=50,w=512,h=120),'Naming','web-note',self.contract);text=b'\n'.join(frame['lines']);self.assertIn(b'New document name',text);self.assertIn(b'web-note',text);self.assertLessEqual(frame['bytes'],512)

if __name__=='__main__':unittest.main()
