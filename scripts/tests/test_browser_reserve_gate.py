"""Synthetic parser controls only; no guest/BOOT/native success is asserted."""
import copy
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

def fixture():
 text=inventory(e.FUTURE_PLANS,True);at=text.index('[net.source.binding.held]')
 text=text[:at]+'[boot.app.catalog.complete] rows=3 actual_grants=1 runtime_mint=0\n[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0\n[bootgraph.boot.handoff] target=sexdisplay id=1 entry=0x4000\n'+text[at:]
 result={'chain':[]};tasks={15:150,30:300,31:310}
 for i,base in enumerate(ROWS):
  row=dict(base,task=tasks[base['target']]);t,k,s=row['target'],row['task'],row['surface'];start=0 if i==0 else len(text)
  if i:text+=f'[app.activation.committed] logical=15 incarnation={i+1} predecessor={row["predecessor"]} successor={t} task={k} surface={s} pages=2 runtime_alloc=0 runtime_mint=0 bsp=1\n[shell.app.relaunch] predecessor={row["predecessor"]} successor={t} task={k} surface={s} receipt_replayed=1\n'
  text+=f'[shell.app.registered.reply] target={t} task={k} surface={s} committed=1\n[app.surface.presented] target={t} surface={s} sequence=1 ok=1\n[shell.focus.set] id={s}\n'
  from check_tcp_capture import fingerprint
  text+=f'[net.job.state] owner=Some(Identity {{ pd: {t}, task: {k} }}) ticket={i+1} from=Connecting to=Sending staged=1025 queued=1025 received=0 scope=private_job\n[browser.net.job.bytes] ticket={i+1} length=1025 hash={hex(fingerprint(p.ECHO))} exact=1 endpoint=controlled_peer tls=0 web=0\n[browser.net.job.released] ticket={i+1} view=Verified private_job=0\n'
  begin=len(text);utc=e.MIN_SECONDS+100+i
  text+=f'[browser.command.started] kind=TlsPrepare accepted=1\n[trust.policy.snapshot] owner={t} task={k} seq=1 profile=1 root_bytes=376 origin=held_boot_grant\n'
  text+=clock(13,130,row['clock'],utc,i+1)+success(t,k,1,utc)
  text+=f'[entropy.seed.admitted] owner={t} task={k} ticket={i+1} bytes=32 source=held_boot_grant\n'
  text+=f'[entropy.seed.progress] owner=Some(Identity {{ pd: {t}, task: {k} }}) ticket={i+1} state=Some(Ready) produced=32 consumed=0 failure=0 private=1\n'
  text+=f'[entropy.seed.released] owner={t} task={k} ticket={i+1} service_payload=erased receipt=retained\n'
  text+=f'[browser.tls.prepared] task_bytes=10000 private_bytes=18248 read=4096 write=2048 tx_bytes=100 utc_snapshot={utc} authenticated=0 network=0\n'
  close_start=len(text);text+=cleanup_lines(row);result['chain'].append(dict(identity=row,start=start,work_start=begin,close_start=close_start,end=len(text)))
 result['exhaustion_start']=len(text);text+='[launcher.text.row] index=10 row=4 name=Open Browser status=CLOSED / RESERVE EXHAUSTED\n[launcher.execute.denied] idx=10 status=CLOSED / RESERVE EXHAUSTED reason=boot_reserve_exhausted\n';result['exhaustion_end']=len(text)
 return text,result

class BrowserReserve(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
  path=self.root/'crates/sex-pdx/src/app_catalog.rs';path.parent.mkdir(parents=True);path.write_text('pub const INCARNATIONS: [Incarnation; 3] = [];')
  for name in('crates/sex-tls/src/lib.rs','apps/kaleidoscope/src/tls_probe.rs'):
   path=self.root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes((ROOT/name).read_bytes())
  for target,value in[(p,'browser_rows'),(p.sources,'plans_from_catalog')]:
   replacement=ROWS if value=='browser_rows'else e.FUTURE_PLANS
   patcher=patch.object(target,value,return_value=replacement);patcher.start();self.addCleanup(patcher.stop)
 def verify(self,text,result):return p.verify(text,result,self.root)
 def test_complete_three_unequal_tasks_and_cumulative_cleanup(self):
  text,result=fixture();checked=self.verify(text,result)
  self.assertEqual(checked['targets'],[15,30,31]);self.assertEqual([s['identity']['task']for s in checked['joins']],[150,300,310]);self.assertFalse(checked['browser_pixels_qualified']);self.assertTrue(checked['reserve_exhausted'])
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
  zoom='[shell.window.action] action=ZoomToggle frame=20 sid=343 ok=1\n'
  text=view+zoom+'layout\n';geometry=dict(viewport=6,tile=dict(w=1024,h=718),layout_offset=len(view+zoom))
  actions=[dict(kind='key',key='ret',start=0,end=len(view+zoom))]
  self.assertEqual(p.resize_evidence(text,row,0,len(text),geometry,actions)['viewport_offset'],0)
  prefix='unrelated immutable history\n'*100;offset=len(prefix);shifted={**geometry,'layout_offset':geometry['layout_offset']+offset};shifted_actions=[{**a,'start':a['start']+offset,'end':a['end']+offset}for a in actions]
  proof=p.resize_evidence(prefix+text,row,offset,offset+len(text),shifted,shifted_actions)
  self.assertEqual(proof['viewport_offset'],offset);self.assertEqual(proof['zoom_offset'],offset+len(view))
  for changed in (text.replace(zoom,''),text.replace('sid=343','sid=300'),text+zoom):
   with self.subTest(changed=changed),self.assertRaises(ValueError):p.resize_evidence(changed,row,0,len(changed),geometry,actions)
  for invalid in ([],actions*2,[dict(kind='key',key='ret',start=len(view+zoom),end=len(text))]):
   with self.subTest(actions=invalid),self.assertRaises(ValueError):p.resize_evidence(text,row,0,len(text),geometry,invalid)

if __name__=='__main__':unittest.main()
