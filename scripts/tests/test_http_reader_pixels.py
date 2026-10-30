"""Independent whole reader pixels/navigation: no native/QEMU execution."""
import copy,hashlib,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from http_content_gate import READER_BODY,READER_TAIL,STREAM_BODY,STREAM_TAIL,STREAM_MODES,MODES,wire_response,stream_receipts,load_pixel_contract,overview_preview,reader_frame,reader_move,reader_position,frame_prefix,verify_reader_pixels
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

class ReaderPixels(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=load_pixel_contract(ROOT)
 def frame(self,body=READER_BODY,w=624,h=706,anchor=0,expected_path=None):return reader_frame(body,w,h,self.contract,anchor,expected_path)
 def trace(self,frame,body=READER_BODY,w=624,h=706,x=600,y=70,sequence=1,ticket=2,position=False):
  return(f'[shell.tile.apply] sid=300 x={x} y={y} w={w} h={h}\n'
   f'[shell.document.viewport] target=15 task=15 surface=300 sequence={sequence} x={x} y={y} w={w} h={h} enqueued=1\n'
   f'[browser.viewport] target=15 task=15 sequence={sequence} w={w} h={h} origin=held_shell\n'
   f'[browser.http.released] ticket={ticket} view=Ready private_job=0\n'
   f'[browser.http.document] ticket={ticket} path={frame["path"]} status=200 private_bytes={len(body)} positive_release=1\n'
   f'[browser.reader.layout] first={frame["first"]} end={frame["end"]} body_rows={frame["body_rows"]} rows={frame["rows"]} cols={frame["cols"]} total_lines={frame["total_lines"]} source_bytes={len(body)} viewport={sequence} origin=validated_body tls=0 pixels=unconfirmed\n'
   +(f'[browser.reader.position] anchor={frame["source_anchor"]} viewport={sequence} tls=0 retained_body_only=1\n'if position else'')+
   f'[browser.view.applied] bytes={frame["bytes"]} http=258 body_bytes={len(body)} pixels=unconfirmed\n')
 def screen(self,frame,x=600,y=70,shift=0):
  pixels=bytearray(1280*800*3);factor=frame['scale'];contract=self.contract;color=contract['color'].to_bytes(3,'big')
  for row,line in enumerate(frame['lines']):
   for column,byte in enumerate(line):
    for gy,bits in enumerate(glyph(contract['font'],chr(byte))):
     for gx in range(5):
      if not bits&(1<<(4-gx)):continue
      for dy in range(factor):
       for dx in range(factor):
        px=x+contract['inset']+(column*contract['advance']+gx)*factor+dx;py=y+contract['top']+(row*contract['line']+gy)*factor+dy+shift
        pixels[(py*1280+px)*3:(py*1280+px)*3+3]=color
  return b'P6\n1280 800\n255\n'+pixels
 def result(self,frame,body=READER_BODY,w=624,h=706,anchor=0,screen=None,trace=None):
  return verify_reader_pixels(trace or self.trace(frame,body,w,h),screen or self.screen(frame),body,self.contract,anchor)
 def test_exact_2048_fixture_complete_home_chrome_and_body(self):
  self.assertEqual(len(READER_BODY),2048);self.assertTrue(READER_BODY.endswith(READER_TAIL));self.assertTrue(all(b==10 or 32<=b<=126 for b in READER_BODY))
  frame=self.frame();self.assertEqual((frame['cols'],frame['rows'],frame['body_rows'],frame['bytes']),(47,10,7,479));self.assertTrue(self.result(frame)['ok'])
  self.assertEqual(frame['lines'][0].rstrip(),b'Plain HTTP / no TLS');self.assertEqual(frame['lines'][1].rstrip(),b'probe.sexos.test /welcome')
 def test_tall_narrow_whole_frame_matches_display_and_source_row_cap(self):
  self.assertEqual(self.contract['reader_max_rows'],24);self.assertEqual(self.contract['max_rows'],24)
  frame=self.frame(b'abcdefghij\n'*100,w=82,h=2000)
  self.assertEqual((frame['rows'],frame['body_rows']),(24,21));self.assertLessEqual(frame['bytes'],512)
 def test_end_reaches_final_source_byte_well_beyond_preview(self):
  start=self.frame();anchor=reader_move(start,'end');end=self.frame(anchor=anchor)
  self.assertGreater(anchor,64);self.assertEqual(end['end'],end['total_lines']);self.assertIn(READER_TAIL,b''.join(end['lines'][2:-1]));self.assertTrue(self.result(end,anchor=anchor)['ok'])
  corrupt=copy.deepcopy(end);last=next(i for i in range(len(corrupt['lines'])-2,1,-1)if b'Z'in corrupt['lines'][i]);corrupt['lines'][last]=corrupt['lines'][last].replace(b'Z',b'X')
  self.assertFalse(self.result(end,anchor=anchor,screen=self.screen(corrupt))['ok'])
 def test_line_page_home_end_navigation_and_no_wrap_at_bounds(self):
  body=b'0123456789'*60;frame=self.frame(body,w=82,h=160);self.assertEqual((frame['cols'],frame['body_rows'],frame['total_lines']),(10,11,60))
  for key,first in [('up',0),('down',1),('pgdn',12),('pgup',1),('home',0),('end',49),('down',49),('pgdn',49),('up',48)]:
   frame=self.frame(body,w=82,h=160,anchor=reader_move(frame,key));self.assertEqual(frame['first'],first)
 def test_resize_uses_source_anchor_and_latest_geometry(self):
  frame=self.frame();anchor=reader_move(frame,'pgdn');frame=self.frame(anchor=anchor);anchor=frame['anchor']
  expanded=self.frame(w=1280,h=706,anchor=anchor);self.assertEqual(expanded['cols'],47)
  # Tile is deliberately moved to fit this actual synthetic framebuffer.
  trace=self.trace(expanded,w=1280,h=706,x=0);screen=self.screen(expanded,x=0)
  self.assertTrue(verify_reader_pixels(trace,screen,READER_BODY,self.contract,anchor)['ok'])
  restored=self.frame(anchor=expanded['anchor']);self.assertTrue(self.result(restored,anchor=expanded['anchor'])['ok'])
 def test_zoom_geometry_uses_exact_shell_notice_instead_of_old_tile(self):
  frame=self.frame(w=1280,h=706);trace=self.trace(frame,w=1280,h=706,x=0)
  trace=trace.replace('[shell.tile.apply] sid=300 x=0 y=70 w=1280 h=706','[shell.tile.apply] sid=300 x=600 y=70 w=624 h=706')
  screen=self.screen(frame,x=0);self.assertTrue(verify_reader_pixels(trace,screen,READER_BODY,self.contract)['ok'])
  for wrong in [trace.replace('surface=300 sequence=1','surface=300 sequence=2'),trace.replace('surface=300 sequence=1','surface=301 sequence=1'),trace.replace('x=0 y=70 w=1280 h=706 enqueued=1','x=0 y=70 w=624 h=706 enqueued=1')]:
   self.assertFalse(verify_reader_pixels(wrong,screen,READER_BODY,self.contract)['ok'])
 def test_wrong_complete_body_byte_or_row_is_rejected(self):
  frame=self.frame();wrong=copy.deepcopy(frame);wrong['lines'][2]=b'X'+wrong['lines'][2][1:]
  self.assertFalse(self.result(frame,screen=self.screen(wrong))['ok']);self.assertFalse(self.result(frame,screen=self.screen(frame,shift=18))['ok'])
 def test_header_host_and_footer_are_required(self):
  frame=self.frame()
  for row in [0,1,frame['rows']-1]:
   wrong=copy.deepcopy(frame);wrong['lines'][row]=b' '*frame['cols'];self.assertFalse(self.result(frame,screen=self.screen(wrong))['ok'])
 def test_padding_spare_column_and_old_rows_below_snapshot_must_erase(self):
  frame=self.frame();raw=self.screen(frame);header=len(b'P6\n1280 800\n255\n')
  for row,column in [(0,frame['cols']-1),(4,frame['cols']),(frame['rows']+2,0)]:
   wrong=bytearray(raw);x=600+self.contract['inset']+column*self.contract['advance']*frame['scale'];y=70+self.contract['top']+row*self.contract['line']*frame['scale'];offset=header+(y*1280+x)*3;wrong[offset:offset+3]=self.contract['color'].to_bytes(3,'big');self.assertFalse(self.result(frame,screen=bytes(wrong))['ok'])
 def test_short_body_keeps_exact_blank_rows(self):
  body=b'first\n\nlast';frame=self.frame(body);self.assertEqual(frame['total_lines'],3);self.assertEqual(frame['lines'][3],b' '*frame['cols']);self.assertTrue(self.result(frame,body)['ok'])
  wrong=copy.deepcopy(frame);wrong['lines'][6]=b'STALE'.ljust(frame['cols'],b' ');self.assertFalse(self.result(frame,body,screen=self.screen(wrong))['ok'])
 def test_readable_measure_and_exact_requested_anchor_survive_wrap_roundtrip(self):
  for width,height in((1280,750),(1024,718),(624,706)):
   frame=self.frame(STREAM_BODY,w=width,h=height);self.assertEqual((frame['cols'],frame['rows'],frame['body_rows'],frame['bytes']),(47,10,7,479));self.assertTrue(frame['lines'][-1].endswith(frame['progress'].encode()))
  body=b'x'*600;anchor=120
  for _ in range(4):
   for width in(82,124,142,624,82):
    frame=self.frame(body,w=width,h=160,anchor=anchor);self.assertEqual(frame['anchor'],120);self.assertEqual(frame['source_anchor'],120)
    if width==124:self.assertEqual((frame['first'],frame['first_byte']),(7,119))
    if width==82:self.assertEqual((frame['first'],frame['first_byte']),(12,120))
  for invalid in(-1,True,1.5,12289):
   with self.assertRaises(ValueError):self.frame(anchor=invalid)
 def test_progress_is_exact_empty_end_and_fully_checked_pixels(self):
  for body in(b'',b'x'*600):
   frame=self.frame(body);self.assertEqual(frame['progress'],'0-0/0'if not body else'1-7/13');self.assertTrue(self.result(frame,body)['ok'])
  body=b'\n'*12287+b'Z';first=self.frame(body);anchor=reader_move(first,'end');last=self.frame(body,anchor=anchor)
  self.assertEqual(last['progress'],'12282-12288/12288');self.assertTrue(self.result(last,body,anchor=anchor)['ok'])
  wrong=copy.deepcopy(last);wrong['lines'][-1]=wrong['lines'][-1].replace(b'12282',b'12281')
  self.assertFalse(self.result(last,body,anchor=anchor,screen=self.screen(wrong))['ok'])
 def test_responsive_https_header_and_complete_controls_preserve_lab_scope(self):
  contract=copy.deepcopy(self.contract);contract['reader_header']=contract['reader_https_headers'][0];contract['reader_footers']=contract['https_reader_footers'];contract['request']=b'GET /welcome HTTP/1.1\r\nHost: probe.sexos.test:46080\r\n\r\n'
  for width,height,header in((624,706,b'Controlled HTTPS / lab root'),(160,100,b'HTTPS / lab root'),(256,118,b'HTTPS / lab root'),(82,160,b'Lab HTTPS')):
   frame=reader_frame(b'x'*2048,width,height,contract);self.assertTrue(frame['lines'][0].startswith(header));self.assertTrue(frame['lines'][-1].startswith(b'PgUp PgDn V'if width in(624,160)else b'V I'))
   if width!=82:self.assertTrue(frame['lines'][-1].endswith(frame['progress'].encode()))
  tiny=reader_frame(b'\n'*12287+b'Z',82,160,contract,12277);self.assertIsNone(tiny['progress']);self.assertEqual(tiny['lines'][-1].rstrip(),b'V I')
 def test_native_position_receipt_is_optional_only_for_legacy_fixtures(self):
  frame=self.frame();trace=self.trace(frame,position=True);image=self.screen(frame);strict=copy.deepcopy(self.contract);strict['reader_require_position']=True
  self.assertTrue(verify_reader_pixels(trace,image,READER_BODY,strict)['ok'])
  legacy=self.trace(frame);self.assertTrue(self.result(frame,trace=legacy)['ok']);self.assertFalse(verify_reader_pixels(legacy,image,READER_BODY,strict)['ok'])
  position=next(line for line in trace.splitlines()if '[browser.reader.position]'in line)
  for wrong in(trace.replace('anchor=0 viewport=1','anchor=1 viewport=1'),trace.replace('anchor=0 viewport=1','anchor=0 viewport=2'),trace.replace('tls=0 retained_body_only=1','tls=1 retained_body_only=1'),trace.replace('retained_body_only=1','retained_body_only=0'),trace+position+'\n',trace.replace(position+'\n','')+position+'\n'):
   self.assertFalse(verify_reader_pixels(wrong,image,READER_BODY,strict)['ok'])
   self.assertFalse(verify_reader_pixels(wrong,image,READER_BODY,self.contract)['ok'])
 def test_intact_position_after_unfinished_foreign_log_fragment_is_valid(self):
  frame=self.frame();trace=self.trace(frame,position=True);token='[browser.reader.position]';raw=trace.replace(token,'[pdx.opcode.unknown] display type_id=0x15 caller='+token)
  strict=dict(self.contract,reader_require_position=True)
  self.assertTrue(verify_reader_pixels(raw,self.screen(frame),READER_BODY,strict)['ok'])
 def test_split_fields_duplicate_and_unterminated_position_still_fail(self):
  frame=self.frame();trace=self.trace(frame,position=True);position=next(line for line in trace.splitlines()if '[browser.reader.position]'in line);strict=dict(self.contract,reader_require_position=True)
  for bad in(trace.replace('anchor=0 viewport=1','anchor=0[other] x=1\n viewport=1'),trace.replace(position,position+'\nnoise'+position),trace[:trace.index(position)]+position):
   self.assertFalse(verify_reader_pixels(bad,self.screen(frame),READER_BODY,strict)['ok'])
 def test_position_with_foreign_suffix_or_embedded_token_is_not_repaired(self):
  frame=self.frame();trace=self.trace(frame,position=True);position=next(line for line in trace.splitlines()if '[browser.reader.position]'in line);strict=dict(self.contract,reader_require_position=True)
  for bad in(trace.replace(position,position+'[other] x=1'),trace.replace('viewport=1 tls=0 retained_body_only=1','viewport=[browser.reader.position]1 tls=0 retained_body_only=1')):
   self.assertFalse(verify_reader_pixels(bad,self.screen(frame),READER_BODY,strict)['ok'])
 def test_exact_frame_prefix_replays_original_snapshot_despite_later_clear(self):
  frame=self.frame();snapshot=self.trace(frame,position=True);record=dict(serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest(),requested_anchor=0)
  full=snapshot+'[browser.http.view] state=Cancelled tls=0\n';prefix=frame_prefix(full,record,reader=True)
  self.assertEqual(prefix,snapshot);self.assertTrue(verify_reader_pixels(prefix,self.screen(frame),READER_BODY,self.contract)['ok']);self.assertFalse(verify_reader_pixels(full,self.screen(frame),READER_BODY,self.contract)['ok'])
  status=dict(record,requested_anchor=None);self.assertEqual(frame_prefix(full,status),snapshot)
 def test_missing_changed_or_out_of_extent_frame_prefix_cannot_replay(self):
  snapshot='actual recorded frame\n';record=dict(serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest(),requested_anchor=0)
  for key in record:
   missing=dict(record);del missing[key]
   with self.assertRaises(ValueError):frame_prefix(snapshot,missing,reader=True)
  for key,value in(('serial_end',0),('serial_end',True),('serial_end',len(snapshot)+1),('serial_sha256','0'*64),('serial_sha256',None),('requested_anchor',None),('requested_anchor',-1),('requested_anchor',12289),('requested_anchor',True)):
   with self.assertRaises(ValueError):frame_prefix(snapshot,dict(record,**{key:value}),reader=True)
  with self.assertRaises(ValueError):frame_prefix('X'+snapshot[1:],record,reader=True)
  with self.assertRaises(ValueError):frame_prefix(snapshot,record,reader=False)
 def test_missing_or_forged_viewport_layout_and_applied_receipts_fail(self):
  frame=self.frame();trace=self.trace(frame)
  for wrong in ['',trace.replace('first=0','first=1'),trace.replace('http=258','http=2'),trace.replace('sequence=1 w=624','sequence=1 w=620'),trace[:trace.index('[browser.view.applied]')]]:
   self.assertFalse(verify_reader_pixels(wrong,self.screen(frame),READER_BODY,self.contract)['ok'])
 def test_actual_framebuffer_must_match_requested_boot_profile(self):
  frame=self.frame();trace=self.trace(frame);screen=self.screen(frame)
  self.assertTrue(verify_reader_pixels(trace,screen,READER_BODY,self.contract,resolution=(1280,800))['ok'])
  self.assertFalse(verify_reader_pixels(trace,screen,READER_BODY,self.contract,resolution=(1024,768))['ok'])
  compact=self.frame(w=512,h=359);raw=self.screen(compact,x=0);pixels=raw.split(b'\n',3)[3]
  cropped=b'P6\n1024 768\n255\n'+b''.join(pixels[y*1280*3:(y*1280+1024)*3]for y in range(768))
  self.assertTrue(verify_reader_pixels(self.trace(compact,w=512,h=359,x=0),cropped,READER_BODY,self.contract,resolution=(1024,768))['ok'])
 def test_clipped_missing_source_and_oversize_fail_closed(self):
  frame=self.frame()
  for wrong in [self.trace(frame).replace('h=706','h=150'),self.trace(frame).replace('y=70','y=200')]:self.assertFalse(self.result(frame,trace=wrong)['ok'])
  with self.assertRaises(ValueError):self.frame(b'x'*(self.contract['reader_max_body']+1))
  with self.assertRaises(ValueError):self.frame(w=20,h=100)
 def test_non_ascii_controls_model_and_overview_are_distinct(self):
  body=b'a\tb\nnext';frame=self.frame(body);self.assertTrue(frame['lines'][2].startswith(b'a b'));self.assertTrue(frame['lines'][3].startswith(b'next'))
  for unsupported in (b'a\r',b'a\0',b'a\x7f',b'\xff','é'.encode()):
   with self.assertRaisesRegex(ValueError,'UTF-8 profile'):self.frame(unsupported)
  self.assertEqual(overview_preview(b' \nA\t\tB\0\xff  ')[:5],b'A B ?');self.assertEqual(overview_preview(READER_BODY)[61:],b'...')

 def test_full_12288_stream_end_and_last_byte_are_independent_pixels(self):
  self.assertEqual(len(STREAM_BODY),12288);self.assertTrue(STREAM_BODY.endswith(STREAM_TAIL));self.assertEqual(self.contract['wire_max_body'],len(STREAM_BODY));self.assertEqual(self.contract['reader_max_body'],len(STREAM_BODY))
  first=self.frame(STREAM_BODY);anchor=reader_move(first,'end');last=self.frame(STREAM_BODY,anchor=anchor)
  self.assertGreater(anchor,11000);self.assertEqual(last['end'],last['total_lines']);self.assertIn(STREAM_TAIL,b''.join(last['lines'][2:-1]));self.assertTrue(self.result(last,STREAM_BODY,anchor=anchor)['ok'])
  bad=copy.deepcopy(last);index=next(i for i in range(len(bad['lines'])-2,1,-1)if b'Z'in bad['lines'][i]);bad['lines'][index]=bad['lines'][index].replace(b'Z',b'X')
  self.assertFalse(self.result(last,STREAM_BODY,anchor=anchor,screen=self.screen(bad))['ok'])
 def test_source_bound_oversize_and_short_stream_wire_are_distinct(self):
  oversized=wire_response('oversized',max_body=self.contract['wire_max_body']);headers,body=oversized.split(b'\r\n\r\n',1)
  self.assertIn(b'Content-Length: 12289',headers);self.assertEqual(len(body),12289)
  short=wire_response('stream-short',STREAM_BODY,max_body=self.contract['wire_max_body']);headers,body=short.split(b'\r\n\r\n',1)
  self.assertIn(b'Content-Length: 12288',headers);self.assertEqual(len(body),12279);self.assertEqual(body,STREAM_BODY[:-9])
  full=wire_response('stream',STREAM_BODY,max_body=self.contract['wire_max_body']);self.assertEqual(full.split(b'\r\n\r\n',1)[1],STREAM_BODY)
 def test_redirected_reader_header_uses_positive_final_path_and_independent_expectation(self):
  frame=self.frame(expected_path=b'/guide/text');trace=self.trace(frame);screen=self.screen(frame)
  self.assertEqual(frame['lines'][1].rstrip(),b'probe.sexos.test /guide/text')
  result=verify_reader_pixels(trace,screen,READER_BODY,self.contract,expected_path=b'/guide/text');self.assertTrue(result['ok'],result)
  self.assertEqual(result['geometry']['path'],'/guide/text');self.assertEqual(result['geometry']['document_ticket'],2)
  self.assertTrue(verify_reader_pixels(trace,screen,READER_BODY,self.contract)['ok'])
  self.assertFalse(verify_reader_pixels(trace,screen,READER_BODY,self.contract,expected_path=b'/welcome')['ok'])
  initial=self.frame();self.assertFalse(verify_reader_pixels(trace,self.screen(initial),READER_BODY,self.contract,expected_path=b'/guide/text')['ok'])
 def test_document_path_requires_matching_ready_release_ticket_and_length(self):
  frame=self.frame(expected_path=b'/guide/text');trace=self.trace(frame);screen=self.screen(frame)
  for wrong in [trace.replace('[browser.http.released] ticket=2','[browser.http.released] ticket=1'),trace.replace('[browser.http.document] ticket=2','[browser.http.document] ticket=3'),trace.replace('view=Ready private_job=0','view=Cancelled private_job=0'),trace.replace('private_job=0','private_job=1'),trace.replace('status=200 private_bytes=2048','status=302 private_bytes=2048'),trace.replace('private_bytes=2048','private_bytes=2047'),trace.replace('positive_release=1','positive_release=0'),trace.replace('ticket=2','ticket=0'),trace.replace('ticket=2','ticket=17536')]:
   with self.subTest(wrong=wrong):self.assertFalse(verify_reader_pixels(wrong,screen,READER_BODY,self.contract,expected_path=b'/guide/text')['ok'])
 def test_missing_unreleased_and_after_layout_document_receipts_cannot_pass(self):
  frame=self.frame();trace=self.trace(frame);screen=self.screen(frame)
  release=next(line for line in trace.splitlines(True)if '[browser.http.released]'in line);document=next(line for line in trace.splitlines(True)if '[browser.http.document]'in line)
  for wrong in [trace.replace(release,''),trace.replace(document,''),trace.replace(release+document,document+release),trace.replace(document,'')+document,trace.replace(document,'')+document.replace('ticket=2','ticket=1')]:
   self.assertFalse(verify_reader_pixels(wrong,screen,READER_BODY,self.contract)['ok'])
 def test_latest_ticket_cannot_reuse_earlier_ready_document_or_receipt(self):
  frame=self.frame();trace=self.trace(frame);screen=self.screen(frame)
  marker='[browser.reader.layout]';at=trace.index(marker)
  for newer in ['[browser.http.released] ticket=3 view=Ready private_job=0\n','[browser.http.released] ticket=3 view=Cancelled private_job=0\n','[browser.http.document] ticket=1 path=/welcome status=200 private_bytes=2048 positive_release=1\n']:
   wrong=trace[:at]+newer+trace[at:];self.assertFalse(verify_reader_pixels(wrong,screen,READER_BODY,self.contract)['ok'])
  # A subsequent completed document requires a new corresponding layout too.
  newer='[browser.http.released] ticket=3 view=Ready private_job=0\n[browser.http.document] ticket=3 path=/guide status=200 private_bytes=2048 positive_release=1\n'
  self.assertFalse(verify_reader_pixels(trace+newer,screen,READER_BODY,self.contract)['ok'])
 def test_current_pending_or_cancelled_view_cannot_reuse_old_ready_path(self):
  frame=self.frame();trace=self.trace(frame);screen=self.screen(frame)
  self.assertTrue(verify_reader_pixels(trace+'[browser.http.view] state=Ready tls=0\n',screen,READER_BODY,self.contract)['ok'])
  for state in ['Pending','Cancelled','Unavailable','Uncertain']:
   self.assertFalse(verify_reader_pixels(trace+f'[browser.http.view] state={state} tls=0\n',screen,READER_BODY,self.contract)['ok'])
 def test_forbidden_or_wrong_frame_path_is_rejected_even_when_it_has_matching_pixels(self):
  frame=self.frame();trace=self.trace(frame);screen=self.screen(frame)
  for wrong_path in [b'//other/guide',b'http://other/guide',b'/a?b',b'/a#b',b'/../guide',b'/a%2fb',b'/a\\b',b'/a b',b'/'*49]:
   with self.assertRaises(ValueError):self.frame(expected_path=wrong_path)
   wrong=trace.replace('path=/welcome','path='+wrong_path.decode('ascii'));self.assertFalse(verify_reader_pixels(wrong,screen,READER_BODY,self.contract)['ok'])
  # Even clipped path suffixes remain constrained by the independent fixture.
  path=b'/'+b'a'*47;frame=self.frame(w=82,h=160,expected_path=path);trace=self.trace(frame,w=82,h=160);screen=self.screen(frame)
  self.assertFalse(verify_reader_pixels(trace,screen,READER_BODY,self.contract,expected_path=path[:-1]+b'b')['ok'])
 def test_original_redirect_negative_fixture_now_denies_cross_endpoint_location(self):
  response=wire_response('redirect');self.assertIn(b'Location: http://outside.sexos.test/elsewhere\r\n',response)

 def stream_trace(self,receiving=True):
  trace='[net.job.resolved.admitted] owner=15 task=15 ticket=2 dns_generation=1 observed=10 expires=60010 now=12 fixed=1 at_begin=1\n[net.job.stream.admitted] owner=15 task=15 ticket=2 kind=9 ring=1024 max_wire=13312 total_ticks=30000 idle_ticks=5000 authority=held_route\n[net.job.egress.submit] ticket=2 sequence=2 length=66 hash=0x1234 scope=dma_admission_not_delivery\n'
  for consumed in [4]+list(range(1024,12289,1024)):
   produced=min(12394,256 if consumed==4 else consumed+256);state='Receiving'if receiving and consumed<12288 else'Complete'
   trace+=f'[net.job.stream.drain] owner=15 task=15 ticket=2 produced={produced} consumed={consumed} buffered={produced-consumed} ring=1024 state_before={state} receipt=retained not_delivery=1\n'
  return trace
 def test_stream_receipts_require_receiving_and_large_actual_consumption(self):
  trace=self.stream_trace();result=stream_receipts(trace,[2],self.contract,[2],[2]);self.assertTrue(result['ok'],result)
  self.assertLess(result['drains'][-1]['produced'],self.contract['stream_max_wire']);self.assertEqual(result['drains'][-1]['consumed'],12288)
  self.assertFalse(stream_receipts(self.stream_trace(False),[2],self.contract,[2],[2])['ok'])
  small='\n'.join(line for line in trace.splitlines()if 'stream.drain'not in line or int(line.split('consumed=')[1].split()[0])<=4096);self.assertFalse(stream_receipts(small,[2],self.contract,[2],[2])['ok'])
 def test_stream_receipts_reject_repeated_consumption_bad_ring_and_false_admission(self):
  trace=self.stream_trace();first=next(line for line in trace.splitlines()if 'consumed=1024 'in line)
  for wrong in [trace+'\n'+first,trace.replace('buffered=252','buffered=1025'),trace.replace('max_wire=13312','max_wire=16384'),trace.replace('kind=9','kind=8'),trace.replace('owner=15','owner=16'),trace.replace('consumed=1024','consumed=2'),trace.replace('receipt=retained','receipt=delivered'),trace.replace('scope=dma_admission_not_delivery','scope=delivery')]:
   self.assertFalse(stream_receipts(wrong,[2],self.contract,[2],[2])['ok'])
 def test_stream_modes_add_all_cases_without_removing_prior_modes(self):
  self.assertEqual(STREAM_MODES,('stream','stream-short','stream-reset','stream-cancel-retry','stream-close'))
  for mode in ('echo','malformed','short-body','oversized','redirect','reset','silence','cancel-retry','close','switch','reader','dns-cname-sections','dns-close')+STREAM_MODES:self.assertIn(mode,MODES)

if __name__=='__main__':unittest.main()
