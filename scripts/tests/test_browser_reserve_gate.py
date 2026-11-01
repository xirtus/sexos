"""Synthetic parser controls only; no guest/BOOT/native success is asserted."""
import copy
import contextlib
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'));sys.path.insert(0,str(Path(__file__).resolve().parent))
import browser_reserve_gate as p
import browser_source_evidence as e
from test_browser_source_evidence import inventory,clock,success

ROWS=[dict(target=x.target,surface=x.surface,lifecycle=x.lifecycle,entropy=x.entropy,clock=x.clock,trust=x.trust,ordinal=i,predecessor=(0,15,30)[i],activation=(None,156,160)[i])for i,x in enumerate(e.FUTURE_PLANS)]

def cleanup_lines(row):
 t,k,s=row['target'],row['task'],row['surface'];text=f'[shell.app.close.request] target={t} task={k} surface={s}\n[task.finish] pd={t} task={k} reason=0 terminal=1\n'
 mask=0
 for role,holder in[(1,3),(2,1),(4,11),(8,13),(16,10)]:
  if role==8:
   text+=f'[net.dns.records.retired] target={t} task={k} remaining=0 before_cleanup_ack=1\n'
   for tag,suffix in[('entropy.seed.retired','private_payload=erased'),('certificate.time.retired','high_water_retained=1'),('trust.policy.retired','')]:text+=f'[{tag}] target={t} task={k} before_cleanup_ack=1'+(' '+suffix if suffix else'')+'\n'
  mask|=role;text+=f'[app.service.ack] target={t} task={k} holder={holder} role={role} acked={mask} required=31\n'
 text+=f'[app.private.retired] target={t} task={k} pages=2 bytes=8192 acked=31 required=31 all_flushed=1 pool_reusable=1 messages=0 signals=0 metadata=retained runtime_alloc=0\n'
 return text+f'[sext.app.pool.ready] target={t} task={k} bytes=8192 zeroed=8192 workspace_records=0 reused=1 zero_owner=sext ok=1\n'

def fixture(mode='quiesced'):
 text=inventory(e.FUTURE_PLANS,True);at=text.index('[net.source.binding.held]')
 text=text[:at]+'[boot.app.catalog.complete] rows=3 actual_grants=1 runtime_mint=0\n[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0\n[bootgraph.boot.handoff] target=sexdisplay id=1 entry=0x4000\n'+text[at:]
 result={'evidence_version':2,'mode':mode,'chain':[]};tasks={15:150,30:300,31:310}
 for i,base in enumerate(ROWS):
  row=dict(base,task=tasks[base['target']]);t,k,s=row['target'],row['task'],row['surface'];start=0 if i==0 else len(text)
  if i:text+=f'[app.activation.committed] logical=15 incarnation={i+1} predecessor={row["predecessor"]} successor={t} task={k} surface={s} pages=2 runtime_alloc=0 runtime_mint=0 bsp=1\n[shell.app.relaunch] predecessor={row["predecessor"]} successor={t} task={k} surface={s} receipt_replayed=1\n'
  text+=f'[shell.app.registered.reply] target={t} task={k} surface={s} committed=1\n[app.surface.presented] target={t} surface={s} sequence=1 ok=1\n[shell.focus.set] id={s}\n'
  from check_tcp_capture import fingerprint
  text+=f'[net.job.state] owner=Some(Identity {{ pd: {t}, task: {k} }}) ticket={i+1} from=Connecting to=Sending staged=1025 queued=1025 received=0 scope=private_job\n[browser.net.job.bytes] ticket={i+1} length=1025 hash={hex(fingerprint(p.ECHO))} exact=1 endpoint=controlled_peer tls=0 web=0\n[browser.net.job.released] ticket={i+1} view=Verified private_job=0\n'
  begin=len(text);utc=e.MIN_SECONDS+100+i
  text+='[browser.resources] reason=command heap_used=0 heap_free=65536 heap_total=65536 tls_payload=0 tls_pending=0 route_busy=0 providers_busy=0 queued=Some(TlsPrepare) body_visible=0\n'
  text+=f'[browser.command.started] kind=TlsPrepare accepted=1\n[trust.policy.snapshot] owner={t} task={k} seq=1 profile=1 root_bytes=376 origin=held_boot_grant\n'
  text+=clock(13,130,row['clock'],utc,i+1)+success(t,k,1,utc)
  text+=f'[entropy.seed.admitted] owner={t} task={k} ticket={i+1} bytes=32 source=held_boot_grant\n'
  text+=f'[entropy.seed.progress] owner=Some(Identity {{ pd: {t}, task: {k} }}) ticket={i+1} state=Some(Ready) produced=32 consumed=0 failure=0 private=1\n'
  text+=f'[entropy.seed.released] owner={t} task={k} ticket={i+1} service_payload=erased receipt=retained\n'
  text+=f'[browser.tls.prepared] task_bytes=10000 private_bytes=18248 read=4096 write=2048 tx_bytes=100 utc_snapshot={utc} authenticated=0 network=0\n'
  if mode=='allocated-close':
   text+='[browser.tls.view] state=HelloPending authenticated=0 network=0\n[browser.resources] reason=change heap_used=18300 heap_free=47236 heap_total=65536 tls_payload=18192 tls_pending=1 route_busy=0 providers_busy=0 queued=None body_visible=0\n'
  close_start=len(text);terminal=cleanup_lines(row)
  if mode=='allocated-close':
   tag=f'[task.finish] pd={t} task={k} reason=0 terminal=1\n'
   effects='[browser.tls.erased] bytes=18192 owned_storage=1\n[browser.tls.closed] pending=1 terminal=1 owned_storage=erased\n'
   for name,suffix in[('trust',''),('clock',''),('entropy',' private_payload=erased')]:effects+=f'[browser.{name}.closed] target={t} task={k} pending=0 terminal=1{suffix}\n'
   effects+=f'[browser.clipboard.closed] target={t} task={k} owned_scratch=erased terminal=1\n[browser.resources] reason=close heap_used=0 heap_free=65536 heap_total=65536 tls_payload=0 tls_pending=0 route_busy=0 providers_busy=0 queued=None body_visible=0\n'
   terminal='[sexinput.ps2.scancode] raw=0x57\n'+terminal.replace(tag,effects+tag,1)+'[sexinput.ps2.scancode] raw=0xd7\n'
  text+=terminal;result['chain'].append(dict(identity=row,start=start,work_start=begin,close_start=close_start,end=len(text)))
 result['exhaustion_start']=len(text);text+='[launcher.text.row] index=10 row=4 name=Open Browser status=CLOSED / RESERVE EXHAUSTED\n[launcher.execute.denied] idx=10 status=CLOSED / RESERVE EXHAUSTED reason=boot_reserve_exhausted\n';result['exhaustion_end']=len(text)
 return text,result

def rewrite(trace,result,old,new):
 """Change raw fixtures while preserving correct independent phase cuts."""
 at=trace.index(old);stop=at+len(old);delta=len(new)-len(old);changed=copy.deepcopy(result)
 def move(value):return value if value<=at else value+delta if value>=stop else at+len(new)
 for step in changed['chain']:
  for key in('start','work_start','close_start','end'):step[key]=move(step[key])
 for key in('exhaustion_start','exhaustion_end'):changed[key]=move(changed[key])
 return trace[:at]+new+trace[stop:],changed

def timing_fixture(trace,result,proof):
 actions=[];result=copy.deepcopy(result)
 for index,(step,join)in enumerate(zip(result['chain'],proof['joins'])):
  identity=step['identity'];target,task,surface=(identity[k]for k in('target','task','surface'));start,end=step['close_start'],step['end'];request=join['close_evidence']['offsets']['request'];request_end=trace.index('\n',request);pool=join['cleanup']['pool_offset'];pool_end=trace.index('\n',pool);issued=(index+1)*1000
  key_index=len(actions);actions.append(dict(kind='key',key='f11',transport='input-send-event',start=start,end=end,start_sha256=p.sha(trace[:start].encode()),end_sha256=p.sha(trace[:end].encode()),receipt_pattern=rf'\[shell.app.close.request\] target={target} task={task} surface={surface}'+p.END,receipt_offset=request,receipt_end=request_end,receipt_serial_end=request_end+1,receipt_serial_sha256=p.sha(trace[:request_end+1].encode()),issued_ns=issued,sent_ns=issued+1,receipt_observed_ns=issued+2,completed_ns=issued+3,qmp=[dict(execute='input-send-event',arguments=dict(events=[dict(type='key',data=dict(down=down,key=dict(type='qcode',data='f11')))for down in(True,False)]))]))
  step['close_observation_index']=len(actions);actions.append(dict(kind='browser_close',mode='allocated-close',target=target,task=task,surface=surface,key_index=key_index,start=start,end=end,serial_sha256=p.sha(trace[:end].encode()),request_offset=request,pool_offset=pool,pool_end=pool_end,issued_ns=issued,request_observed_ns=issued+2,pool_observed_ns=issued+4,key_to_request_ns=2,key_to_pool_ns=4))
 return result,actions,dict(monotonic_clock=p.MONOTONIC_CLOCK,host_monotonic_start_ns=100,host_monotonic_end_ns=10000)

class BrowserReserve(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
  path=self.root/'crates/sex-pdx/src/app_catalog.rs';path.parent.mkdir(parents=True);path.write_text('pub const INCARNATIONS: [Incarnation; 3] = [];')
  for name in('crates/sex-tls/src/lib.rs','apps/kaleidoscope/src/tls_probe.rs'):
   path=self.root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes((ROOT/name).read_bytes())
  for target,value in[(p,'browser_rows'),(p.sources,'plans_from_catalog')]:
   replacement=ROWS if value=='browser_rows'else e.FUTURE_PLANS
   patcher=patch.object(target,value,return_value=replacement);patcher.start();self.addCleanup(patcher.stop)
 def verify(self,text,result,mode='quiesced'):return p.verify(text,result,self.root,mode)
 def test_complete_three_unequal_tasks_and_cumulative_cleanup(self):
  text,result=fixture();checked=self.verify(text,result)
  self.assertEqual(checked['targets'],[15,30,31]);self.assertEqual([s['identity']['task']for s in checked['joins']],[150,300,310]);self.assertFalse(checked['browser_pixels_qualified']);self.assertTrue(checked['reserve_exhausted'])
 def test_allocated_close_all_three_requires_actual_owned_payload_not_pipe_metadata(self):
  text,result=fixture('allocated-close');proof=self.verify(text,result,'allocated-close')
  self.assertEqual(proof['mode'],'allocated-close')
  for join in proof['joins']:
   evidence=join['close_evidence'];self.assertEqual((evidence['task_bytes'],evidence['private_bytes'],evidence['erased_payload_bytes']),(10000,18248,18192));self.assertEqual(evidence['live_resources']['pending'],1);self.assertEqual(evidence['zero_resources']['used'],0)
 def test_allocated_close_rejects_prior_q_expiry_reprepare_zero_and_wrong_erasure(self):
  for old,new in[('state=HelloPending authenticated=0 network=0','state=TimedOut authenticated=0 network=0'),('state=HelloPending authenticated=0 network=0','state=Cancelled authenticated=0 network=0'),('[browser.tls.erased] bytes=18192','[browser.tls.erased] bytes=0'),('[browser.tls.erased] bytes=18192','[browser.tls.erased] bytes=18248'),('[browser.tls.closed] pending=1','[browser.tls.closed] pending=0'),('tls_payload=18192 tls_pending=1','tls_payload=0 tls_pending=0'),('reason=change heap_used=18300','reason=clear heap_used=18300')]:
   text,result=fixture('allocated-close');bad,changed=rewrite(text,result,old,new)
   with self.subTest(old=old,new=new),self.assertRaises(ValueError):self.verify(bad,changed,'allocated-close')
  for event in['[sexinput.ps2.scancode] raw=0x10\n','[browser.tls.erased] bytes=18192 owned_storage=1\n','[browser.command.queued] kind=TlsPrepare document=0 retained=1 accepted_replay=0\n']:
   text,result=fixture('allocated-close');marker='[browser.resources] reason=change';bad,changed=rewrite(text,result,marker,event+marker)
   with self.subTest(event=event),self.assertRaises(ValueError):self.verify(bad,changed,'allocated-close')
 def test_allocated_close_rejects_stale_crossed_and_nonzero_terminal_resources(self):
  for old,new in[('reason=change heap_used=18300 heap_free=47236','reason=change heap_used=1 heap_free=65535'),('reason=close heap_used=0 heap_free=65536','reason=close heap_used=1 heap_free=65535'),('reason=close heap_used=0','reason=change heap_used=0'),('target=30 task=300 pending=0 terminal=1','target=30 task=30 pending=0 terminal=1'),('heap_total=65536 tls_payload=18192','heap_total=65536 target=31 task=310 tls_payload=18192')]:
   text,result=fixture('allocated-close');bad,changed=rewrite(text,result,old,new)
   with self.subTest(old=old),self.assertRaises(ValueError):self.verify(bad,changed,'allocated-close')
  text,result=fixture('allocated-close');live=next(line+'\n'for line in text.splitlines()if line.startswith('[browser.resources] reason=change'))
  bad,changed=rewrite(text,result,live,'');marker='[browser.tls.prepared]';bad,changed=rewrite(bad,changed,marker,live+marker)
  with self.assertRaisesRegex(ValueError,'stale/quiesced/not allocated'):self.verify(bad,changed,'allocated-close')
 def test_atomic_resource_receipt_can_interrupt_legacy_serial_text(self):
  text,result=fixture('allocated-close');bad,changed=rewrite(text,result,'[browser.resources] reason=close','[access.focus.describe] node_id=8195 role=7[browser.resources] reason=close')
  self.assertTrue(self.verify(bad,changed,'allocated-close')['ok'])
  for raw in ('[browser.resources] reason=close heap_used=0','[browser.resources] garbage\n'):
   with self.subTest(raw=raw),self.assertRaises(ValueError):p.resource_rows(raw,0,len(raw))
 def test_allocated_close_rejects_early_pool_and_next_activation(self):
  text,result=fixture('allocated-close');pool=next(line+'\n'for line in text.splitlines()if line.startswith('[sext.app.pool.ready]'))
  bad,changed=rewrite(text,result,pool,'');marker='[task.finish] pd=15';bad,changed=rewrite(bad,changed,marker,pool+marker)
  with self.assertRaises(ValueError):self.verify(bad,changed,'allocated-close')
  text,result=fixture('allocated-close');activation=next(line+'\n'for line in text.splitlines()if line.startswith('[app.activation.committed]'))
  bad,changed=rewrite(text,result,activation,'');marker='[sext.app.pool.ready] target=15';bad,changed=rewrite(bad,changed,marker,activation+marker)
  with self.assertRaises(ValueError):self.verify(bad,changed,'allocated-close')
 def test_explicit_mode_and_new_schema_cannot_relabel_quiesced_158(self):
  good=dict(evidence_version=2,options=dict(mode='allocated-close'));result=dict(evidence_version=2,mode='allocated-close');self.assertEqual(p.evidence_mode(good,result,'allocated-close'),'allocated-close')
  for manifest,record,requested in[({},result,'allocated-close'),(dict(good,evidence_version=1),result,'allocated-close'),(good,dict(result,mode='quiesced'),'allocated-close'),(good,result,'quiesced'),(dict(good,options={}),result,'allocated-close')]:
   with self.subTest(manifest=manifest,record=record),self.assertRaises(ValueError):p.evidence_mode(manifest,record,requested)
  text,result=fixture();result['mode']='allocated-close'
  with self.assertRaises(ValueError):self.verify(text,result,'allocated-close')
 def test_host_close_observations_join_exact_hardware_request_pool_and_raw_prefixes(self):
  text,result=fixture('allocated-close');proof=self.verify(text,result,'allocated-close');result,actions,manifest=timing_fixture(text,result,proof)
  observed=p.close_observations(text,result,actions,proof,manifest);self.assertEqual([r['task']for r in observed['rows']],[150,300,310]);self.assertEqual([r['key_to_pool_ns']for r in observed['rows']],[4,4,4]);self.assertIn('not hard realtime',observed['scope'])
 def test_preexisting_close_request_cannot_be_claimed_by_later_hardware_f11(self):
  text,result=fixture('allocated-close');edge='[sexinput.ps2.scancode] raw=0x57\n';request='[shell.app.close.request] target=15 task=150 surface=300\n'
  bad,result=rewrite(text,result,edge+request,request+edge);proof=self.verify(bad,result,'allocated-close');result,actions,manifest=timing_fixture(bad,result,proof)
  actions[0]['start']=actions[0]['receipt_end']+1;actions[0]['start_sha256']=p.sha(bad[:actions[0]['start']].encode())
  self.assertTrue(p.verify_keys(bad,[actions[0]])['ok'])
  with self.assertRaisesRegex(ValueError,'F11 lacks the exact complete'):p.close_observations(bad,result,actions,proof,manifest)
 def test_reversed_f11_edges_reject_even_with_recomputed_serial_prefixes(self):
  text,result=fixture('allocated-close');proof=self.verify(text,result,'allocated-close')
  a=text.index('[sexinput.ps2.scancode] raw=0x57');b=text.index('[sexinput.ps2.scancode] raw=0xd7')
  bad=text[:a]+text[a:b].replace('raw=0x57','raw=0xd7',1)+text[b:].replace('raw=0xd7','raw=0x57',1)
  changed,actions,manifest=timing_fixture(bad,result,proof)
  with self.assertRaisesRegex(ValueError,'reversed physical'):p.close_observations(bad,changed,actions,proof,manifest)
 def test_host_observations_reject_forged_clock_cuts_subjects_and_non_f11(self):
  text,result=fixture('allocated-close');proof=self.verify(text,result,'allocated-close');result,actions,manifest=timing_fixture(text,result,proof)
  for index,key,value in[(0,'key','q'),(0,'issued_ns',True),(0,'start',True),(0,'issued_ns',float('nan')),(0,'receipt_observed_ns',999),(0,'receipt_serial_sha256','0'*64),(0,'receipt_serial_end',1),(0,'qmp',[dict(execute='send-key',arguments={})]),(1,'task',15),(1,'pool_observed_ns',999),(1,'key_to_pool_ns',True),(1,'key_to_pool_ns',1),(1,'serial_sha256','0'*64),(1,'key_index',True)]:
   changed=copy.deepcopy(actions);changed[index][key]=value
   with self.subTest(key=key,value=value),self.assertRaises(ValueError):p.close_observations(text,result,changed,proof,manifest)
  with self.assertRaises(ValueError):p.close_observations(text,result,actions,proof,dict(manifest,host_monotonic_end_ns=1000))
  with self.assertRaisesRegex(ValueError,'unrecorded/extra/truncated'):
   p.close_observations(text+'[sexinput.ps2.scancode] raw=0x57\n',result,actions,proof,manifest)
  changed=copy.deepcopy(actions);changed.append(copy.deepcopy(actions[0]))
  with self.assertRaises(ValueError):p.close_observations(text,result,changed,proof,manifest)
 def test_check_cli_forwards_explicit_mode_without_launching_guest(self):
  iso=self.root/'fixture.iso';binding=self.root/'binding.json';out=self.root/'evidence';iso.write_bytes(b'iso');binding.write_text('{}')
  for mode in('quiesced','allocated-close'):
   argv=['browser_reserve_gate.py','--iso',str(iso),'--binding',str(binding),'--out',str(out),'--mode',mode,'--check']
   with self.subTest(mode=mode),patch.object(sys,'argv',argv),patch.object(p,'replay',return_value=dict(ok=True,mode=mode))as replay,patch.object(p.subprocess,'Popen',side_effect=AssertionError('read-only check cannot launch')),contextlib.redirect_stdout(io.StringIO()):
    p.main();replay.assert_called_once_with(p.ROOT,iso.resolve(),binding.resolve(),out.resolve(),mode)
 def test_preparation_counts_source_pipe_metadata_not_only_queue_arrays(self):
  accounting=p.tls_accounting(self.root)
  self.assertEqual(accounting['pipe_bytes'],2*accounting['queue_bytes']+accounting['pipe_metadata_bytes'])
  self.assertEqual((accounting['pipe_bytes'],accounting['pipe_metadata_bytes']),(8248,56))
  text,result=fixture();self.assertTrue(self.verify(text,result)['ok'])
  # The previous fixture/oracle both omitted Pipe's56 metadata bytes.
  with self.assertRaisesRegex(ValueError,'local TLS bounds/source UTC differ'):
   self.verify(text.replace('task_bytes=10000 private_bytes=18248','task_bytes=10000 private_bytes=18192',1),result)
  contract=p.source_contract(ROOT)
  self.assertEqual(contract['tls_accounting'],accounting)
  self.assertEqual(contract['source_sha256']['crates/sex-tls/src/lib.rs'],p.sha((ROOT/'crates/sex-tls/src/lib.rs').read_bytes()))
 def test_unqualified_pipe_or_session_accounting_fails_before_trace_acceptance(self):
  path=self.root/'crates/sex-tls/src/lib.rs';source=path.read_text()
  for old,new in[('PIPE_METADATA_BYTES:usize=56','PIPE_METADATA_BYTES:usize=55'),
                  ('2*QUEUE_BYTES+PIPE_METADATA_BYTES','2*QUEUE_BYTES'),
                  ('private_bytes:task.layout.size()+core::mem::size_of::<Pipe>()','private_bytes:task.layout.size()+2*QUEUE_BYTES')]:
   self.assertIn(old,source);path.write_text(source.replace(old,new,1))
   with self.subTest(old=old),self.assertRaises(ValueError):p.tls_accounting(self.root)
  path.write_text(source)
 def test_missing_and_crossed_every_successor_identity(self):
  for index in[1,2]:
   for key,value in[('task',30),('surface',300),('predecessor',15),('target',15)]:
    if key=='predecessor'and index==1:continue
    text,result=fixture();result['chain'][index]['identity'][key]=value
    with self.subTest(index=index,key=key),self.assertRaises(ValueError):self.verify(text,result)
  text,result=fixture();result['chain'].pop(1)
  with self.assertRaises(ValueError):self.verify(text,result)
 def test_cleanup_requires_all_actual_roles_exact_erasure_and_no_early_pool(self):
  row=dict(ROWS[1],task=300);base=cleanup_lines(row)
  for old,new in[('holder=13 role=8','holder=11 role=8'),('acked=15 required=31','acked=7 required=31'),('zeroed=8192','zeroed=4096'),('pages=2 bytes=8192','pages=1 bytes=8192'),('task=300 holder=1','task=30 holder=1'),('bytes=8192 zeroed=8192','bytes=8193 zeroed=8193'),('high_water_retained=1','high_water_retained=0')]:
   with self.subTest(old=old),self.assertRaises(ValueError):p.cleanup(base.replace(old,new,1),row,0,len(base.replace(old,new,1)))
  lines=base.splitlines(True);pool=lines.pop();lines.insert(2,pool);bad=''.join(lines)
  with self.assertRaises(ValueError):p.cleanup(bad,row,0,len(bad))
 def test_duplicate_and_truncated_critical_close_records_fail(self):
  row=dict(ROWS[0],task=150);text=cleanup_lines(row)
  for bad in[text+text.splitlines(True)[0],text.rstrip('\n'),text.replace('[task.finish]','[task.finish.bad]')]:
   with self.assertRaises(ValueError):p.cleanup(bad,row,0,len(bad))
 def test_old_or_reordered_phase_windows_fail(self):
  for key in['start','work_start','close_start','end']:
   text,result=fixture();result['chain'][1][key]=result['chain'][0]['work_start']
   with self.subTest(key=key),self.assertRaises(ValueError):self.verify(text,result)
 def test_clock_source_slot_and_seed_release_cannot_cross_consumer(self):
  for old,new in[('task=130 slot=71','task=130 slot=61'),('owner=30 task=300 seq=1','owner=15 task=150 seq=1'),('owner=30 task=300 ticket=2 service_payload','owner=31 task=310 ticket=2 service_payload'),('utc_snapshot='+str(e.MIN_SECONDS+101),'utc_snapshot='+str(e.MIN_SECONDS+100)),('produced=32 consumed=0','produced=8 consumed=0')]:
   text,result=fixture();self.assertIn(old,text);bad=text.replace(old,new,1)
   with self.subTest(old=old),self.assertRaises(ValueError):self.verify(bad,result)
 def test_missing_boot_seal_and_unobserved_source_versions_fail(self):
  for old,new in[('runtime_map=capability_leaf','runtime_map=ambient'),('rows=3 actual_grants=1','rows=2 actual_grants=1'),('trust_version=1','trust_version=2')]:
   text,result=fixture();bad=text.replace(old,new,1)
   with self.subTest(old=old),self.assertRaises(ValueError):self.verify(bad,result)
 def test_exhaustion_requires_denial_and_no_new_activation(self):
  text,result=fixture()
  for old,new in[('CLOSED / RESERVE EXHAUSTED','RUNNING / FOCUS'),('launcher.execute.denied','launcher.execute.accepted')]:
   bad=text.replace(old,new)
   with self.subTest(old=old),self.assertRaises(ValueError):self.verify(bad,result)
  bad=text+'[app.activation.committed] logical=15 incarnation=4 predecessor=31 successor=32 task=320 surface=345 pages=2 runtime_alloc=0 runtime_mint=0 bsp=1\n'
  with self.assertRaises(ValueError):self.verify(bad,result)
 def test_serial_utf8_complete_prefix_and_immutable_capture_prefix(self):
  path=self.root/'serial.log';path.write_bytes(b'hello\r\n'+bytes([0xe2,0x86]));self.assertEqual(p.serial_text(path),'hello\n')
  path.write_bytes(b'hello\r\n'+bytes([0xe2,0x86,0x92]));self.assertEqual(p.serial_text(path),'hello\n→')
  path.write_bytes(b'bad\xff\n')
  with self.assertRaises(UnicodeDecodeError):p.serial_text(path)
  frame=dict(serial_end=2,serial_sha256=p.sha('ab'.encode()));self.assertEqual(p.frame_prefix('abcd',frame),'ab');frame['serial_end']=3
  with self.assertRaises(ValueError):p.frame_prefix('abcd',frame)
 def test_key_release_and_fresh_hardware_order_required(self):
  text='[sexinput.ps2.scancode] raw=0x26\n[sexinput.ps2.scancode] raw=0xa6\n';action=dict(kind='key',key='l',start=0,end=len(text),transport='input-send-event')
  self.assertTrue(p.verify_keys(text,[action])['ok'])
  for bad in[text.replace('raw=0xa6','raw=0x26'),text.replace('raw=0x26','raw=0x24')]:
   with self.assertRaises(ValueError):p.verify_keys(bad,[dict(action,end=len(bad))])
 def test_palette_uses_new_browser_index_without_changing_old_zoom_index(self):
  calls=[];p.palette(dict(key=lambda code,pattern:calls.append((code,pattern))))
  self.assertEqual([code for code,_ in calls],['grave_accent']+['j']*10);self.assertIn('old=9 new=10',calls[-1][1])
 def test_contract_requires_every_source_and_runner_hash(self):
  c=dict(source_sha256={'a':'11','b':'22'});p.require_inputs(dict(sources={'a':'11'},runner_sha256={'b':'22'}),c)
  with self.assertRaises(ValueError):p.require_inputs(dict(sources={'a':'11'},runner_sha256={}),c)
class CapturedPaintBoundaries(unittest.TestCase):
 def frame(self,before,after):return dict(serial_end=len(before),serial_sha256=p.sha(before.encode()),post_serial_end=len(after),post_serial_sha256=p.sha(after.encode()))
 def test_late_receipt_cannot_qualify_an_earlier_screendump(self):
  row=dict(target=30,task=300,surface=343);before='[browser.tls.layout] target=30 task=300 surface=343\n'
  after=before+'[clock.source.snapshot] unrelated civil update\n'
  self.assertEqual(p.capture_fence(after,self.frame(before,after),row),after)
  for event in ('[browser.tls.applied] target=30 task=300 surface=343\n','[display.browser.text.committed] target=30 task=300 surface=343\n','[browser.viewport] target=30 task=300 sequence=6\n','[browser.tls.view] state=Cancelled\n','[shell.focus.set] id=201\n'):
   changed=before+event
   with self.subTest(event=event),self.assertRaises(ValueError):p.capture_fence(changed,self.frame(before,changed),row)
 def test_post_capture_prefix_is_immutable_and_bounded(self):
  text='public frame\n';row=dict(target=31,task=310,surface=344);frame=self.frame(text,text)
  for key,value in [('post_serial_end',len(text)+1),('post_serial_end',-1),('post_serial_sha256','0'*64),('serial_sha256','0'*64)]:
   changed={**frame,key:value}
   with self.subTest(key=key,value=value),self.assertRaises(ValueError):p.capture_fence(text,changed,row)
 def test_resize_requires_exact_zoom_and_enter_but_allows_late_caller_return(self):
  row=dict(target=30,task=300,surface=343)
  view='[browser.viewport] target=30 task=300 sequence=6 w=1024 h=718 origin=held_shell\n'
  zoom='[shell.window.action] action=ZoomToggle frame=20 sid=343 ok=1 reason=ok\n'
  text=view+zoom+'layout\n';geometry=dict(viewport=6,tile=dict(w=1024,h=718),layout_offset=len(view+zoom))
  actions=[dict(kind='key',key='ret',start=0,end=len(view+zoom))]
  self.assertEqual(p.resize_evidence(text,row,0,len(text),geometry,actions)['viewport_offset'],0)
  prefix='unrelated immutable history\n'*100;offset=len(prefix);shifted={**geometry,'layout_offset':geometry['layout_offset']+offset};shifted_actions=[{**a,'start':a['start']+offset,'end':a['end']+offset}for a in actions]
  proof=p.resize_evidence(prefix+text,row,offset,offset+len(text),shifted,shifted_actions)
  self.assertEqual(proof['viewport_offset'],offset);self.assertEqual(proof['zoom_offset'],offset+len(view))
  for changed in (text.replace(zoom,''),text.replace('sid=343','sid=300'),text.replace(' reason=ok',''),text+zoom):
   with self.subTest(changed=changed),self.assertRaises(ValueError):p.resize_evidence(changed,row,0,len(changed),geometry,actions)
  for invalid in ([],actions*2,[dict(kind='key',key='ret',start=len(view+zoom),end=len(text))]):
   with self.subTest(actions=invalid),self.assertRaises(ValueError):p.resize_evidence(text,row,0,len(text),geometry,invalid)

if __name__=='__main__':unittest.main()
