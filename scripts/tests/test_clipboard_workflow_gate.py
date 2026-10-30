"""Synthetic negative controls for qualification logic; no native claim."""
import copy,hashlib,json,struct,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import clipboard_workflow_gate as gate
import http_content_gate as http
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

class ClipboardWorkflow(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=gate.source_contract(ROOT)
 def frame(self,selection=(3,5),anchor=0,w=624,h=706):return gate.selected_frame(gate.BODY,w,h,self.c,anchor,selection,self.c['selection_label'])
 def trace(self,f,w=624,h=706,y=70):
  return (f'[shell.document.viewport] target=15 task=15 surface=300 sequence=1 x=50 y={y} w={w} h={h} enqueued=1\n'
   f'[browser.viewport] target=15 task=15 sequence=1 w={w} h={h} origin=held_shell\n'
   '[browser.http.released] ticket=2 view=Ready private_job=0\n[browser.http.document] ticket=2 path=/welcome status=200 private_bytes=12288 positive_release=1\n'
   f'[browser.reader.layout] first={f["first"]} end={f["end"]} body_rows={f["body_rows"]} rows={f["rows"]} cols={f["cols"]} total_lines={f["total_lines"]} source_bytes=12288 viewport=1 origin=validated_body tls=0 pixels=unconfirmed\n'
   f'[browser.reader.position] anchor={f["anchor"]} viewport=1 tls=0 retained_body_only=1\n'
   f'[browser.view.applied] bytes={f["bytes"]} http=258 body_bytes=12288 pixels=unconfirmed\n')
 def screen(self,f,highlight=True,base_y=70):
  w,h=1280,800;pixels=bytearray(w*h*3);scale=f['scale'];a,b=f['selection'];fg=self.c['color'].to_bytes(3,'big');bg=self.c['selection_color'].to_bytes(3,'big')
  for row,line in enumerate(f['lines']):
   for col,byte in enumerate(line):
    for gy in range(9):
     for gx in range(6):
      on=gy<7 and gx<5 and bool(glyph(self.c['font'],chr(byte))[gy]&(1<<(4-gx)))
      selected=highlight and a<=row*(f['cols']+1)+col<b
      color=fg if on else bg if selected else b'\0\0\0'
      for dy in range(scale):
       for dx in range(scale):
        x=50+self.c['inset']+(col*6+gx)*scale+dx;y=base_y+self.c['top']+(row*9+gy)*scale+dy
        pixels[(y*w+x)*3:(y*w+x)*3+3]=color
  return b'P6\n1280 800\n255\n'+pixels
 def check(self,f,image=None,selection=(3,5),trace=None):return gate.verify_browser(self.trace(f)if trace is None else trace,self.screen(f)if image is None else image,gate.BODY,self.c,f['anchor'],selection,'Selected',(1280,800))
 def test_actual_source_closure_and_reverse_mapping(self):
  for path in('crates/sex-pdx/src/clipboard_client.rs','servers/quil/src/document_exchange.rs','scripts/clipboard_workflow_gate.py'):
   self.assertEqual(self.c['source_sha256'][path],gate.sha((ROOT/path).read_bytes()))
  self.assertEqual(self.frame((5,3))['selection'],self.frame((3,5))['selection']);self.assertTrue(self.check(self.frame())['ok'])
 def test_whole_body_highlight_includes_more_than_256_source_bytes(self):
  f=self.frame((0,12288));r=self.check(f,selection=(0,12288));self.assertTrue(r['ok'],r);self.assertGreater(r['highlight_pixels'],1000)
 def test_missing_highlight_wrong_range_and_chrome_highlight_fail(self):
  f=self.frame();self.assertFalse(self.check(f,self.screen(f,False))['ok']);self.assertFalse(self.check(f,selection=(4,6))['ok'])
  wrong=copy.deepcopy(f);wrong['selection']=(0,2);self.assertFalse(self.check(f,self.screen(wrong))['ok'])
 def test_stale_text_missing_applied_or_position_cannot_pass(self):
  f=self.frame();t=self.trace(f)
  for bad in(t.replace('private_bytes=12288','private_bytes=256'),t.replace('[browser.view.applied]','[other]'),t.replace('anchor=0 viewport','anchor=47 viewport')):
   self.assertFalse(self.check(f,trace=bad)['ok'])
 def test_selection_reflow_uses_original_source_cells_and_trimmed_tail_has_no_invented_cells(self):
  a=self.frame((40,70),w=624);b=self.frame((40,70),w=142)
  self.assertNotEqual(a['selection'],b['selection']);self.assertEqual(gate.selected_frame(b'one\n   ',624,706,self.c,0,(4,7),b'')['selection'],(0,0))
  self.assertEqual(gate.selected_frame(b'one\ntwo',624,706,self.c,0,(3,4),b'')['selection'],(0,0))
 def test_raw_prefix_byte_boundary_and_hash_are_mandatory(self):
  raw=b'hello\nworld\n';row=dict(serial_bytes=6,serial_sha256=gate.sha(raw[:6]));self.assertEqual(gate.capture_prefix(raw,row),'hello\n')
  for bad in(dict(row,serial_bytes=7),dict(row,serial_sha256='0'*64),dict(row,serial_bytes=100),{}):
   with self.assertRaises(ValueError):gate.capture_prefix(raw,bad)
 def join_trace(self):
  h=hex(gate.fnv(gate.BODY));return (
   f'[browser.clipboard.copy] ticket=1 generation=1 revision=0 start=0 end=12288 bytes=12288 hash={h} accepted=1\n'
   '[shell.clipboard.request] pd=15 task=15 ticket=1 opcode=0x281 result=Ok(())\n[shell.clipboard.request] pd=15 task=15 ticket=1 opcode=0x28a result=Ok(())\n'
   f'[browser.clipboard.published] ticket=1 bytes=12288 generation=1 hash={h} positive_ack=1\n'
   '[shell.app.switch] old=300 new=201 live=1 shortcut=alt_tab\n'
   '[quil.selection] generation=1 revision=4 bytes=44 anchor=0 caret=44\n[quil.clipboard.intent] ticket=2 action=paste generation=1 revision=4 start=0 end=44\n'
   '[shell.clipboard.request] pd=9 task=9 ticket=2 opcode=0x283 result=Ok(())\n[shell.clipboard.request] pd=9 task=9 ticket=2 opcode=0x285 result=Ok(())\n'
   f'[quil.clipboard.paste] ticket=2 bytes=12288 generation=1 hash={h} applied=1 atomic=1\n')
 def test_exact_transfer_requires_real_subjects_matching_publication_and_atomic_selection(self):
  t=self.join_trace();j=gate.clipboard_join(t);self.assertEqual(j['bytes'],12288);self.assertEqual(j['consumer']['replaced_bytes'],44)
  for bad in(t.replace('pd=9 task=9','pd=9 task=19'),t.replace('end=12288 bytes=12288','end=256 bytes=256'),t.replace('caret=44','caret=43'),t.replace('applied=1 atomic=1','applied=0 atomic=1'),t+t):
   with self.assertRaises(ValueError):gate.clipboard_join(bad)
 def test_forged_shell_identity_source_binding_is_rejected(self):
  original=Path.read_text
  def read(path,*args,**kwargs):
   value=original(path,*args,**kwargs)
   return value.replace('if sender != SHELL','if false')if path.name=='clipboard_client.rs'else value
  with patch.object(Path,'read_text',read):
   with self.assertRaises(ValueError):gate.source_contract(ROOT)

 def test_independent_raw_tcp_capture_requires_complete_exact_nontruncated_http_body(self):
  import test_tls_exchange_gate as fixture
  response=http.wire_response('stream',gate.BODY,12288)
  def evidence(payload):
   records=[]
   def flow(guest,request,reply):
    records.extend([fixture.packet(True,guest,46078,100,0,2),fixture.packet(False,46078,guest,500,101,0x12),fixture.packet(True,guest,46078,101,501,0x10),fixture.packet(True,guest,46078,101,501,0x18,request),fixture.packet(False,46078,guest,501,101+len(request),0x18,reply),fixture.packet(True,guest,46078,101+len(request),501+len(reply),0x11),fixture.packet(False,46078,guest,501+len(reply),102+len(request),0x11)])
   flow(48001,http.ECHO,http.ECHO)
   records.extend([fixture.packet(True,48001,46079,0,0,0,http.dns_query(1),True),fixture.packet(False,46079,48001,0,0,0,http.dns_response(1),True)])
   flow(48002,self.c['request'],payload)
   return struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,65535,1)+b''.join(struct.pack('<IIII',0,0,len(row),len(row))+row for row in records)
  peer=dict(rows=[dict(request_hex=http.ECHO.hex(),response_hex=http.ECHO.hex()),dict(request_hex=self.c['request'].hex(),response_hex=response.hex())],dns=[dict(ticket=1,response_hex=http.dns_response(1).hex())],attempts=[dict(ticket=1,kind='answer',response_hex=http.dns_response(1).hex(),sent_bytes=len(http.dns_response(1)))])
  self.assertTrue(gate.wire_check(evidence(response),peer)['ok'])
  for bad in(response[:-1],response[:-1]+b'X',response[:256]):
   with self.assertRaises(ValueError):gate.wire_check(evidence(bad),peer)
 def test_selected_render_outbox_source_cannot_silently_discard_selection(self):
  original=Path.read_text
  def read(path,*args,**kwargs):
   value=original(path,*args,**kwargs)
   return value.replace('metadata,visible_selection)','metadata,(0,0))')if str(path).endswith('apps/kaleidoscope/src/main.rs')else value
  with patch.object(Path,'read_text',read):
   with self.assertRaises(ValueError):http.load_pixel_contract(ROOT)

 def negative_trace(self):
  h=hex(gate.fnv(gate.BODY));return self.join_trace()+(
   '[quil.selection] generation=1 revision=5 bytes=12288 anchor=12288 caret=12288\n[quil.clipboard.intent] ticket=3 action=paste generation=1 revision=5 start=12288 end=12288\n'
   '[shell.clipboard.request] pd=9 task=9 ticket=3 opcode=0x283 result=Ok(())\n[shell.clipboard.request] pd=9 task=9 ticket=3 opcode=0x285 result=Ok(())\n'
   f'[quil.clipboard.paste] ticket=3 bytes=12288 generation=1 hash={h} applied=0 atomic=1 reason=capacity\n'
   '[shell.app.close.request] target=15 task=15 surface=300\n[browser.import.closed] target=15 task=15 pending=0 accepted=0 terminal=1 owned_storage=erased\n'
   '[shell.app.cleanup.ack] target=15 task=15 surface=300 ok=1\n'
   +''.join(f'[app.service.ack] target=15 task=15 holder={role+1} role={role} ok=1\n'for role in(1,2,4,8,16))+
   '[net.dns.records.retired] target=15 task=15 remaining=0 before_cleanup_ack=1\n[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1\n'
   '[sext.app.pool.ready] target=15 task=15 bytes=565248 zeroed=565248 workspace_records=0 reused=1 zero_owner=sext ok=1\n'
   '[quil.clipboard.intent] ticket=4 action=paste generation=1 revision=5 start=12288 end=12288\n[shell.clipboard.request] pd=9 task=9 ticket=4 opcode=0x283 result=Ok(())\n[quil.clipboard.stopped] ticket=4 state=Empty payload_erased=1\n')
 def test_negative_native_joins_require_capacity_exact_eof_and_complete_close_before_empty(self):
  trace=self.negative_trace();positive=gate.clipboard_join(trace);result=gate.negative_joins(trace,positive)
  self.assertEqual(result['capacity']['ticket'],3);self.assertEqual(result['producer_close']['zeroed_bytes'],565248);self.assertEqual(result['empty']['ticket'],4)
  for bad in(trace.replace('start=12288 end=12288','start=0 end=12288',1),trace.replace('zeroed=565248','zeroed=1'),trace.replace('state=Empty','state=Cancelled'),trace.replace('pd=9 task=9 ticket=4','pd=9 task=19 ticket=4'),trace+'[quil.clipboard.paste] ticket=4 bytes=1 generation=1 hash=0x1 applied=1 atomic=1\n',trace.replace('[shell.app.close.request]','[quil.document.edit] revision=6 pending=0 bytes=12288\n[shell.app.close.request]')):
   with self.assertRaises(ValueError):gate.negative_joins(bad,gate.clipboard_join(bad))
 def test_negative_snapshot_allows_only_status_change_and_rejects_any_body_caret_or_viewport_change(self):
  original=dict(x=0,y=100,w=624,h=500,top=36,cols=47,rows=10,cursor=12288,view=11900,text='UNTITLED * PASTED\nNEW OPEN SAVE\nlast source bytes')
  rejected=dict(original,text='UNTITLED * PASTE TOO LARGE\nNEW OPEN SAVE\nlast source bytes')
  self.assertTrue(gate.unchanged_quil(original,rejected)['ok'])
  for bad in(dict(rejected,cursor=12287),dict(rejected,w=623),dict(rejected,text=rejected['text']+'X')):
   with self.assertRaises(ValueError):gate.unchanged_quil(original,bad)

 def test_only_incomplete_trailing_utf8_scalar_is_trimmed_without_mutating_raw_evidence(self):
  head=b'[boot] normal\n'
  for text in('¢','→','😀'):
   encoded=text.encode('utf-8')
   for size in range(1,len(encoded)):
    raw=head+encoded[:size];original=bytes(raw)
    self.assertEqual(gate.complete_prefix(raw),head)
    self.assertEqual(raw,original)
    completed=raw+encoded[size:]+b' next\n'
    self.assertEqual(gate.complete_prefix(completed),head+encoded+b' next\n')
  self.assertEqual(gate.complete_prefix(b''),b'')
  self.assertEqual(gate.complete_prefix(head+'→'.encode()),head+'→'.encode())
 def test_malformed_utf8_in_middle_or_tail_remains_fatal(self):
  for raw in(b'valid\xfftail',b'valid\xff',b'valid\xe2(other)',b'valid\xf0\x80',b'bad\xed\xa0\x80tail',b'bad\xff'+bytes.fromhex('e286')):
   with self.assertRaises(UnicodeDecodeError):gate.complete_prefix(raw)
 def test_recorded_serial_hash_covers_exact_complete_trimmed_bytes_and_replay_stays_strict(self):
  raw=b'prefix '+bytes.fromhex('e286');prefix=gate.complete_prefix(raw)
  row=dict(serial_bytes=len(prefix),serial_sha256=gate.sha(prefix))
  self.assertEqual(gate.capture_prefix(raw+bytes.fromhex('92')+b' future',row),'prefix ')
  self.assertEqual(raw,b'prefix '+bytes.fromhex('e286'))
  partial=dict(serial_bytes=len(raw),serial_sha256=gate.sha(raw))
  with self.assertRaises(UnicodeDecodeError):gate.capture_prefix(raw,partial)
  with self.assertRaises(ValueError):gate.capture_prefix(b'changed'+raw,row)
 def test_decode_diagnostics_are_bounded_metadata_and_never_repr_whole_serial_file(self):
  raw=b'PRIVATE_EVIDENCE_'*10000+b'\xff'
  try:gate.complete_prefix(raw)
  except UnicodeDecodeError as exc:
   summary=gate.failure_summary(exc)
   self.assertIn('UnicodeDecodeError',summary);self.assertIn('start=',summary);self.assertIn('input_bytes=',summary)
   self.assertNotIn('PRIVATE_EVIDENCE',summary);self.assertLess(len(summary),220)
  else:self.fail('invalid serial must fail')
  self.assertLess(len(gate.failure_summary(ValueError('x'*10000))),560)

 def test_hardware_key_packets_release_every_modifier_without_send_key_hold_timer(self):
  class Q:
   def __init__(self):self.calls=[];self.down=set();self.strokes=[]
   def cmd(self,op,args):
    self.assert_op=op;self.calls.append((op,args))
    if op!='input-send-event':raise AssertionError('deferred QMP key timer used')
    for event in args['events']:
     data=event['data'];code=data['key']['data']
     if data['down']:
      self.down.add(code)
      if code not in('ctrl','shift'):self.strokes.append((code,'shift'in self.down,'ctrl'in self.down))
     else:self.down.remove(code)
  q=Q()
  for keys in(('right',),('shift','right'),('shift','left'),('ctrl','shift','s'),('f11',)):
   gate.hardware_keys(q,keys);self.assertEqual(q.down,set());self.assertNotIn('hold-time',q.calls[-1][1])
  self.assertEqual(q.strokes,[('right',False,False),('right',True,False),('left',True,False),('s',True,True),('f11',False,False)])
  for keys in((),('shift','shift'),('ctrl','shift','alt','s')):
   with self.assertRaises(ValueError):gate.hardware_keys(q,keys)
 def test_browser_movement_waits_fresh_actual_admission_after_every_explicit_pulse(self):
  order=[];snapshot='old serial';offset=len(snapshot)
  class Q:
   def cmd(self,op,args):
    order.append(('hardware',op,args));self.pending=False
  q=Q()
  def text():order.append(('snapshot',));return snapshot
  def record(keys):order.append(('record',keys))
  def wait(pattern,start):
   self.assertEqual(start,offset);self.assertIn('slot=43 accepted=1',pattern);self.assertEqual(order[-1][0],'hardware');order.append(('admission',pattern,start));return 'actual admission'
  wanted=[('right',)]*5+[('shift','right')]*3+[('shift','left')]*5
  for keys in wanted:self.assertEqual(gate.browser_movement(q,keys,text,wait,record),'actual admission')
  self.assertEqual([row[0]for row in order],['snapshot','record','hardware','admission']*13)
  self.assertEqual([row[1]for row in order if row[0]=='record'],wanted)
  source=(ROOT/'scripts/clipboard_workflow_gate.py').read_text();self.assertNotIn('q.key(',source);self.assertNotIn('q.key_sequence(',source)

 def test_focus_helper_tab_issues_one_alt_tab_action_and_releases_before_fresh_focus_barrier(self):
  order=[];actions=[];stream='[shell.focus.set] id=300\n';calls=0
  class Q:
   def cmd(self,op,args):order.append(('hardware',op,args))
  q=Q()
  def text():return stream
  def wait(pattern,start):
   nonlocal stream,calls
   calls+=1;self.assertEqual(start,len(stream));self.assertEqual(actions,[('alt','tab')]);self.assertEqual(order[-1][0],'hardware')
   event=order[-1][2]['events'];self.assertEqual([(e['data']['key']['data'],e['data']['down'])for e in event],[('alt',True),('tab',True),('tab',False),('alt',False)])
   stream+='[shell.focus.set] id=201\n[shell.app.switch] old=300 new=201 live=1 shortcut=alt_tab\n';order.append(('focus_barrier',));return 'actual focus'
  def key(code,pattern):return gate.qualified_key(q,code,pattern,text,wait,actions.append)
  gate.focus_surface(201,text,key)
  self.assertEqual(calls,1);self.assertEqual(actions,[('alt','tab')]);self.assertEqual([row[0]for row in order],['hardware','focus_barrier'])
 def test_nonfocus_simple_key_remains_one_explicit_unmodified_packet(self):
  calls=[];actions=[]
  class Q:
   def cmd(self,op,args):calls.append((op,args))
  result=gate.qualified_key(Q(),'ret','positive',lambda:'old',lambda pattern,start:(pattern,start),actions.append)
  self.assertEqual(result,('positive',3));self.assertEqual(actions,[('ret',)])
  self.assertEqual([e['data']['key']['data']for e in calls[0][1]['events']],['ret','ret'])
 def test_cross_app_native_join_requires_live_alt_tab_between_copy_publication_and_consumer_intent(self):
  t=self.join_trace();self.assertGreater(gate.clipboard_join(t)['alt_tab_offset'],gate.clipboard_join(t)['published_offset'])
  marker='[shell.app.switch] old=300 new=201 live=1 shortcut=alt_tab\n'
  for bad in(t.replace(marker,''),t.replace('new=201 live=1 shortcut=alt_tab','new=201 live=0 shortcut=alt_tab'),t.replace('shortcut=alt_tab','shortcut=tab'),marker+t.replace(marker,'')):
   with self.assertRaises(ValueError):gate.clipboard_join(bad)

 def test_canonical_desktop_boundary_accepts_y50_and_rejects_y49(self):
  self.assertEqual(self.c['desktop_top'],50);f=self.frame();image=self.screen(f,base_y=50)
  good=self.check(f,image,trace=self.trace(f,y=50));self.assertTrue(good['ok'],good)
  bad=self.check(f,self.screen(f,base_y=49),trace=self.trace(f,y=49));self.assertFalse(bad['ok']);self.assertIn('outside framebuffer',bad['errors'][0])
 def test_zoom_barrier_requires_strictly_new_geometry_layout_position_and_applied(self):
  f=self.frame();old=self.trace(f);action='[shell.window.action] action=ZoomToggle frame=13 sid=300 ok=1\n'
  fresh=self.trace(f).replace('sequence=1','sequence=2').replace('viewport=1','viewport=2')
  trace=old+action+fresh;after=len(old);result=gate.viewport_barrier(trace,1,after,self.c)
  self.assertEqual(result['sequence'],2);self.assertLess(result['zoom_offset'],result['queued_offset']);self.assertLess(result['queued_offset'],result['notice_offset']);self.assertLess(result['notice_offset'],result['layout_offset']);self.assertLess(result['layout_offset'],result['applied_offset'])
  self.assertEqual(gate.browser_viewport_sequence(trace),2)
  for bad,previous,boundary in((old,1,0),(trace.replace(action,''),1,after),(trace,2,after),(trace,1,len(trace)),(old+action+fresh.replace('[browser.view.applied]','[other]'),1,after),(old+action+fresh.replace('viewport=2 origin=validated_body','viewport=1 origin=validated_body'),1,after),(old+action+fresh.replace('sequence=2 x=50','sequence=9 x=50'),1,after),(old+action+fresh.replace('[browser.reader.position]','[other.position]'),1,after)):
   with self.assertRaises(ValueError):gate.viewport_barrier(bad,previous,boundary,self.c)

 def test_filename_keyboard_is_preflighted_from_actual_unshifted_editor_source(self):
  source=(ROOT/'servers/quil/src/main.rs').read_text()
  self.assertEqual(gate.NAME,'clipnote');self.assertEqual(gate.name_keyboard(gate.NAME,source),tuple('clipnote'));self.assertEqual(self.c['name_keys'],tuple('clipnote'))
  for name in('clip-note','Clipnote','clip note','clip\nnote','é','x'*17,''):
   with self.assertRaises(ValueError):gate.name_keyboard(name,source)
  changed=source.replace("else { b'c' }","else { b'x' }")
  with self.assertRaises(ValueError):gate.name_keyboard('clipnote',changed)



class UTF8ClipboardWorkflow(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=gate.source_contract(ROOT,'utf8')
 def frame(self,selection=(6,13),w=624,h=706,body=None):
  return gate.selected_frame(gate.UTF8_BODY if body is None else body,w,h,self.c,0,selection,self.c['selection_label'])
 def trace(self,f,w=624,h=706):return ClipboardWorkflow.trace(self,f,w,h)
 def screen(self,f,resolution=(1280,800),highlight=True):
  width,height=resolution;pixels=bytearray(width*height*3);factor=f['scale'];a,b=f['selection'];fg=self.c['color'].to_bytes(3,'big');bg=self.c['selection_color'].to_bytes(3,'big')
  for row,line in enumerate(f['lines']):
   for col,(char,start,stop)in enumerate(gate.text_cells(line)):
    selected=highlight and a<=f['row_offsets'][row]+start<f['row_offsets'][row]+stop<=b
    bitmap=glyph(self.c['font'],char)
    for gy in range(9):
     for gx in range(6):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)));color=fg if on else bg if selected else b'\0\0\0'
      for dy in range(factor):
       for dx in range(factor):
        x=50+self.c['inset']+(col*6+gx)*factor+dx;y=70+self.c['top']+(row*9+gy)*factor+dy;pixels[(y*width+x)*3:(y*width+x)*3+3]=color
  return b'P6\n'+f'{width} {height}\n255\n'.encode()+pixels
 def check(self,f,image=None,selection=(6,13),w=624,h=706,resolution=(1280,800)):
  return gate.verify_browser(self.trace(f,w,h),self.screen(f,resolution)if image is None else image,gate.UTF8_BODY,self.c,0,selection,'Selected',resolution)
 def test_fixed_profile_is_exact_bounded_utf8_with_every_required_scalar_and_tail(self):
  body=gate.profile_body('utf8');self.assertEqual(len(body),12288);decoded=body.decode('utf8');self.assertEqual(len(body),len(gate.BODY));self.assertNotEqual(gate.sha(body),gate.sha(gate.BODY))
  for char in('é','€','🙂','漢','𐍈','\t','\n'):self.assertIn(char,decoded)
  self.assertTrue(body.endswith(b'UTF8 CLIPBOARD BYTE TAIL 105 END'));self.assertEqual(gate.reverse_selection(body),(6,13));self.assertEqual(gate.reverse_selection(gate.BODY),(3,5))
  splits=set()
  for char,start,end in gate.text_cells(body):
   if len(char.encode())>1 and start//16!=(end-1)//16:splits.add((len(char.encode()),16-start%16))
  self.assertTrue({(2,1),(3,1),(3,2),(4,1),(4,2),(4,3)}<=splits)
 def test_native_sized_both_profile_tiles_have_scalar_pixels_and_exact_byte_selection(self):
  for w,h,resolution in ((624,706,(1280,800)),(496,674,(1024,768))):
   f=self.frame(w=w,h=h);check=self.check(f,w=w,h=h,resolution=resolution);self.assertTrue(check['ok'],check);self.assertLessEqual(f['bytes'],512);self.assertEqual(f['bytes'],len(b'\n'.join(f['lines'])));self.assertEqual(f['selection'][1]-f['selection'][0],7)
   self.assertEqual(len(f['lines'][2].decode()),f['cols']);self.assertIn('Aé€🙂漢 ',f['lines'][2].decode());self.assertGreater(len(f['lines'][2]),f['cols'])
 def test_midscalar_selection_anchor_and_partial_byte_range_cannot_be_painted(self):
  for selection in ((2,3),(6,9),(7,13),(-1,1),(0,12289),(1.0,3)):
   with self.assertRaises(ValueError):self.frame(selection)
  for anchor in (2,4,7):
   with self.assertRaises(ValueError):gate.utf8_reader_frame(gate.UTF8_BODY,624,706,self.c,anchor)
  f=self.frame();self.assertFalse(self.check(f,selection=(6,9))['ok']);self.assertEqual(self.frame((13,6))['selection'],f['selection'])
 def test_changed_unicode_glyph_missing_highlight_or_wrong_rendered_byte_selection_fail(self):
  f=self.frame();changed=copy.deepcopy(f);changed['lines'][2]=changed['lines'][2].replace('é'.encode(),'ä'.encode())
  self.assertFalse(self.check(f,self.screen(changed))['ok']);self.assertFalse(self.check(f,self.screen(f,highlight=False))['ok'])
  changed=copy.deepcopy(f);changed['selection']=(f['selection'][0]+1,f['selection'][1]);self.assertFalse(self.check(f,self.screen(changed))['ok'])
  tab=self.frame((13,14));self.assertEqual(tab['selection'][1]-tab['selection'][0],1);self.assertTrue(self.check(tab,selection=(13,14))['ok'])
 def test_full_frame_budget_reserves_worst_serialized_row_and_footer_after_final_lf(self):
  body='🙂'.encode()*3072;f=gate.utf8_reader_frame(body,1280,750,self.c,0,(0,len(body)),b'Copied / exact bytes')
  self.assertEqual((f['cols'],f['body_rows'],f['rows']),(47,1,4));self.assertEqual(f['max_row_cost'],189);self.assertLessEqual(f['bytes'],512)
  joined=b'\n'.join(f['lines']);self.assertEqual(f['row_offsets'][-1],joined.rfind(b'\n')+1);self.assertTrue(joined[f['row_offsets'][-1]:].startswith(b'Copied / exact bytes'))
  self.assertLess(f['selection'][1],f['row_offsets'][-1]);self.assertEqual(len(f['lines'][2].decode()),47)
  # Final whitespace is retained in raw data but never invents visible cells.
  blank=gate.utf8_reader_frame('é\t\n   '.encode(),624,706,self.c,0,(2,7))
  self.assertEqual(blank['selection'],(0,0))
 def test_recorded_profile_sha_and_cli_request_cannot_select_a_different_fixture(self):
  manifest=dict(text_profile='utf8',body_sha256=gate.sha(gate.UTF8_BODY),body_bytes=12288)
  self.assertEqual(gate.manifest_profile(manifest),('utf8',gate.UTF8_BODY));self.assertEqual(gate.manifest_profile(manifest,'utf8')[0],'utf8')
  for bad in ({},dict(manifest,text_profile='ascii'),dict(manifest,body_sha256=gate.sha(gate.BODY)),dict(manifest,body_bytes=12287),dict(manifest,text_profile=[])):
   with self.assertRaises(ValueError):gate.manifest_profile(bad)
  with self.assertRaises(ValueError):gate.manifest_profile(manifest,'ascii')
  self.assertEqual(gate.source_contract(ROOT)['text_profile'],'ascii')
 def test_utf8_publication_capacity_and_cleanup_joins_bind_whole_raw_hash(self):
  source=ClipboardWorkflow();trace=source.negative_trace().replace(hex(gate.fnv(gate.BODY)),hex(gate.fnv(gate.UTF8_BODY)))
  positive=gate.clipboard_join(trace,gate.UTF8_BODY);self.assertEqual(positive['bytes'],12288);self.assertTrue(gate.negative_joins(trace,positive,gate.UTF8_BODY)['ok'])
  with self.assertRaises(ValueError):gate.clipboard_join(trace)
  with self.assertRaises(ValueError):gate.negative_joins(trace,positive)
 def test_exact_raw_utf8_journal_rejects_valid_different_bytes_even_when_hashes_are_updated(self):
  import test_https_import_gate as fixture
  before=fixture.disk_object(bytearray(4096*512),2,b'old-note',b'old');after=fixture.disk_object(bytearray(before),5,gate.NAME.encode(),gate.UTF8_BODY)
  doc=gate.disk_check(bytes(before),bytes(after),gate.UTF8_BODY);self.assertEqual(doc['bytes'],12288);self.assertTrue(gate.unchanged_document(bytes(after),bytes(after),doc,gate.UTF8_BODY)['ok'])
  wrong=gate.UTF8_BODY.replace('é'.encode(),'ä'.encode(),1);modified=fixture.disk_object(bytearray(before),5,gate.NAME.encode(),wrong)
  with self.assertRaises(AssertionError):gate.disk_check(bytes(before),bytes(modified),gate.UTF8_BODY)
  with self.assertRaises(AssertionError):gate.disk_check(bytes(before),bytes(after),gate.BODY)



 def test_utf8_wire_requires_exact_utf8_charset_and_exact_raw_bytes(self):
  import test_tls_exchange_gate as fixture
  response=gate.fixture_response(gate.UTF8_BODY);self.assertIn(b'charset=utf-8\r\n',response);self.assertIn(b'charset=us-ascii\r\n',gate.fixture_response(gate.BODY))
  def evidence(reply):
   records=[]
   def flow(guest,request,data):
    records.extend([fixture.packet(True,guest,46078,100,0,2),fixture.packet(False,46078,guest,500,101,0x12),fixture.packet(True,guest,46078,101,501,0x10),fixture.packet(True,guest,46078,101,501,0x18,request),fixture.packet(False,46078,guest,501,101+len(request),0x18,data),fixture.packet(True,guest,46078,101+len(request),501+len(data),0x11),fixture.packet(False,46078,guest,501+len(data),102+len(request),0x11)])
   flow(48001,http.ECHO,http.ECHO);records.extend([fixture.packet(True,48001,46079,0,0,0,http.dns_query(1),True),fixture.packet(False,46079,48001,0,0,0,http.dns_response(1),True)]);flow(48002,self.c['request'],reply)
   capture=struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,65535,1)+b''.join(struct.pack('<IIII',0,0,len(frame),len(frame))+frame for frame in records)
   peer=dict(rows=[dict(request_hex=http.ECHO.hex(),response_hex=http.ECHO.hex()),dict(request_hex=self.c['request'].hex(),response_hex=reply.hex())],dns=[dict(ticket=1,response_hex=http.dns_response(1).hex())],attempts=[dict(ticket=1,kind='answer',response_hex=http.dns_response(1).hex(),sent_bytes=len(http.dns_response(1)))])
   return capture,peer
  raw,peer=evidence(response);self.assertEqual(gate.wire_check(raw,peer,body=gate.UTF8_BODY)['body_sha256'],gate.sha(gate.UTF8_BODY))
  for bad in (response.replace(b'charset=utf-8',b'charset=us-ascii'),response.replace('é'.encode(),'ä'.encode(),1),response[:-1],response.replace('漢'.encode(),'€'.encode(),1)):
   raw,peer=evidence(bad)
   with self.assertRaises(ValueError):gate.wire_check(raw,peer,body=gate.UTF8_BODY)
 def test_check_cli_uses_recorded_profile_and_rejects_explicit_mismatch_before_replay(self):
  manifest=dict(text_profile='utf8',body_sha256=gate.sha(gate.UTF8_BODY),body_bytes=12288,binding={'same':'image'},final_disk_sha256='d',input_disk_sha256='d')
  with tempfile.TemporaryDirectory()as directory:
   out=Path(directory)
   for name in ('write','recovery'):(out/name).mkdir();(out/name/'manifest.json').write_text(json.dumps(manifest))
   with patch.object(sys,'argv',['gate','--check',str(out),'--text-profile','ascii']),patch.object(gate,'replay_boot')as replay:
    with self.assertRaises(ValueError):gate.main()
    replay.assert_not_called()
   with patch.object(sys,'argv',['gate','--check',str(out)]),patch.object(gate,'source_contract',return_value=self.c)as contract,patch.object(gate,'replay_boot',return_value={'ok':True})as replay,patch('builtins.print'):
    gate.main();self.assertEqual(contract.call_args.args[1],'utf8');self.assertEqual(replay.call_count,2)
   changed=dict(manifest,text_profile='ascii',body_sha256=gate.sha(gate.BODY));(out/'recovery/manifest.json').write_text(json.dumps(changed))
   with patch.object(sys,'argv',['gate','--check',str(out)]),patch.object(gate,'source_contract',return_value=self.c),patch.object(gate,'replay_boot',return_value={'ok':True}),patch('builtins.print'):
    with self.assertRaises(ValueError):gate.main()

if __name__=='__main__':unittest.main()
