"""Independent synthetic controls, never native qualification evidence."""
import copy,json,struct,sys,unittest
from pathlib import Path
from unittest import mock
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import quil_readability_gate as g
import test_https_import_gate as disk_fixture
ROOT=Path(__file__).resolve().parents[2]

def trace_fixture():
 text='[boot.app.resources] target=9 task=90 surface=156 private_pages=92 required=31 coordinator=3\n[command_palette.execute] cmd=1 name=Open Quil\n[launcher.action] target=9 task=90 frame=201 action=Focus from=0 to=0 visible=1 focused=1 activated=0 queued=0 accepted=1 ok=1 pixels=unconfirmed\n[quil.new.ok] bytes=0\n';actions=[]
 def append(line):
  nonlocal text
  text+=line+'\n'
 def toolbar(action,sequence,semantic):
  start=len(text);append('[silk-shell.pointer.recv] class=EV_BTN btn=1 pressed=true');append(f'[shell.document.pointer] target=9 action={action} status=0');append(f'[quil.ui.pointer] action={action} sequence={sequence}');append(semantic);append('[silk-shell.pointer.recv] class=EV_BTN btn=1 pressed=false');actions.append(dict(kind='toolbar',action=action,serial_start=start,serial_end=len(text)))
 toolbar(4,8,'[quil.name.mode] on=1 save_as=1')
 def save(body,generation,bank,revision):
  append(f'[files.document.durable] caller=9 object=5 identity=393217 generation={generation} bank={bank} bytes={len(body)} hash={hex(g.fnv(body))} barriers=3 readback=1')
  append(f'[quil.persist.save.ok] bytes={len(body)} hash={hex(g.fnv(body))} doc=5');append(f'[quil.document.saved] saved_revision={revision} revision={revision} dirty=0')
 save(g.BODY,1,0,14)
 toolbar(1,9,'[quil.new.ok] bytes=0');toolbar(2,10,'[shell.document.open] target=9 linen=200');append('[linen.quil.disk_doc.intent] object_id=4 path_id=5 ok=1');append('[launcher.action] target=9 task=90 frame=201 action=Focus from=0 to=0 visible=1 focused=1 activated=0 queued=0 accepted=1 ok=1 pixels=unconfirmed');append(f'[quil.persist.load.ok] bytes={len(g.BODY)} hash={hex(g.fnv(g.BODY))} doc=5');append('[command_palette.execute] cmd=1 name=Open Quil');append('[launcher.action] target=9 task=90 frame=201 action=Focus from=0 to=0 visible=1 focused=1 activated=0 queued=0 accepted=1 ok=1 pixels=unconfirmed')
 start=len(text);toolbar(3,11,'[quil.document.begin] kind=save object=5 revision=17 bytes=14');save(g.FINAL,2,1,17);actions[-1]['serial_end']=len(text)
 return text,actions

def raw_disk():
 disk=disk_fixture.disk_object(bytearray(4096*512),5,g.NAME.encode(),g.BODY);start=(2200+5*128)*512+32768;header=bytearray(512);header[:4]=b'QJ02';header[4]=5;struct.pack_into('<I',header,8,393217);struct.pack_into('<Q',header,16,2);struct.pack_into('<I',header,24,len(g.FINAL));struct.pack_into('<Q',header,32,g.fnv(g.FINAL));struct.pack_into('<Q',header,504,g.fnv(header[:504]));disk[start:start+512]=header;disk[start+512:start+512+len(g.FINAL)]=g.FINAL;return bytes(disk)

class ResponsiveLayout(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=g.source_contract(ROOT)
 def test_actual_sources_bind_eight_cell_labels_and_all_controls(self):
  self.assertEqual(self.c['cell'],8);self.assertEqual(self.c['toolbar_labels'],['New     ','Open    ','Save    ','Save As ']);self.assertEqual(len(self.c['commands']),5);self.assertEqual(self.c['name_keys'],tuple(g.NAME));self.assertGreater(len(self.c['source_sha256']),30)
 def test_both_real_profile_tiles_show_four_unambiguous_buttons(self):
  for w,h in ((624,706),(496,674)):
   l=g.layout(w,h,self.c);v=dict(w=w,h=h,x=8,y=86,**{k:l[k]for k in ('cols','rows','top')});self.assertEqual(l['per'],4)
   points=[g.toolbar_point(action,v,self.c)for action in (1,2,3,4)];self.assertEqual(len(set(points)),4)
   for index,(x,y)in enumerate(points):
    self.assertEqual((x-v['x']-8)//(6*l['scale'])//8,index);self.assertEqual((y-v['y']-l['top'])//(9*l['scale']),l['header'])
   self.assertEqual(points[3][0]-points[2][0],8*6*l['scale'])
 def test_reflow_two_one_and_hidden_toolbar_never_uses_old_five_cell_hit(self):
  for w,per in ((250,4),(210,2),(100,1)):
   l=g.layout(w,400,self.c);self.assertEqual(l['per'],per);v=dict(w=w,h=400,x=0,y=50,**{k:l[k]for k in ('cols','rows','top')});self.assertEqual(len(set(g.toolbar_point(a,v,self.c)for a in (1,2,3,4))),4)
  l=g.layout(624,26,self.c);v=dict(w=624,h=26,x=0,y=50,**{k:l[k]for k in ('cols','rows','top')})
  with self.assertRaises(ValueError):g.toolbar_point(1,v,self.c)
  v['h']=400
  with self.assertRaises(ValueError):g.toolbar_point(4,v,self.c)
 def make_view(self,stage):
  l=g.layout(624,706,self.c);header='readone SAVED'if stage in('returned','resaved','reopened','saved','commands')else'Name: readone#'if stage=='naming'else'UNTITLED READY'
  body=g.FINAL if stage in('returned','resaved')else b''if stage=='new-empty'else g.BODY
  lines=[header]+([('> 'if i==0 else'  ')+label for i,label in enumerate(self.c['commands'])]+self.c['command_hints']if stage=='commands'else[''.join(self.c['toolbar_labels']),self.c['normal_hint'],body.decode()+'#'])
  return dict(w=624,h=706,text='\n'.join(lines),**{k:l[k]for k in('cols','rows','top')})
 def test_complete_toolbar_name_body_and_help_are_mandatory(self):
  for stage in ('edited','naming','saved','new-empty','reopened','resaved','returned','commands'):g.chrome(self.make_view(stage),stage,self.c)
  v=self.make_view('returned')
  for old,new in [('Save As ','SaveAs  '),('Ctrl Shift S','Ctrl A'),('readable notez#','readable note#'),('readone SAVED','other SAVED')]:
   bad=dict(v,text=v['text'].replace(old,new))
   with self.assertRaises(ValueError):g.chrome(bad,'returned',self.c)
  v=self.make_view('commands')
  for text in(v['text'].replace('Ctrl A select all',''),v['text'].replace('> Inspect','  Inspect'),v['text'].replace('Save As  Ctrl Shift S','SaveAs')):
   with self.assertRaises(ValueError):g.chrome(dict(v,text=text),'commands',self.c)
 def test_real_whole_view_pixel_helper_rejects_header_hint_and_body_corruption(self):
  v=self.make_view('edited');w,h=1280,800;x,y=8,86;paint=77;data=bytearray(w*h*3)
  for row,line in enumerate(v['text'].splitlines()):
   for col,char in enumerate(line):
    for gy,bits in enumerate(g.launch.glyph(self.c['font'],char)):
     for gx in range(5):
      if bits&(1<<(4-gx)):
       for dy in range(2):
        for dx in range(2):
         px=x+8+(col*6+gx)*2+dx;py=y+v['top']+(row*9+gy)*2+dy;data[(py*w+px)*3:(py*w+px)*3+3]=b'\xe0\xf0\xff'
  text=v['text'].encode();trace=(f'[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=0 first=0 rows=1\n'
   '[launcher.text.row] index=0 row=0 name=Focus Quil status=RUNNING\n[command_palette.close]\n'
   '[linen.list.text] x=640 y=50 w=640 h=375 scale=2 text_color=0xf0f8ff rows=3 selected=1 row_y=72 row_h=36 committed_view=1\n[linen.list.label] id=1 row=0 text=first\n[linen.list.label] id=2 row=1 text=second\n[linen.list.label] id=3 row=2 text=third\n'
   f'[shell.document.viewport] target=9 task=90 surface=156 sequence=1 x={x} y={y} w={v["w"]} h={v["h"]} enqueued=1\n'
   f'[quil.ui.view] target=9 surface=156 paint={paint} w={v["w"]} h={v["h"]} top={v["top"]} cols={v["cols"]} rows={v["rows"]} cursor={len(g.BODY)} view=0\n'
   f'[quil.ui.text] paint={paint} off=0 hex={text.hex()}\n[quil.ui.admitted] paint={paint} bytes={len(text)} presentation=unconfirmed\n[quil.ui.applied] paint={paint} bytes={len(text)} pixels=unconfirmed\n')
  image=b'P6\n1280 800\n255\n'+data;g.verify_frame(trace,image,'edited',self.c,(w,h))
  header=len(image)-len(data)
  for row in range(4):
   changed=bytearray(image);py=y+v['top']+row*18;px=x+8
   offset=header+(py*w+px)*3;changed[offset:offset+3]=b'\xe0\xf0\xff'if changed[offset:offset+3]!=b'\xe0\xf0\xff'else b'\0\0\0'
   with self.assertRaises(ValueError):g.verify_frame(trace,bytes(changed),'edited',self.c,(w,h))
  with self.assertRaises(ValueError):g.verify_frame(trace.replace('pixels=unconfirmed','pixels=missing'),image,'edited',self.c,(w,h))
 def test_new_fourth_linen_row_does_not_hide_launcher_erasure_evidence(self):
  trace=('[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=0 first=0 rows=1\n'
   '[launcher.text.row] index=0 row=0 name=Focus Quil status=RUNNING / FOCUS\n[command_palette.close]\n'
   '[linen.list.text] x=0 y=50 w=640 h=750 scale=2 text_color=0xf0f8ff rows=4 selected=5 row_y=72 row_h=36 committed_view=1\n'
   '[linen.list.label] id=1 row=0 text=first\n[linen.list.label] id=2 row=1 text=second\n[linen.list.label] id=3 row=2 text=third\n[linen.list.label] id=5 row=3 text=readone\n')
  data=bytearray(1280*800*3);image=b'P6\n1280 800\n255\n'+data
  with self.assertRaisesRegex(ValueError,'insufficient'):g.launch.launcher_erased(trace,image,self.c,(1280,800))
  result=g.launcher_erased(trace,image,self.c,(1280,800));self.assertGreater(result['checked_ink_pixels'],1000)
  v,lines=g.launch.launcher_layout(trace)
  for row,line in lines:
   for col,char in enumerate(line):
    for gy,bits in enumerate(g.launch.glyph(self.c['font'],char)):
     for gx in range(5):
      if bits&(1<<(4-gx)):
       for dy in range(2):
        for dx in range(2):
         x=v['x']+8+(col*6+gx)*2+dx;y=v['y']+36+(row*9+gy)*2+dy;data[(y*1280+x)*3:(y*1280+x)*3+3]=b'\xf0\xf8\xff'
  with self.assertRaisesRegex(ValueError,'old launcher text remains'):g.launcher_erased(trace,b'P6\n1280 800\n255\n'+data,self.c,(1280,800))
 def test_legitimate_exact_linen_ink_overlap_partial_menu_and_coverage(self):
  trace=('[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=0 first=0 rows=1\n'
   '[launcher.text.row] index=0 row=0 name=Focus Quil status=RUNNING / FOCUS\n[command_palette.close]\n'
   '[linen.list.text] x=400 y=200 w=480 h=280 scale=2 text_color=0xf0f8ff rows=1 selected=5 row_y=72 row_h=36 committed_view=1\n'
   '[linen.list.label] id=5 row=0 text=readone\n')
  data=bytearray(1280*800*3)
  def paint(row,line):
   for col,char in enumerate(line):
    for gy,bits in enumerate(g.launch.glyph(self.c['font'],char)):
     for gx in range(5):
      if bits&(1<<(4-gx)):
       for dy in range(2):
        for dx in range(2):
         x=408+(col*6+gx)*2+dx;y=236+(row*9+gy)*2+dy;data[(y*1280+x)*3:(y*1280+x)*3+3]=b'\xf0\xf8\xff'
  for row,line in [(0,self.c['linen_headers'][0]),(1,self.c['linen_headers'][1]),(2,'> readone')]:paint(row,line)
  image=b'P6\n1280 800\n255\n'+data;result=g.launcher_erased(trace,image,self.c,(1280,800));self.assertGreater(result['known_linen_ink_skipped'],100)
  paint(3,'  RUNNING / FOCUS')
  with self.assertRaisesRegex(ValueError,'old launcher text remains'):g.launcher_erased(trace,b'P6\n1280 800\n255\n'+data,self.c,(1280,800))
  ambiguous=dict(self.c,linen_headers=['APPS / ACTIONS',self.c['linen_headers'][1]])
  with self.assertRaisesRegex(ValueError,'insufficient'):g.launcher_erased(trace,image,ambiguous,(1280,800))
 def test_whole_pixel_and_launcher_erasure_checks_both_required(self):
  v=self.make_view('returned')
  with mock.patch.object(g,'launcher_erased',return_value={}),mock.patch.object(g,'quil_view',return_value=v),mock.patch.object(g,'quil_pixels',return_value=[]):self.assertIn('geometry',g.verify_frame('trace',b'image','returned',self.c,(1280,800)))
  with mock.patch.object(g,'launcher_erased',side_effect=ValueError('overlay remains')):
   with self.assertRaises(ValueError):g.verify_frame('trace',b'image','returned',self.c,(1280,800))
  with mock.patch.object(g,'launcher_erased',return_value={}),mock.patch.object(g,'quil_view',return_value=v),mock.patch.object(g,'quil_pixels',return_value=['actual glyph mismatch']):
   with self.assertRaises(ValueError):g.verify_frame('trace',b'image','returned',self.c,(1280,800))

class CustodyAndJournal(unittest.TestCase):
 def test_actual_four_actions_and_exact_two_durable_saves(self):
  trace,actions=trace_fixture();r=g.joins(trace,actions);self.assertEqual(r['pointer_sequences'],[8,9,10,11]);self.assertEqual(r['generations'],[1,2]);self.assertEqual(g.journal(raw_disk(),r)['hash'],hex(g.fnv(g.FINAL)))
 def test_foreign_duplicate_wrong_hash_weak_saved_or_missing_edges_fail(self):
  trace,actions=trace_fixture()
  bad=[trace.replace('task=90 frame=201','task=9 frame=201',1),trace.replace('action=4','action=3',1),trace.replace('sequence=9','sequence=8'),trace.replace('target=9 action=2','target=15 action=2'),trace.replace('caller=9 object=5','caller=15 object=5',1),trace.replace('hash='+hex(g.fnv(g.FINAL)),'hash=0x1'),trace.replace('revision=14 dirty=0','revision=14 dirty=1'),trace.replace('barriers=3','barriers=2'),trace.replace('generation=2','generation=1'),trace.replace('pressed=false','pressed=true',1)]
  for changed in bad:
   with self.subTest(change=changed[-100:]):
    with self.assertRaises(ValueError):g.joins(changed,actions)
  changed=copy.deepcopy(actions);changed[0]['serial_end']=0
  with self.assertRaises(ValueError):g.joins(trace,changed)
 def test_three_focus_effects_have_distinct_explicit_and_automatic_windows(self):
  trace,actions=trace_fixture();meta=g.joins(trace,actions);self.assertEqual(len(meta['focus_windows']),3)
  focus='[launcher.action] target=9 task=90 frame=201 action=Focus from=0 to=0 visible=1 focused=1 activated=0 queued=0 accepted=1 ok=1 pixels=unconfirmed\n'
  lines=trace.splitlines(True);indexes=[i for i,line in enumerate(lines)if line==focus]
  for index in indexes:
   for replace in ('',focus*2,focus.replace('task=90','task=9'),focus.replace('frame=201','frame=301')):
    changed=lines.copy();changed[index]=replace
    with self.assertRaises(ValueError):g.joins(''.join(changed),actions)
  # A retained exact queued admission may precede its one positive completion
  # inside the same automatic-open window; it does not create another effect.
  position=trace.index(focus,trace.index('[linen.quil.disk_doc.intent]'));queued=focus.replace('visible=1 focused=1','visible=0 focused=0').replace('queued=0 accepted=1 ok=1','queued=1 accepted=1 ok=0');changed=trace[:position]+queued+trace[position:];shifted=copy.deepcopy(actions)
  for row in shifted:
   for key in ('serial_start','serial_end'):
    if row[key]>=position:row[key]+=len(queued)
  self.assertEqual(len(g.joins(changed,shifted)['focus_windows']),3)
  # Same count and exact subject are insufficient: move the automatic focus
  # before the selected-open intent, or the final explicit effect before its command.
  for index in (1,2):
   moved=lines.copy();moved[indexes[index]]=''
   boundary=next(i for i,line in enumerate(moved)if line.startswith('[linen.quil.disk_doc.intent]'))if index==1 else next(i for i,line in enumerate(moved)if line.startswith('[quil.persist.load.ok]'))
   moved.insert(boundary,focus)
   with self.assertRaises(ValueError):g.joins(''.join(moved),actions)
 def test_unselected_or_foreign_load_order_cannot_claim_reopen(self):
  trace,actions=trace_fixture()
  for changed in(trace.replace('path_id=5','path_id=6'),trace.replace('doc=5','doc=6',1),trace.replace('[quil.persist.load.ok]','[fake.load.ok]')):
   with self.assertRaises(ValueError):g.joins(changed,actions)
 def test_both_journal_generations_name_identity_hash_and_bytes_are_checked(self):
  trace,actions=trace_fixture();meta=g.joins(trace,actions);raw=raw_disk();start=(2200+5*128)*512
  for offset in(start+512,start+32768+512,start+504,start+32768+24,2046*512+16+5*32):
   changed=bytearray(raw);changed[offset]^=1
   with self.assertRaises((ValueError,AssertionError)):g.journal(bytes(changed),meta)
  wrong=dict(meta,identity=1)
  with self.assertRaises(ValueError):g.journal(raw,wrong)

if __name__=='__main__':unittest.main()
