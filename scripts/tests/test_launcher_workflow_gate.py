"""Synthetic checker controls only; native authority and pixels require QEMU."""
from pathlib import Path
import copy,json,sys,tempfile,unittest
from unittest import mock
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import launcher_workflow_gate as g
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

def action(target,kind,origin,to,task=None,queued=0):
 task=target if task is None else task;activated=int(target==17 and kind=='Open');queued=int(queued or activated)
 return f'[launcher.action] target={target} task={task} frame={g.TARGETS[target]["frame"]} action={kind} from={origin} to={to} visible={0 if queued else 1} focused={0 if queued else 1} activated={activated} queued={queued} accepted=1 ok={0 if queued else 1} pixels=unconfirmed\n'
def trace(tasks=None):
 tasks={9:9,12:12,17:17}if tasks is None else tasks;out=''
 for target in g.TARGETS:out+=f'[boot.app.resources] target={target} task={tasks[target]} surface={g.TARGETS[target]["surface"]} private_pages=92 required=31 coordinator=3\n'
 out+=f'[boot.app.successor] logical=12 incarnation=2 predecessor=12 successor=17 task={tasks[17]} surface=302 key=12 dormant=1 leaves=92 runtime_mint=0\n'
 for target in g.TARGETS:
  origin=0 if target!=17 else 1;destination=1-origin;task=tasks[target];frame=g.TARGETS[target]['frame'];fid=target+100
  if target!=9:
   if target==17:out+=f'[app.activation.committed] logical=12 incarnation=2 predecessor=12 successor=17 task={task} surface=302 pages=92 runtime_alloc=0 runtime_mint=0 bsp=1\n'
   out+=action(target,'Open',origin,origin,task)
   if target==17:out+=f'[spindle.incarnation.start] target=17 task={task} surface=302 fresh=1\n[app.surface.presented] target=17 surface=302 sequence=1 ok=1\n'
  out+=action(target,'Focus',origin,origin,task)
  out+=f'[silk.lifecycle.minimize.hidden] sid={frame} frame={fid}\n[silk.lifecycle.restore.ok] frame={fid} sid={frame}\n'
  out+=action(target,'Restore',origin,origin,task)
  out+=f'[silk.frame.scene.move.done] frame={fid} from={origin} to={destination} ownership_unique=1 focus_valid=1 ok=1\n'
  out+=action(target,'BringHere',origin,destination,task)
  if target==9:continue
  out+=f'[app.close.accepted] target={target} task={task} coordinator=3\n[net.dns.records.retired] target={target} task={task} remaining=0 before_cleanup_ack=1\n[net.app.cleanup.ack] target={target} task={task} client_leases=0 shared_cache=retained ok=1\n'
  for role in [1,2,4,8,16]:out+=f'[app.service.ack] target={target} task={task} holder=13 role={role} ok=1\n'
  out+=f'[app.private.retired] target={target} task={task} pages=92 bytes=376832 acked=31 required=31 all_flushed=1 pool_reusable=1\n[sext.app.pool.ready] target={target} task={task} bytes=376832 zeroed=376832 reused=1 zero_owner=sext ok=1\n'
 return out+'[launcher.execute.denied] idx=0 status=CLOSED reason=boot_reserve_exhausted\n'

def ppm(w,h,color=(0x30,0x38,0x60)):
 return b'P6\n'+f'{w} {h}\n255\n'.encode()+bytes(color)*(w*h)
def pixel(image,w,x,y,color):
 header=image.index(b'\n255\n')+5;data=bytearray(image);offset=header+(y*w+x)*3;data[offset:offset+3]=color;return bytes(data)
def boot_trace():
 return ''.join(f'[display.child.parent_gate] target=12 surface={sid} parent=153 observed=0 standalone=0 retained=1\n'for sid in g.TARGETS[12]['panes'])+'[shell.tile.apply] sid=300 x=512 y=425 w=512 h=343\n'

class LauncherWorkflow(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=g.source_contract(ROOT)
 def test_actual_source_has_complete_cohort_bindings_and_hit_geometry(self):
  self.assertEqual(self.c['launcher_count'],10);self.assertEqual(self.c['launcher_app_count'],7);self.assertEqual(self.c['exhausted_title'],'Open Spindle')
  self.assertEqual(g.frame_specs(self.c)[-1]['title'],self.c['exhausted_title'])
  self.assertGreater(len(self.c['source_sha256']),20);self.assertEqual(self.c['grid'],[1008,632,264,160]);self.assertEqual(self.c['canvas'],0x0c1420)
  self.assertEqual(g.minimize_point(12,dict(x=200,y=100),self.c),(225,114))
  self.assertEqual(g.minimize_point(9,dict(x=208,y=136),self.c),(225,114))
 def test_exact_finite_actions_cleanup_and_boot_successor_pass(self):
  r=g.lifecycle_joins(trace());self.assertEqual(len(r['actions']),11);self.assertEqual([c['target']for c in r['cleanup']],[12,17])
 def test_actual_task_is_separate_catalog_data_not_domain_alias(self):
  self.assertEqual(g.lifecycle_joins(trace({9:90,12:120,17:170}))['catalogs'][17]['task'],170)
 def test_queued_to_actual_positive_is_required_for_existing_actions(self):
  raw=trace();positive=action(12,'Restore',0,0);pending=action(12,'Restore',0,0,queued=1);raw=raw.replace(positive,pending+positive)
  self.assertEqual(len(g.lifecycle_joins(raw)['actions']),11)
  for invalid in [raw.replace(positive,''),raw.replace(pending,pending*2),raw.replace(positive,positive.replace('task=12','task=99')),raw.replace(positive,positive.replace('visible=1','visible=0'))]:
   with self.subTest(invalid=invalid[-150:]):
    with self.assertRaises(ValueError):g.lifecycle_joins(invalid)
 def test_successor_open_uses_queued_activation_and_real_later_presentation(self):
  raw=trace();self.assertTrue(g.lifecycle_joins(raw)['actions'][7]['queued'])
  for old,new in [('activated=1 queued=1 accepted=1 ok=0','activated=1 queued=0 accepted=1 ok=1'),('[app.surface.presented] target=17','[app.surface.presented] target=99'),('[spindle.incarnation.start] target=17','[spindle.incarnation.start] target=12')]:
   with self.subTest(old=old):
    with self.assertRaises(ValueError):g.lifecycle_joins(raw.replace(old,new))
 def test_foreign_duplicate_malformed_failed_and_reordered_effects_fail(self):
  raw=trace();one=action(12,'Focus',0,0)
  bad=[raw.replace(one,''),raw.replace(one,one*2),raw.replace(one,one.replace('target=12','target=99')),raw.replace(one,one.replace('frame=153','frame=154')),raw.replace(one,one.replace('task=12','task=17')),raw.replace(one,one.replace('ok=1','ok=0')),raw.replace(one,one.replace('accepted=1','accepted=0')),raw.replace(one,one.replace('focused=1','focused=2')),raw.replace(one,one.replace('action=Focus','action=Restore'))]
  for value in bad:
   with self.subTest(value=value[-100:]):
    with self.assertRaises(ValueError):g.lifecycle_joins(value)
 def test_existing_restore_bring_or_focus_cannot_reactivate(self):
  for name in ('Focus','Restore','BringHere'):
   raw=trace();old=action(12,name,0,1 if name=='BringHere'else 0)
   with self.assertRaises(ValueError):g.lifecycle_joins(raw.replace(old,old.replace('activated=0','activated=1')))
 def test_cleanup_roles_pages_zeroing_and_order_are_mandatory(self):
  raw=trace();bad=[raw.replace('role=16','role=8',1),raw.replace('before_cleanup_ack=1','before_cleanup_ack=0',1),raw.replace('zeroed=376832','zeroed=1',1),raw.replace('pages=92 bytes=376832','pages=91 bytes=376832',1),raw.replace('ownership_unique=1','ownership_unique=0',1),raw.replace('frame=112 from=0','frame=117 from=0',1)]
  for value in bad:
   with self.assertRaises(ValueError):g.lifecycle_joins(value)
 def test_missing_or_duplicate_activation_and_preactivation_start_fail(self):
  raw=trace();line=next(row+'\n'for row in raw.splitlines()if row.startswith('[app.activation.committed]'))
  for value in [raw.replace(line,''),raw.replace(line,line*2),line+raw.replace(line,'')]:
   with self.assertRaises(ValueError):g.lifecycle_joins(value)
 def test_terminal_predecessor_cannot_render_or_start_new_network_work(self):
  for extra in ['[spindle.view.accepted] target=12 task=12 sequence=99\n','[spindle.net.intent] target=17 task=17 command=net-probe\n']:
   with self.assertRaises(ValueError):g.lifecycle_joins(trace()+extra)
 def test_cleanboot_checks_all_catalog_gate_records_and_no_ink(self):
  c=dict(self.c);image=ppm(1024,768);result=g.no_ghost(boot_trace(),image,c,(1024,768));self.assertEqual(result['forbidden_ink'],0);self.assertEqual(result['forbidden_canvas'],0);self.assertGreater(result['canvas_checked_pixels'],0)
  for malformed in [boot_trace().replace('surface=160','surface=302'),boot_trace()+boot_trace().splitlines()[0]+'\n',boot_trace().replace('standalone=0','standalone=1'),boot_trace().replace('observed=0','observed=1')]:
   with self.assertRaises(ValueError):g.no_ghost(malformed,image,c,(1024,768))
 def test_foreground_and_opaque_only_ghosts_both_fail(self):
  for color in [self.c['foreground'].to_bytes(3,'big'),self.c['canvas'].to_bytes(3,'big')]:
   image=pixel(ppm(1024,768),1024,1010,700,color)
   with self.assertRaises(ValueError):g.no_ghost(boot_trace(),image,self.c,(1024,768))
 def test_browser_rim_is_excluded_only_from_opaque_scan(self):
  image=pixel(ppm(1024,768),1024,1020,700,self.c['canvas'].to_bytes(3,'big'));self.assertEqual(g.no_ghost(boot_trace(),image,self.c,(1024,768))['forbidden_canvas'],0)
  image=pixel(image,1024,1020,700,self.c['foreground'].to_bytes(3,'big'))
  with self.assertRaises(ValueError):g.no_ghost(boot_trace(),image,self.c,(1024,768))
 def test_cleanboot_rejects_missing_browser_bounds_and_preopened_spindle(self):
  image=ppm(1024,768)
  for raw in [boot_trace().replace('sid=300','sid=301'),boot_trace()+'[spindle.placeholder.open] frame=4\n',boot_trace()+'[shell.app.viewport] target=12 sid=154 x=0 y=50 w=1 h=1\n',boot_trace()+action(12,'Open',0,0)]:
   with self.assertRaises(ValueError):g.no_ghost(raw,image,self.c,(1024,768))
 def test_frame_expectations_cannot_weaken_identity_kind_marker_or_title(self):
  specs=g.frame_specs(self.c);g.frame_contract(specs,self.c)
  for key,value in [('kind','boot'),('marker',''),('target',17),('title','Open Fake')]:
   rows=copy.deepcopy(specs);index=next(i for i,row in enumerate(rows)if row['name']=='spindle-12-typed');rows[index][key]=value
   with self.assertRaises(ValueError):g.frame_contract(rows,self.c)
  with self.assertRaises(ValueError):g.frame_contract(specs[:-1],self.c)
 def test_terminal_caption_is_catalog_derived_not_last_live_action(self):
  changed=dict(self.c,exhausted_title='Catalog Spindle')
  self.assertEqual(g.frame_specs(changed)[-1]['title'],'Catalog Spindle')
  with self.assertRaises(ValueError):g.frame_contract(g.frame_specs(self.c),changed)
 def test_cancelled_actual_action_and_utf8_boot_boundary_fail_closed(self):
  raw=trace();positive=action(12,'Restore',0,0);rejected=positive.replace('accepted=1 ok=1','accepted=0 ok=0')
  with self.assertRaises(ValueError):g.lifecycle_joins(raw.replace(positive,rejected))
  prefix='boot \u2500\n';full=(prefix+raw).encode();frame=dict(serial_bytes=len(prefix.encode()),serial_sha256=g.sha(prefix.encode()))
  boundary=len(g.input_gate.capture_prefix(full,frame));self.assertNotEqual(boundary,frame['serial_bytes'])
  self.assertEqual(len(g.lifecycle_joins(full.decode(),boundary)['actions']),11)
 def test_saved_serial_prefix_requires_exact_byte_boundary_and_hash(self):
  raw=b'prefix\nnext\n';frame=dict(serial_bytes=7,serial_sha256=g.sha(raw[:7]));self.assertEqual(g.input_gate.capture_prefix(raw,frame),'prefix\n')
  for changed in [dict(frame,serial_bytes=len(raw)+1),dict(frame,serial_sha256='0'*64),{},dict(frame,serial_bytes=0)]:
   with self.assertRaises(ValueError):g.input_gate.capture_prefix(raw,changed)
 def test_launcher_title_requires_visible_exact_action_and_positive_applied(self):
  raw='[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=0 first=0 rows=1\n[launcher.text.row] index=0 row=0 name=Restore Spindle status=RUNNING / LAST INSTANCE\n[launcher.text.admitted] complete=1 selected=0 bytes=80 applied=1 pixels=unconfirmed\n'
  view,lines=g.launcher_layout(raw);data=bytearray(1280*800*3)
  for row,line in lines:
   for col,char in enumerate(line[:38]):
    for gy,bits in enumerate(glyph(self.c['font'],char)):
     for gx in range(5):
      if bits&(1<<(4-gx)):
       for dy in range(2):
        for dx in range(2):
         x=408+col*12+gx*2+dx;y=236+row*18+gy*2+dy;data[(y*1280+x)*3:(y*1280+x)*3+3]=b'\xf0\xf8\xff'
  image=b'P6\n1280 800\n255\n'+data;frame=dict(name='launcher-12-restore',kind='launcher',target=12,title='Restore Spindle');g.verify_frame(raw,image,frame,self.c,(1280,800))
  for bad_trace,bad_frame in [(raw.replace('applied=1','applied=0'),frame),(raw,dict(frame,title='Focus Spindle')),(raw.replace('name=Restore','name=Open'),frame)]:
   with self.assertRaises(ValueError):g.verify_frame(bad_trace,image,bad_frame,self.c,(1280,800))
  with self.assertRaises(ValueError):g.verify_frame(raw,ppm(1280,800),frame,self.c,(1280,800))


class LauncherErasureControls(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=g.source_contract(ROOT)
 def trace(self,linen='x=0 y=50 w=640 h=750'):
  return ('[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=0 first=0 rows=1\n'
   '[launcher.text.row] index=0 row=0 name=Focus Spindle status=RUNNING / LAST INSTANCE\n'
   '[command_palette.close]\n[linen.list.text] '+linen+' scale=2 text_color=0xf0f8ff rows=3 committed_view=1\n')
 def ink(self,trace,only_row=None,clip=None):
  image=ppm(1280,800);v,lines=g.launcher_layout(trace)
  for row,line in lines:
   if only_row is not None and row!=only_row:continue
   for col,char in enumerate(line[:38]):
    for gy,bits in enumerate(glyph(self.c['font'],char)):
     for gx in range(5):
      if not bits&(1<<(4-gx)):continue
      for dy in range(2):
       for dx in range(2):
        x=v['x']+8+(col*6+gx)*2+dx;y=v['y']+36+(row*9+gy)*2+dy
        if clip is None or clip[0]<=x<clip[2]and clip[1]<=y<clip[3]:image=pixel(image,1280,x,y,b'\xf0\xf8\xff')
  return image
 def test_actual_old_signature_and_lower_rows_after_header_erasure_fail(self):
  trace=self.trace();self.assertEqual(g.launcher_erased(trace,ppm(1280,800),self.c,(1280,800))['remaining_launcher_ink'],0)
  for row in (None,0,3):
   with self.subTest(row=row):
    with self.assertRaisesRegex(ValueError,'old launcher text remains'):g.launcher_erased(trace,self.ink(trace,row),self.c,(1280,800))
 def test_known_linen_band_overlap_excluded_but_coverage_mandatory(self):
  trace=self.trace('x=400 y=200 w=80 h=280')
  allowed=self.ink(trace,clip=(408,236,472,416))
  result=g.launcher_erased(trace,allowed,self.c,(1280,800));self.assertGreater(result['linen_overlap_skipped'],0)
  for invalid in (self.trace('x=0 y=200 w=1280 h=750'),trace.replace('[command_palette.close]',''),trace.replace('[linen.list.text]','[wrong.list.text]')):
   with self.assertRaises(ValueError):g.launcher_erased(invalid,allowed,self.c,(1280,800))

class WireAndDriverControls(unittest.TestCase):
 def capture(self,payload=None,port=46078,extra=False):
  import struct
  from check_tcp_capture import checksum
  payload=g.ECHO if payload is None else payload;local=bytes([10,0,2,15]);remote=bytes([10,0,2,2]);packets=[]
  def packet(out,seq,ack,flags,data=b'',guest=48001):
   src,dst=(local,remote)if out else(remote,local);sport,dport=(guest,port)if out else(port,guest)
   tcp=struct.pack('!HHIIBBHHH',sport,dport,seq,ack,5<<4,flags,65535,0,0)+data
   pseudo=src+dst+b'\0\6'+len(tcp).to_bytes(2,'big');check=checksum(pseudo+tcp);tcp=tcp[:16]+check.to_bytes(2,'big')+tcp[18:]
   ip=struct.pack('!BBHHHBBH4s4s',0x45,0,20+len(tcp),0,0,64,6,0,src,dst);ip=ip[:10]+checksum(ip).to_bytes(2,'big')+ip[12:]
   packets.append(b'\0'*12+b'\x08\x00'+ip+tcp)
  packet(True,100,0,2);packet(False,200,101,18);packet(True,101,201,16);packet(True,101,201,24,payload);packet(True,101+len(payload),201,17);packet(False,201,102+len(payload),24,g.ECHO);packet(False,201+len(g.ECHO),102+len(payload),17);packet(True,102+len(payload),202+len(g.ECHO),16)
  if extra:packet(True,300,0,2,guest=48002)
  return struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,65535,1)+b''.join(struct.pack('<IIII',i,0,len(p),len(p))+p for i,p in enumerate(packets,1))
 def test_actual_capture_joins_single_startup_guest_source_port_not_service_port(self):
  result=g.wire_check(self.capture());self.assertEqual(result['connections'][0]['guest_port'],48001)
  for raw in [self.capture(g.ECHO+b'extra'),self.capture(port=46080),self.capture(extra=True),self.capture()[:-1]]:
   with self.assertRaises(ValueError):g.wire_check(raw)
 def test_retained_frame_boundaries_are_not_mutable_latest_serial(self):
  raw=b'old\nnew\n';frame=dict(serial_bytes=4,serial_sha256=g.sha(raw[:4]));self.assertEqual(g.input_gate.capture_prefix(raw,frame),'old\n')
  with self.assertRaises(ValueError):g.input_gate.capture_prefix(b'bad\nnew\n',frame)

class PointerAndPrefixControls(unittest.TestCase):
 def test_pointer_waits_normalized_xy_then_distinct_down_and_up_receipts(self):
  order=[]
  class Q:
   def cmd(self,operation,args):order.append(('cmd',operation,args))
  def wait(pattern,start):order.append(('wait',pattern,start))
  g.pointer_click(Q(),225,114,(1280,800),lambda:'prefix',wait,lambda row:order.append(('record',row)))
  self.assertEqual([row[0]for row in order],['record','cmd','wait','cmd','wait','cmd','wait'])
  self.assertIn('raw_x=',order[2][1]);self.assertIn('raw_y=',order[2][1]);self.assertIn('pressed=true',order[4][1]);self.assertIn('pressed=false',order[6][1]);self.assertTrue(order[3][2]['events'][0]['data']['down']);self.assertFalse(order[5][2]['events'][0]['data']['down'])
  with self.assertRaises(ValueError):g.pointer_click(Q(),1280,114,(1280,800),lambda:'',wait,lambda _:None)
 def test_safe_parking_alternates_with_receipt_barriers_and_no_sleep(self):
  for resolution in ((1280,800),(1024,768)):
   order=[]
   class Q:
    def cmd(self,operation,args):order.append(('cmd',args))
   def wait(pattern,start):order.append(('wait',pattern,start))
   def record(row):order.append(('record',row))
   with mock.patch.object(g.time,'sleep',side_effect=AssertionError('movement uses actual receipt')):
    g.pointer_park(Q(),resolution,lambda:'prefix',wait,record,0)
    g.pointer_park(Q(),resolution,lambda:'prefix plus',wait,record,1)
   self.assertEqual([row[0]for row in order],['record','cmd','wait']*2)
   self.assertEqual((order[0][1]['x'],order[0][1]['y']),(resolution[0]-20,10))
   self.assertEqual(order[3][1]['x'],resolution[0]-21)
   self.assertNotEqual(order[1][1],order[4][1]);self.assertEqual(order[5][2],len('prefix plus'))
   self.assertIn('raw_x=',order[2][1]);self.assertIn('raw_y=',order[2][1])
 def test_click_parks_only_after_actual_release_receipt(self):
  order=[]
  class Q:
   def cmd(self,*args):order.append('cmd')
  def wait(pattern,start):order.append('release'if'pressed=false'in pattern else'barrier')
  with mock.patch.object(g.time,'sleep',side_effect=AssertionError('no arbitrary wait')):
   g.pointer_click(Q(),225,114,(1280,800),lambda:'prefix',wait,lambda _:None,lambda:order.append('park'))
  self.assertEqual(order[-2:],['release','park'])
 def test_failed_capture_keeps_exact_last_image_and_serial_prefix(self):
  with tempfile.TemporaryDirectory()as directory:
   out=Path(directory);path=out/'failed.ppm';image=ppm(2,2);path.write_bytes(image);snapshot=b'exact last prefix\n';frame=dict(name='failed',kind='launcher',target=12,title='Open Spindle')
   receipt=g.failed_frame(out,path,frame,snapshot,'glyph mismatch=10')
   self.assertEqual(receipt['screenshot_sha256'],g.sha(image));self.assertEqual(receipt['result'],dict(ok=False,errors=['glyph mismatch=10']))
   self.assertEqual((out/receipt['serial_prefix']).read_bytes(),snapshot)
   self.assertEqual(g.input_gate.capture_prefix(snapshot+b'future\n',receipt),snapshot.decode())
   self.assertEqual(json.loads((out/'failed.failed.json').read_text()),receipt)
   with self.assertRaises(ValueError):g.input_gate.capture_prefix(b'changed prefix\n',receipt)
   with self.assertRaises(ValueError):g.failed_frame(out,path,frame,snapshot,'overwrite')
 def test_frame_cannot_borrow_a_future_or_foreign_action_from_full_serial(self):
  frame=dict(name='spindle-12-restored',target=12)
  with self.assertRaises(ValueError):g.frame_action(action(12,'Focus',0,0),frame)
  with self.assertRaises(ValueError):g.frame_action(action(17,'Restore',1,1),frame)
  self.assertEqual(g.frame_action(action(12,'Restore',0,0),frame)['target'],12)
  with self.assertRaises(ValueError):g.frame_action(action(12,'Restore',0,0,queued=1),frame)
 def test_async_successor_open_requires_actual_presented_in_same_prefix(self):
  frame=dict(name='spindle-17-opened',target=17);raw=action(17,'Open',1,1)
  with self.assertRaises(ValueError):g.frame_action(raw,frame)
  self.assertEqual(g.frame_action(raw+'[app.surface.presented] target=17 surface=302 sequence=1 ok=1\n',frame)['target'],17)

if __name__=='__main__':unittest.main()
