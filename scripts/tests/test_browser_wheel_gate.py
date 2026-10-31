"""Independent wheel custody/arithmetic controls. No native execution."""
import copy,hashlib,sys,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import http_content_gate as gate
import test_http_reader_pixels as reader_fixture
ROOT=Path(__file__).resolve().parents[2]

class WheelChecker(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=gate.load_wheel_contract(ROOT)
 def fixture(self,wheel=-1,target='body',before=0,post=True,reports=None,resolution=(1280,800)):
  w,h,x,y=(640,375,640,425)if resolution[0]==1280 else(512,359,512,409)
  tile=dict(w=w,h=h,x=x,y=y);model=reader_fixture.ReaderPixels();model.contract=self.contract;frame=gate.reader_frame(gate.READER_BODY,w,h,self.contract,before)
  if target in('pending','closed'):
   trace=f'[shell.document.viewport] target=15 task=15 surface=300 sequence=1 x={x} y={y} w={w} h={h} enqueued=1\n[browser.viewport] target=15 task=15 sequence=1 w={w} h={h} origin=held_shell\n'
   trace+='[net.job.stream.drain] owner=15 task=15 ticket=2 produced=2048 consumed=1024 buffered=1024 ring=1024 state_before=Receiving receipt=retained not_delivery=1\n'if target=='pending'else'[sext.app.pool.ready] target=15 task=15 bytes=4096 ok=1\n'
  else:trace=model.trace(frame,gate.READER_BODY,w,h,x,y,position=True)
  px,py=x+w-24,y+64
  if target in('outside','closed'):px,py=resolution[0]-20,10
  elif target=='title':px,py=x+w//2,y+14
  elif target=='modal':
   px,py=max(x+8,408)+10,max(y+36,236)+2
   trace+='[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=0 first=0 rows=1\n[launcher.text.row] index=0 row=0 name=Open Spindle status=READY\n'
  pointer=len(trace);rx=round(px*32767/(resolution[0]-1));ry=round(py*32767/(resolution[1]-1));sx=rx*(resolution[0]-1)//32767;sy=ry*(resolution[1]-1)//32767
  trace+=f'[shell.abs.normalize] raw_x={rx} raw_y={ry} sx={sx} sy={sy} accepted=1 reason=ok\n';start=len(trace);anchor=before
  for delta in reports or[wheel]:
   packed=rx|(ry<<16)|(1<<32)|((delta&255)<<40);trace+=f'[sexusb.wheel.device] slot=1 bytes=6 absolute=1 delta={delta} packed={packed} up_positive=1\n[sexinput.wheel.received] caller=5 task=5 absolute=1 delta={delta} packed={packed} up_positive=1\n'
   normalized=f'[sexinput.wheel.normalized] delta={delta} word={delta&0xffffffffffffffff} reserved=0 class=5 admitted=1 up_positive=1\n'
   if target in('body','pending'):
    trace+=f'[shell.browser.wheel.queued] pd=15 task=15 route=43 wheel={delta} x={sx} y={sy} accepted=1 capacity=32\n';delivered=f'[shell.browser.wheel.delivered] pd=15 task=15 route=43 wheel={delta} accepted=1 status=0\n'
    if target=='body':
     current=gate.reader_frame(gate.READER_BODY,w,h,self.contract,anchor);after,changed=gate.wheel_move(current,delta);app=f'[browser.reader.wheel] wheel={delta} before={anchor} anchor={after} changed={int(changed)} viewport=1 retained_body_only=1\n';anchor=after
    else:app=''
    trace+=app+delivered+normalized if post else delivered+normalized+app
   else:trace+=f'[shell.browser.wheel.rejected] reason=target wheel={delta} x={sx} y={sy} effects=0\n'+normalized
  action=dict(name='test',wheel=wheel,target=target,before=before,after=anchor,x=px,y=py,pointer_start=pointer,serial_start=start,serial_qmp_return=start,serial_end=len(trace),host_start=1.,host_qmp_return=1.1,host_end=1.2,transport='input-send-event',events=gate.wheel_events(wheel))
  return trace,action
 def check(self,trace,action,resolution=(1280,800)):return gate.inspect_wheel_action(trace,action,gate.READER_BODY,self.contract,resolution)
 def mutate(self,trace,action,old,new):
  prefix=trace[:action['serial_start']];segment=trace[action['serial_start']:];self.assertIn(old,segment);changed=prefix+segment.replace(old,new);row=dict(action,serial_end=len(changed));return changed,row
 def test_source_binds_actual_usb_input_shell_reader_and_exact_task_abi(self):
  for name in('servers/sexusb/src/main.rs','servers/sexinput/src/main.rs','servers/silk-shell/src/main.rs','crates/sex-pdx/src/app_scroll.rs','apps/kaleidoscope/src/document_reader.rs'):
   self.assertEqual(self.contract['source_sha256'][name],hashlib.sha256((ROOT/name).read_bytes()).hexdigest())
 def test_serial_character_offsets_normalize_crlf_and_preserve_unicode_prefix_hash(self):
  import tempfile
  trace,action=self.fixture();prefix='[fixture] café € 🙂\n';expected=prefix+trace;row=dict(action)
  for key in('pointer_start','serial_start','serial_qmp_return','serial_end'):row[key]+=len(prefix)
  raw=expected.replace('\n','\r\n').encode('utf-8')
  with tempfile.TemporaryDirectory()as d:
   path=Path(d)/'serial.log';path.write_bytes(raw);normalized=gate.serial_text(path)
   self.assertEqual(normalized,expected);self.assertGreater(len(raw),len(normalized));self.assertEqual(path.read_bytes(),raw)
   self.assertTrue(self.check(normalized,row)['ok'])
   frame=dict(serial_end=row['serial_end'],serial_sha256=hashlib.sha256(expected.encode()).hexdigest(),requested_anchor=None)
   self.assertEqual(gate.frame_prefix(normalized,frame),expected)
   # The former byte-decoding replay both shifts action windows and changes
   # prefix hashes even though the original raw log is unchanged.
   with self.assertRaises(ValueError):self.check(raw.decode('utf-8'),row)
   with self.assertRaises(ValueError):gate.frame_prefix(raw.decode('utf-8'),frame)
 def test_replay_reads_normalized_character_windows_but_hashes_original_raw_serial(self):
  import json,tempfile
  class ReachedWindowCheck(Exception):pass
  trace,action=self.fixture();prefix='[fixture] café € 🙂\n';expected=prefix+trace;raw=expected.replace('\n','\r\n').encode()
  def check(actual,*args):
   self.assertEqual(actual,expected);raise ReachedWindowCheck()
  with tempfile.TemporaryDirectory()as d:
   root=Path(d);iso=root/'tested.iso';binding=root/'source-binding.json';iso.write_bytes(b'fixture');binding.write_text('{}')
   files={'serial.log':raw,'wire.pcap':b'fixture','endpoint-exchanges.json':b'{}','dns-endpoint-exchanges.json':b'{}'}
   for name,data in files.items():(root/name).write_bytes(data)
   manifest=dict(wheel=True,binding={},mode='reader',pixel_source_sha256={},command=['-cdrom',str(iso.resolve()),'nec-usb-xhci,id=xhci','usb-tablet,bus=xhci.0'],resolution='1280x800');(root/'manifest.json').write_text(json.dumps(manifest))
   evidence=dict(serial_offset_units=gate.WHEEL_SERIAL_UNITS,serial_prefix_hash_encoding='UTF-8',raw_sha256={name:hashlib.sha256(data).hexdigest()for name,data in files.items()},reader_frames=[],status_frames=[],other_frames=[],actions=[]);path=root/'wheel-evidence.json';path.write_text(json.dumps(evidence))
   with patch('network_contention_gate.frozen_binding',return_value={}),patch.object(gate,'load_wheel_contract',return_value={'source_sha256':{}}),patch.object(gate,'inspect_wheel',side_effect=check)as windows:
    with self.assertRaises(ReachedWindowCheck):gate.verify_wheel_evidence(root,iso,binding,root)
    self.assertEqual(windows.call_count,1)
    (root/'serial.log').write_bytes(raw+b'\r\n')
    with self.assertRaisesRegex(ValueError,'raw file differs'):gate.verify_wheel_evidence(root,iso,binding,root)
    self.assertEqual(windows.call_count,1)
    (root/'serial.log').write_bytes(raw);evidence['serial_offset_units']='raw bytes';path.write_text(json.dumps(evidence))
    with self.assertRaisesRegex(ValueError,'offset/hash convention'):gate.verify_wheel_evidence(root,iso,binding,root)
    self.assertEqual(windows.call_count,1)
 def test_real_qmp_wheel_buttons_have_no_fake_key_or_relative_axis(self):
  for value in(-3,-1,1,3):
   events=gate.wheel_events(value);self.assertEqual(len(events),2*abs(value));self.assertTrue(all(e['type']=='btn'and e['data']['button']==('wheel-up'if value>0 else'wheel-down')for e in events));self.assertEqual([e['data']['down']for e in events],[True,False]*abs(value))
  for value in(0,4,-4,128,None):
   with self.assertRaises(ValueError):gate.wheel_events(value)
 def test_three_visual_rows_and_both_endpoint_clamps(self):
  for resolution in((1280,800),(1024,768)):
   trace,action=self.fixture(1,resolution=resolution);result=self.check(trace,action,resolution);self.assertEqual(result['anchor'],0);self.assertFalse(result['events'][0]['changed'])
   trace,action=self.fixture(-1,resolution=resolution);result=self.check(trace,action,resolution);tile,_=gate.wheel_viewport(trace);frame=gate.reader_frame(gate.READER_BODY,tile['w'],tile['h'],self.contract);self.assertEqual(result['anchor'],frame['starts'][3])
   end=gate.reader_move(frame,'end');trace,action=self.fixture(-1,before=end,resolution=resolution);result=self.check(trace,action,resolution);self.assertEqual(result['anchor'],end);self.assertFalse(result['events'][0]['changed'])
 def test_post_call_shell_and_input_receipts_may_follow_app(self):
  for post in(True,False):
   trace,action=self.fixture(post=post);result=self.check(trace,action);self.assertEqual(result['events'][0]['callee_before_shell_return'],post);self.assertEqual(result['events'][0]['callee_before_input_return'],post)
 def test_actual_report_coalescing_and_separate_notches_have_same_exact_result(self):
  answers=[]
  for reports in([-3],[-1,-1,-1],[-2,-1]):
   trace,action=self.fixture(-3,reports=reports);result=self.check(trace,action);self.assertEqual(result['reports'],len(reports));answers.append(result['anchor'])
  self.assertEqual(len(set(answers)),1)
 def test_missing_each_actual_stage_is_rejected(self):
  trace,action=self.fixture()
  for kind in('device','received','normalized','queued','delivered','reader'):
   changed,row=self.mutate(trace,action,'['+gate.WHEEL_TAGS[kind]+']','[absent.receipt]')
   with self.assertRaises(ValueError,msg=kind):self.check(changed,row)
 def test_duplicate_stage_cannot_fabricate_a_second_notch(self):
  trace,action=self.fixture()
  for kind in('device','received','normalized','queued','delivered','reader'):
   line=next(l for l in trace.splitlines(True)if l.startswith('['+gate.WHEEL_TAGS[kind]+']'));changed,row=self.mutate(trace,action,line,line*2)
   with self.assertRaises(ValueError,msg=kind):self.check(changed,row)
 def test_wrong_direction_alias_word_reserved_class_or_sender_fail(self):
  trace,action=self.fixture()
  mutations=[('absolute=1 delta=-1','absolute=0 delta=-1'),('caller=5 task=5','caller=5 task=6'),('caller=5 task=5','caller=4 task=4'),('word=18446744073709551615','word=255'),('reserved=0 class=5','reserved=1 class=5'),('reserved=0 class=5','reserved=0 class=4'),('wheel=-1 x=','wheel=1 x=')]
  for old,new in mutations:
   changed,row=self.mutate(trace,action,old,new)
   with self.assertRaises(ValueError,msg=old):self.check(changed,row)
 def test_wrong_target_task_route_coordinates_capacity_and_app_math_fail(self):
  trace,action=self.fixture()
  for old,new in [('pd=15 task=15','pd=15 task=16'),('route=43','route=44'),('capacity=32','capacity=31'),('capacity=32','capacity=320'),('accepted=1 status=0','accepted=1 status=1'),('before=0 anchor=','before=1 anchor='),('changed=1','changed=0'),('viewport=1 retained','viewport=2 retained'),('bytes=6 absolute','bytes=5 absolute')]:
   changed,row=self.mutate(trace,action,old,new)
   with self.assertRaises(ValueError,msg=old):self.check(changed,row)
 def test_source_position_race_and_raw_coordinate_change_fail(self):
  trace,action=self.fixture();row=dict(action,before=1)
  with self.assertRaises(ValueError):self.check(trace,row)
  changed,row=self.mutate(trace,action,'packed=','packed=1')
  with self.assertRaises(ValueError):self.check(changed,row)
 def test_device_before_input_and_queue_before_app_are_real_causal_edges(self):
  trace,action=self.fixture()
  for a,b in[('device','received'),('queued','reader')]:
   first=next(l for l in trace.splitlines(True)if l.startswith('['+gate.WHEEL_TAGS[a]+']'));second=next(l for l in trace.splitlines(True)if l.startswith('['+gate.WHEEL_TAGS[b]+']'));changed,row=self.mutate(trace,action,first,'');changed=changed.replace(second,second+first);row['serial_end']=len(changed)
   with self.assertRaises(ValueError):self.check(changed,row)
 def test_outside_title_and_real_overlap_require_rejection_without_app_effects(self):
  for target in('outside','title','modal','closed'):
   trace,action=self.fixture(target=target);self.assertTrue(self.check(trace,action)['ok']);changed,row=self.mutate(trace,action,'[shell.browser.wheel.rejected]','[absent.rejection]')
   with self.assertRaises(ValueError):self.check(changed,row)
   changed=trace+'[browser.reader.wheel] wheel=-1 before=0 anchor=47 changed=1 viewport=1 retained_body_only=1\n';row=dict(action,serial_end=len(changed))
   with self.assertRaises(ValueError):self.check(changed,row)
 def test_rejection_target_is_derived_from_actual_pointer_and_open_overlay(self):
  trace,action=self.fixture(target='outside');row=dict(action,target='body')
  with self.assertRaises(ValueError):self.check(trace,row)
  trace,action=self.fixture(target='modal');changed=trace[:action['serial_start']]+'[command_palette.close]\n'+trace[action['serial_start']:];row=dict(action,serial_start=action['serial_start']+24,serial_qmp_return=action['serial_start']+24,serial_end=len(changed))
  with self.assertRaises((ValueError,AssertionError)):self.check(changed,row)
 def test_pending_real_receiving_has_held_delivery_but_no_reader_effect(self):
  for reports in([-3],[-1,-1,-1]):
   trace,action=self.fixture(-3,target='pending',reports=reports);result=self.check(trace,action);self.assertEqual(result['anchor'],0);self.assertEqual(result['events'],[])
   changed=trace+'[browser.reader.layout] forbidden\n';row=dict(action,serial_end=len(changed))
   with self.assertRaises(ValueError):self.check(changed,row)
   changed=trace.replace('state_before=Receiving','state_before=Complete')
   with self.assertRaises(ValueError):self.check(changed,action)
 def test_request_receipts_and_pointer_must_be_inside_actual_qmp_window(self):
  trace,action=self.fixture()
  for row in(dict(action,serial_qmp_return=action['serial_end']+1),dict(action,serial_start=action['serial_end']),dict(action,pointer_start=action['serial_start']),dict(action,host_qmp_return=0),dict(action,transport='synthetic'),dict(action,events=[])):
   with self.assertRaises(ValueError):self.check(trace,row)
 def test_missing_or_foreign_pointer_normalization_fails(self):
  trace,action=self.fixture()
  for changed in(trace.replace('[shell.abs.normalize]','[absent.normalize]'),trace.replace('reason=ok','reason=edge_before_ready')):
   with self.assertRaises(ValueError):self.check(changed,action)
 def test_whole_reader_glyph_oracle_remains_independent_of_wheel_receipts(self):
  model=reader_fixture.ReaderPixels();model.contract=self.contract;frame=gate.reader_frame(gate.READER_BODY,624,706,self.contract);anchor=gate.wheel_move(frame,-1)[0];moved=gate.reader_frame(gate.READER_BODY,624,706,self.contract,anchor);trace=model.trace(moved,position=True);image=model.screen(moved)
  self.assertTrue(gate.verify_reader_pixels(trace,image,gate.READER_BODY,self.contract,anchor)['ok']);wrong=copy.deepcopy(moved);wrong['lines'][2]=b'X'+wrong['lines'][2][1:];self.assertFalse(gate.verify_reader_pixels(trace,model.screen(wrong),gate.READER_BODY,self.contract,anchor)['ok'])

 def test_exact_pending_plan_rejects_extra_admission_egress_and_reader(self):
  first,a=self.fixture(-3,target='pending');a['name']='pending-burst';second,b=self.fixture(-1,target='closed');b['name']='closed'
  offset=len(first)
  for key in('pointer_start','serial_start','serial_qmp_return','serial_end'):b[key]+=offset
  trace=first+second;self.assertTrue(gate.inspect_wheel(trace,[a,b],gate.READER_BODY,self.contract,(1280,800),True)['ok'])
  for tail in('[net.job.resolved.admitted] forged\n','[net.dns.egress.submit] forged\n','[browser.reader.layout] forged\n','[net.job.egress.submit] ticket=3\n'):
   with self.assertRaises(ValueError):gate.inspect_wheel(trace+tail,[a,b],gate.READER_BODY,self.contract,(1280,800),True)
  with self.assertRaises(ValueError):gate.inspect_wheel(trace,[b,a],gate.READER_BODY,self.contract,(1280,800),True)
 def test_finite_pixel_phases_cannot_reuse_stale_or_wrong_anchor_capture(self):
  actions=[];frames=[dict(screenshot='browser-reader-initial.ppm',requested_anchor=0,serial_end=1)]
  for index,(name,delta,target)in enumerate(gate.WHEEL_PLAN):
   before=0 if index in(0,1,3,8)else 30 if index==2 else 100
   after=30 if name=='down'else 0 if name in('top','up','closed')else 100
   row=dict(name=name,wheel=delta,target=target,before=before,after=after,pointer_start=50+100*index,serial_end=100+100*index);actions.append(row);filename='browser-wheel-'+('end-clamp'if name=='end'else name)+'.ppm';frames.append(dict(screenshot=filename,requested_anchor=after if target in('body','outside','title')else None,serial_end=row['serial_end']+10))
  frames+=[dict(screenshot='browser-wheel-end-entry.ppm',requested_anchor=actions[4]['before'],serial_end=440),dict(screenshot='browser-wheel-modal-erased.ppm',requested_anchor=actions[7]['after'],serial_end=820,launcher_erasure=True)]
  self.assertTrue(gate.wheel_frame_phases(actions,frames))
  for change in('stale','anchor','erasure','missing'):
   bad=copy.deepcopy(frames)
   if change=='stale':bad[2]['serial_end']=actions[1]['serial_end']-1
   elif change=='anchor':bad[2]['requested_anchor']=0
   elif change=='erasure':bad[-1]['launcher_erasure']=False
   else:bad=bad[1:]
   with self.assertRaises(ValueError):gate.wheel_frame_phases(actions,bad)
 def test_reader_capture_retries_erasure_on_the_same_snapshot_without_input(self):
  import tempfile
  source='complete source snapshot';events=[];clock=[0]
  class Q:
   def cmd(self,name,args):
    self_test.assertEqual(name,'screendump');Path(args['filename']).write_bytes(b'P6\n1 1\n255\n\0\0\0');events.append('capture')
  def extra(prefix,image):
   self.assertEqual(prefix,source);events.append('erasure')
   if events.count('erasure')==1:raise ValueError('old launcher still painted')
  self_test=self
  def now():clock[0]+=.01;return clock[0]
  with tempfile.TemporaryDirectory()as d,patch.object(gate,'verify_reader_pixels',side_effect=lambda *args:{'ok':True,'errors':[]}),patch.object(gate.time,'monotonic',side_effect=now),patch.object(gate.time,'sleep',side_effect=lambda _:None):
   result=gate.wait_reader_pixels(Q(),lambda:source,Path(d),'actual',b'',{'source_sha256':{}},0,lambda:None,extra_check=extra)
  self.assertTrue(result['ok']);self.assertEqual(events,['capture','erasure','capture','erasure']);self.assertEqual(result['serial_sha256'],hashlib.sha256(source.encode()).hexdigest())

 def test_check_cli_uses_recorded_mode_and_needs_no_disk_or_runtime(self):
  import contextlib,io,tempfile
  with tempfile.TemporaryDirectory()as d:
   root=Path(d);iso=root/'tested.iso';binding=root/'binding.json';iso.write_bytes(b'fixture');binding.write_text('{}');argv=['http_content_gate.py','--wheel','--check','--iso',str(iso),'--binding',str(binding),'--out',str(root)]
   with patch.object(sys,'argv',argv),patch.object(gate,'verify_wheel_evidence',return_value={'ok':True})as replay,patch.object(gate.subprocess,'Popen',side_effect=AssertionError('read-only check')),contextlib.redirect_stdout(io.StringIO())as output:gate.main()
   self.assertEqual(replay.call_count,1);self.assertIn('"ok": true',output.getvalue())
 def test_wheel_cannot_start_without_frozen_binding_or_in_other_modes(self):
  import contextlib,io
  for argv in (['http_content_gate.py','--wheel','--iso','unused','--out','unused','--mode','reader'],['http_content_gate.py','--wheel','--iso','unused','--out','unused','--mode','echo','--binding','unused']):
   with patch.object(sys,'argv',argv),patch.object(gate.subprocess,'Popen',side_effect=AssertionError('no runtime')),contextlib.redirect_stderr(io.StringIO()),self.assertRaises(SystemExit)as error:gate.main()
   self.assertEqual(error.exception.code,2)

if __name__=='__main__':unittest.main()
