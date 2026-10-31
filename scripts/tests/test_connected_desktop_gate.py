"""Adversarial connected-session evidence controls. No VM/build execution."""
import copy,gzip,hashlib,json,struct,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import connected_desktop_gate as gate
ROOT=Path(__file__).resolve().parents[2]

def history_disk(commands=('old note',),generation=3,bank=0):
 disk=bytearray(2*1024*1024);data=b''.join(len(s.encode()).to_bytes(2,'little')+s.encode()for s in commands);payload=bytearray(24+len(data));payload[:4]=b'SHI1';struct.pack_into('<IHH',payload,4,12,len(commands),len(payload));payload[24:]=data;struct.pack_into('<Q',payload,16,gate.fnv(payload));header=bytearray(512);header[:8]=b'SEXHST01';struct.pack_into('<IIQIII',header,8,12,1,generation,12,12,len(payload));struct.pack_into('<Q',header,40,gate.fnv(payload));struct.pack_into('<Q',header,48,gate.fnv(header));at=(1808+9*bank)*512;disk[at:at+512]=header;disk[at+512:at+512+len(payload)]=payload;return bytes(disk)

def history_trace(meta,cold=False):
 h=meta['fingerprint'];n=meta['size'];count=meta['count'];generation=meta['generation'];prefix=''
 if cold:prefix=f'[files.history.recovered] logical=12 source=12 source_task=12 target=12 generation={generation} bytes={n} source_hash={h} owned_hash={h} data_hash={meta["data_hash"]} boot_grant=53\n'
 result=prefix+f'[files.history.open] target=12 handle=7 mode={"load"if cold else"save"} seq=1\n'
 if not cold:result+=f'[files.history.durable] logical=12 target=12 generation={generation} bank={meta["bank"]} bytes={n} hash={h} barriers=3 readback=1\n[files.history.commit] target=12 handle=7 seq=2 bytes={n} hash={h}\n'
 result+=f'[files.history.close] target=12 handle=7 seq=3 ok=1\n[spindle.history.receipt] target=12 task=12 kind={"loaded"if cold else"saved"} handle=7 bytes={n} count={count} hash={h} error=0\n';return result

def key_fixture(keys=('ctrl','a')):
 trace='before\n';start=len(trace)
 for key in keys:trace+=f'[sexinput.ps2.scancode] raw={hex(gate.KEY_CODES[key])}\n'
 for key in reversed(keys):trace+=f'[sexinput.ps2.scancode] raw={hex(gate.KEY_CODES[key]|0x80)}\n'
 row=dict(kind='key',keys=list(keys),serial_start=start,serial_qmp_return=start,serial_end=len(trace),host_start=1,host_qmp_return=2,host_end=3,transport='input-send-event',events=[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for down,ks in((True,keys),(False,tuple(reversed(keys))))for k in ks]);return trace,row

class ConnectedChecker(unittest.TestCase):
 def test_actual_source_closure_includes_text_image_wheel_history_and_own_runner(self):
  c=gate.source_contract(ROOT)
  for name in('apps/kaleidoscope/src/main.rs','servers/sexinput/src/main.rs','apps/spindle/src/history_persistence.rs','servers/sexfiles/src/history_journal.rs','crates/sex-pdx/src/clipboard_client.rs','scripts/connected_desktop_gate.py','scripts/tests/test_connected_desktop_gate.py'):
   self.assertEqual(c['source_sha256'][name],gate.sha((ROOT/name).read_bytes()))
  self.assertEqual(len(gate.BODY),2048);self.assertEqual(len(gate.FINAL),2049)
 def test_history_raw_exact_commands_and_only_empty_seed_is_optional(self):
  data=history_disk();meta=gate.history_state(data);self.assertEqual(meta['commands'],['old note']);self.assertEqual(meta['generation'],3)
  self.assertEqual(gate.history_state(bytes(len(data)),False)['commands'],[])
  for index in(1808*512,1808*512+48,1808*512+512):
   bad=bytearray(data);bad[index]^=1
   with self.assertRaises(RuntimeError):gate.history_state(bytes(bad),False)
 def test_ambiguous_committed_history_is_never_empty(self):
  a=history_disk(('left',));b=history_disk(('right',),bank=1);bad=a[:(1808+9)*512]+b[(1808+9)*512:]
  with self.assertRaisesRegex(RuntimeError,'ambiguous'):gate.history_state(bad,False)
 def test_history_append_respects_actual_ring_and_payload_bounds(self):
  self.assertEqual(gate.appended_history(['a']),['a',gate.COMMAND]);many=['x'*256]*128;actual=gate.appended_history(many)
  self.assertEqual(actual[-1],gate.COMMAND);self.assertLessEqual(len(actual),128);self.assertLessEqual(24+sum(2+len(x)for x in actual),4096);self.assertGreater(24+sum(2+len(x)for x in ['x'*256]+actual),4096)
 def test_cold_readonly_checks_inactive_bank_header_and_padding(self):
  disk=history_disk();self.assertTrue(gate.no_cold_history_write(disk,disk))
  for at in((1808+9)*512,(1808+9)*512+511,1808*512+512+400):
   bad=bytearray(disk);bad[at]^=1
   with self.assertRaisesRegex(ValueError,'bank/header/padding'):gate.no_cold_history_write(disk,bytes(bad))
 def test_history_positive_durable_commit_close_and_fresh_cold_lease(self):
  meta=gate.history_state(history_disk())
  self.assertEqual(gate.history_join(history_trace(meta),meta)['handle'],7)
  # Numeric lease IDs may restart at boot; cold freshness is new open/close.
  self.assertEqual(gate.history_join(history_trace(meta,True),meta,True)['handle'],7)
 def test_history_missing_foreign_duplicate_or_reordered_effects_fail(self):
  meta=gate.history_state(history_disk());trace=history_trace(meta);lines=trace.splitlines(True)
  cases=[trace.replace('target=12','target=17',1),trace+lines[-1],''.join(lines[:1]+lines[2:]),''.join([lines[1],lines[0]]+lines[2:]),trace.replace('barriers=3','barriers=2'),trace.replace('readback=1','readback=0'),trace.replace('seq=3','seq=1')]
  for bad in cases:
   with self.subTest(bad=bad),self.assertRaises(ValueError):gate.history_join(bad,meta)
 def test_command_before_async_history_not_reverse(self):
  meta=gate.history_state(history_disk());command='[spindle.instance.command] target=12 task=12 name=echo ok=1\n';trace=history_trace(meta)
  self.assertEqual(gate.command_before_history(command+trace,meta)['handle'],7)
  with self.assertRaisesRegex(ValueError,'preceded command'):gate.command_before_history(trace+command,meta)
 def test_real_hardware_chord_and_physical_windows(self):
  trace,row=key_fixture();self.assertEqual(gate.verify_keys(trace,[row])['physical_makes'],2)
  for changed in(trace.replace('raw=0x1e','raw=0x30'),trace.replace('[sexinput.ps2.scancode] raw=0x1e\n',''),trace+'[sexinput.ps2.scancode] raw=0x1e\n'):
   with self.assertRaises(ValueError):gate.verify_keys(changed,[row])
 def test_fake_transport_wrong_qmp_events_and_window_alias_fail(self):
  trace,row=key_fixture()
  for key,value in(('transport','app-opcode'),('events',[]),('serial_start',row['serial_end']),('serial_end',len(trace)+1),('host_qmp_return',4)):
   bad=copy.deepcopy(row);bad[key]=value
   with self.subTest(key=key),self.assertRaises(ValueError):gate.verify_keys(trace,[bad])
 def test_full_alpha_policy_remains_separate_and_unchanged(self):
  self.assertIn('not full alpha acceptance',gate.SCOPE)
  from check_alpha_session import session_errors
  self.assertTrue(session_errors({'ok':True,'soak':{'seconds':900,'cycles':1}}))
 def test_fault_and_deferred_drop_rejected_in_replay(self):
  for fault in('[fault.kill] task=15','unexpected user fault','[sexfiles.defer.drop] seq=8','serial.record.overflow','KERNEL PAGE FAULT'):
   with self.subTest(fault=fault),self.assertRaises(ValueError):gate.reject_faults('normal\n'+fault+'\n')
  gate.reject_faults('[normal] all=1\n')
 def test_frame_name_cannot_substitute_a_different_app_or_mode(self):
  frames=[dict(name=n,kind='quil',screenshot=n+'.ppm',mode=None)for n in gate.COLD_FRAMES];frames[1]['kind']='spindle';self.assertTrue(gate.frame_contract(frames,True))
  for change in('kind','name','screenshot'):
   bad=copy.deepcopy(frames);bad[0][change]='image'
   with self.assertRaises(ValueError):gate.frame_contract(bad,True)
 def test_crlf_unicode_character_prefix_and_raw_hash_are_distinct(self):
  with tempfile.TemporaryDirectory()as d:
   p=Path(d)/'serial.log';raw='é € 🙂\r\n[actual] event=1\r\n'.encode();p.write_bytes(raw);trace=gate.http.serial_text(p);frame=dict(serial_end=len(trace),serial_sha256=gate.sha(trace.encode()),requested_anchor=None)
   self.assertEqual(gate.http.frame_prefix(trace,frame),trace);self.assertNotEqual(gate.sha(raw),frame['serial_sha256'])
   with self.assertRaises(ValueError):gate.http.frame_prefix(raw.decode(),frame)
 def test_cli_check_needs_no_source_disk_and_does_not_launch(self):
  import contextlib,io
  with tempfile.TemporaryDirectory()as d:
   root=Path(d);iso=root/'tested.iso';binding=root/'source-binding.json';iso.write_bytes(b'fixture');binding.write_text('{}')
   with patch.object(sys,'argv',['connected_desktop_gate.py','--iso',str(iso),'--binding',str(binding),'--out',d,'--check']),patch.object(gate,'replay',return_value={'ok':True,'full_alpha_acceptance':False})as replay,patch.object(gate.subprocess,'Popen',side_effect=AssertionError('no VM')),contextlib.redirect_stdout(io.StringIO()):gate.main()
   self.assertEqual(replay.call_count,1)

 def test_reopened_frame_cannot_reuse_saved_prefix(self):
  doc={'object':5};saved=f'[quil.persist.save.ok] bytes={len(gate.FINAL)} hash={hex(gate.fnv(gate.FINAL))} doc=5\n[quil.document.saved] saved_revision=8 revision=8 dirty=0\n'
  self.assertTrue(gate.frame_semantic(saved,{'name':'quil-saved'},doc))
  with self.assertRaises(ValueError):gate.frame_semantic(saved,{'name':'quil-reopened'},doc)
  loaded=saved+f'[quil.persist.load.ok] bytes={len(gate.FINAL)} hash={hex(gate.fnv(gate.FINAL))} doc=5\n'
  self.assertTrue(gate.frame_semantic(loaded,{'name':'quil-reopened'},doc))
  with self.assertRaises(ValueError):gate.frame_semantic(loaded.replace('doc=5','doc=6'),{'name':'quil-cold'},doc)
 def overlay_fixture(self):
  return f'[browser.import.layout] state=NoDocument name= bytes=479 cols=47 rows=10 viewport=1 hash=0x123 pixels=unconfirmed\n[browser.clipboard.refused] ticket=3 reason=reader_hidden effects=0\n[browser.clipboard.stopped] ticket=3 state=Unsupported payload_erased=1\n[browser.import.dismissed] source=http anchor=0 bytes={len(gate.BODY)} restored=1 network=0 selection_preserved=1\n'
 def test_hidden_reader_copy_is_refused_and_positive_cancel_precedes_return(self):
  trace=self.overlay_fixture();self.assertEqual(gate.overlay_join(trace)['refused_ticket'],3)
  for old,new in(('state=Unsupported','state=Cancelled'),('ticket=3 state','ticket=4 state'),('selection_preserved=1','selection_preserved=0'),('bytes=2048','bytes=2049'),('source=http','source=https')):
   with self.subTest(old=old),self.assertRaises(ValueError):gate.overlay_join(trace.replace(old,new))
  for tag in('browser.clipboard.refused','browser.clipboard.stopped','browser.import.dismissed'):
   bad=''.join(line for line in trace.splitlines(True)if not line.startswith('['+tag+']'))
   with self.assertRaises(ValueError):gate.overlay_join(bad)
 def test_hidden_reader_cannot_publish_scroll_or_refetch(self):
  trace=self.overlay_fixture();at=trace.index('[browser.import.dismissed]')
  for event in('[browser.clipboard.copy] accepted=1','[browser.clipboard.published] positive_ack=1','[browser.reader.wheel] changed=1','[browser.http.document] ticket=4','[net.dns.egress.submit] ticket=3'):
   with self.subTest(event=event),self.assertRaises(ValueError):gate.overlay_join(trace[:at]+event+'\n'+trace[at:])
 def test_profile_cannot_relabel_model_resolution_or_tablet(self):
  iso=Path('/tmp/example.iso').resolve();command=gate.desktop.native_command(iso,Path('/tmp/disk'),Path('/tmp/serial'),Path('/tmp/sock'),Path('/tmp/pcap'),'e1000e','1280x800');m=dict(command=command,model='e1000e',resolution='1280x800');self.assertTrue(gate.command_profile(m,iso))
  for key,value in(('model','e1000'),('resolution','1024x768')):
   bad=dict(m);bad[key]=value
   with self.assertRaises(ValueError):gate.command_profile(bad,iso)
  bad=dict(m,command=[x for x in command if x!='usb-tablet,bus=xhci.0'])
  with self.assertRaises(ValueError):gate.command_profile(bad,iso)
 def test_empty_name_overlay_uses_actual_source_label_hint_and_padded_rows(self):
  c=gate.storage.source_contract(ROOT);tile=dict(x=640,y=425,w=640,h=375);viewport=7;trace='[shell.document.viewport] target=15 task=15 surface=300 sequence=7 x=640 y=425 w=640 h=375 enqueued=1\n[browser.viewport] target=15 task=15 sequence=7 w=640 h=375 origin=held_shell\n'
  # Independent source renderer reference uses a space name: one blank cell
  # remains indistinguishable from the required empty-name padded row.
  reference=gate.storage.import_frame(tile,'NoDocument','-',c);cols=reference['cols'];rows=reference['rows'];lines=[bytearray(b' '*cols)for _ in range(rows)];source=c['import_labels']['NoDocument']+b'\n\n'+c['import_hints'].get('NoDocument',c['import_default_hint']);r=col=0
  for byte in source:
   if byte==10:r+=1;col=0;continue
   if col==cols:r+=1;col=0
   if r>=rows:break
   lines[r][col]=byte;col+=1
  payload=b'\n'.join(lines);n=len(payload);h=hex(gate.fnv(payload));code=gate.storage.STATE_CODES['NoDocument'];trace+=f'[browser.import.layout] state=NoDocument name= bytes={n} cols={cols} rows={rows} viewport={viewport} hash={h} pixels=unconfirmed\n[browser.import.applied] bytes={n} hash={h} rows={rows} viewport={viewport} state_code={code} sequence=8 positive_ack=1\n[browser.view.applied] bytes={n} http={1024|(code<<11)} body_bytes=0 pixels=unconfirmed\n'
  actual=gate.empty_import_geometry(trace,c);self.assertEqual(actual['lines'],[bytes(line)for line in lines]);self.assertTrue(all(b==32 for b in actual['lines'][1]))
  for old,new in(('name= bytes','name=web-note bytes'),('positive_ack=1','positive_ack=0'),(h,'0x123')):
   with self.assertRaises(ValueError):gate.empty_import_geometry(trace.replace(old,new),c)
 def test_mixed_read_samples_keep_both_protocols_and_reject_foreign_or_regressed(self):
  trace='[net.job.duplex.admitted] owner=15 task=15 ticket=2 kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route\n[net.job.stream.admitted] owner=15 task=15 ticket=3 kind=9 ring=1024 max_wire=13312 total_ticks=30000 idle_ticks=5000 authority=held_route\n'
  for ticket in(2,3):trace+=f'[net.job.stream.drain] owner=15 task=15 ticket={ticket} produced=100 consumed=80 buffered=20 ring=1024 state_before=Receiving receipt=retained not_delivery=1\n'
  flows=[dict(guest_port=48002,peer_port=46080,incoming=b'x'*100),dict(guest_port=48003,peer_port=46078,incoming=b'x'*100)]
  with patch.object(gate.tls,'reassemble',return_value=(flows,1)):
   self.assertEqual([r['kind']for r in gate.mixed_drains(trace,b'fixture')['samples']],[10,9])
   for old,new in(('owner=15 task=15 ticket=3 produced','owner=9 task=9 ticket=3 produced'),('ticket=3 produced','ticket=4 produced'),('buffered=20','buffered=21'),('ring=1024 state','ring=512 state'),('produced=100','produced=101'),('kind=9','kind=10')):
    with self.subTest(old=old),self.assertRaises(ValueError):gate.mixed_drains(trace.replace(old,new),b'fixture')
   with self.assertRaises(ValueError):gate.mixed_drains(trace+trace.splitlines(True)[-1],b'fixture')
   with self.assertRaises(ValueError):gate.mixed_drains(trace+'[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1\n'+trace.splitlines(True)[-1],b'fixture')

 def test_no_prior_launcher_skips_erasure_but_partial_antecedent_rejects(self):
  c={'launcher_app_count':7,'launcher_count':10}
  with patch.object(gate.readability,'launcher_erased',return_value={'remaining':0})as erased:
   self.assertIsNone(gate.historical_launcher_erasure('[boot] normal=1\n',b'fixture',c,(1280,800)));self.assertEqual(erased.call_count,0)
   for trace in('[launcher.open] count=7 selected=0 ok=1\n','[shell.palette.item] idx=0\n','[launcher.text.view] x=400\n','[command_palette.close]\n','[launcher.open] count=oops\n'):
    with self.subTest(trace=trace),self.assertRaises(ValueError):gate.historical_launcher_erasure(trace,b'fixture',c,(1280,800))
   with self.assertRaisesRegex(ValueError,'post-Spindle'):gate.historical_launcher_erasure('',b'fixture',c,(1280,800),True)
   self.assertEqual(erased.call_count,0)
 def test_real_historical_launcher_requires_latest_layout_close_and_pixel_erasure(self):
  c={'launcher_app_count':7,'launcher_count':10};opened='[launcher.open] count=7 selected=0 ok=1\n[shell.palette.open] ok=1 selected=0 count=10\n';layout='[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=0 first=0 rows=1\n[launcher.text.row] index=0 row=0 name=Open Spindle status=READY\n';closed='[command_palette.close]\n';trace=opened+layout+closed
  with patch.object(gate.readability,'launcher_erased',return_value={'remaining':0})as erased:
   self.assertEqual(gate.historical_launcher_erasure(trace,b'fixture',c,(1280,800),True),{'remaining':0});self.assertEqual(erased.call_count,1)
   for bad in(opened+layout,trace+opened,opened+closed,trace.replace('name=Open Spindle status=READY','incomplete=1')):
    with self.subTest(bad=bad),self.assertRaises(ValueError):gate.historical_launcher_erasure(bad,b'fixture',c,(1280,800),True)
   self.assertEqual(erased.call_count,1)
  with patch.object(gate.readability,'launcher_erased',side_effect=ValueError('stale launcher ink')):
   with self.assertRaisesRegex(ValueError,'stale launcher ink'):gate.historical_launcher_erasure(trace,b'fixture',c,(1280,800),True)
 def test_live_workspace_returns_after_close12_before_close15(self):
  trace='[shell.scene.switch] from=0 to=1\n[close12.pool]\n[shell.scene.switch] from=1 to=0\n[shell.app.close.request] target=15 task=15 surface=300\n';cleanup=[{'pool_offset':trace.index('[close12.pool]')}]
  self.assertTrue(gate.scene_return_join(trace,cleanup));self.assertTrue(gate.scene_return_join('cold scene0',[],True))
  for bad in(trace.replace('[shell.scene.switch] from=1 to=0\n',''),trace+'[shell.scene.switch] from=0 to=1\n',trace.replace('from=1 to=0','from=1 to=2')):
   with self.assertRaises(ValueError):gate.scene_return_join(bad,cleanup)
  with self.assertRaisesRegex(ValueError,'follow Close12'):gate.scene_return_join(trace,[{'pool_offset':trace.index('[shell.app.close.request]')}])
  with self.assertRaises(ValueError):gate.scene_return_join(trace,cleanup,True)

if __name__=='__main__':unittest.main()
