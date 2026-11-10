"""Independent synthetic panel pixels and adverse receipts; no native evidence."""
import sys,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path[:0]=[str(ROOT/'scripts'),str(ROOT/'scripts/tests')]
import tls_prepare_gate as t
import browser_source_evidence as e
from test_browser_source_evidence import inventory
C=t.tls_contract(ROOT)
LABELS={'Preparing':'preparing','HelloPending':'hello pending; no peer','Unavailable':'unavailable','Cancelled':'cancelled','Rejected':'rejected','TimedOut':'timed out','Uncertain':'waiting receipt'}

def fnv(data):
 h=0xcbf29ce484222325
 for byte in data:h=((h^byte)*0x100000001b3)%2**64
 return h

def case(target=15,state='HelloPending',paint=7):
 task={15:150,30:300,31:310}[target];sid={15:300,30:343,31:344}[target]
 lines=['Kaleidoscope / TLS','TLS preparation:',LABELS[state],'Local only / no peer','L prep Q clear Y peer']+['']*8
 payload='\n'.join(line.ljust(37)for line in lines).encode();assert len(payload)==493
 code=list(LABELS).index(state)+1;common=f'target={target} task={task} surface={sid} viewport=5 '
 stamp=f'bytes=493 hash={fnv(payload)} cols=37 rows=13 state_code={code} complete=1 '
 trace=inventory(e.FUTURE_PLANS,True)
 trace+=f'[browser.viewport] target={target} task={task} sequence=5 w=244 h=180 origin=held_shell\n'
 trace+=f'[shell.document.viewport] target={target} task={task} surface={sid} sequence=5 x=20 y=60 w=244 h=180 enqueued=1\n';start=len(trace)
 trace+='[browser.tls.layout] '+common+'w=244 h=180 '+stamp+'pixels=unconfirmed\n'
 trace+=f'[display.browser.text.committed] target={target} task={task} surface={sid} paint={paint} bytes=493 hash={fnv(payload)} changed=1 pixels=unconfirmed\n'
 trace+='[browser.tls.applied] '+common+stamp+f'sequence={paint} positive_ack=1 pixels=unconfirmed\n'
 width,height=320,260;raw=bytearray(width*height*3);background=(0x704890 if sid==343 else 0x303860).to_bytes(3,'big');foreground=(0xcdd6f4).to_bytes(3,'big')
 for y in range(60,240):
  for x in range(20,264):raw[(y*width+x)*3:(y*width+x)*3+3]=background
 from check_desktop_pixels import draw_cell
 for row,line in enumerate(lines):
  for col,char in enumerate(line):draw_cell(raw,width,28+col*6,94+row*9,char,1,foreground,C['font'])
 return trace,b'P6\n320 260\n255\n'+raw,(start,len(trace)),t.Consumer(target,task,sid)

class Panel(unittest.TestCase):
 def test_three_unequal_held_identities_all_states(self):
  for target in (15,30,31):
   for state in LABELS:
    trace,ppm,window,ctx=case(target,state)
    self.assertEqual(t.consumer_context(trace,target),ctx)
    result=t.verify_tls_pixels(trace,ppm,state,C,consumer=ctx,window=window)
    self.assertTrue(result['ok'],result);self.assertTrue(result['status_qualified'])
 def test_payload_and_bottom_rim_boundaries(self):
  # Literal row/column goldens independent of production equations.
  for w,h,cols,rows,complete in[(244,180,37,13,True),(255,180,38,13,True),(256,180,19,7,False),(127,80,17,4,False),(244,48,37,0,False),(244,49,37,1,False),(244,57,37,1,False),(244,58,37,2,False),(20,180,0,0,False)]:
   model=t.tls_panel_frame(w,h,'HelloPending',C);self.assertEqual((model['columns'],model['rows'],model['complete']),(cols,rows,complete))
  tiny=t.tls_panel_frame(127,80,'HelloPending',C)['data'];self.assertIn(b'Enlarge tile',tiny);self.assertNotIn(b'hello',tiny)
 def test_foreign_stale_full_hash_and_window_controls(self):
  trace,ppm,window,ctx=case(30)
  changes=[trace.replace('surface=343 viewport=5','surface=344 viewport=5'),trace.replace('task=300 surface=343','task=30 surface=343'),trace.replace('sequence=7 positive_ack=1','sequence=6 positive_ack=1'),trace.replace('positive_ack=1','positive_ack=0'),trace.replace('[display.browser.text.committed]','[missing.commit]'),trace.replace('paint=7','paint=8'),trace.replace('hash=','hash=0',1),trace.replace('bytes=493','bytes=492',1),trace+'[browser.viewport] target=30 task=300 sequence=6 w=244 h=180 origin=held_shell\n',trace.replace('slot=73 source=','slot=60 source='),trace+'[net.app.cleanup.ack] target=30 task=300 client_leases=0 shared_cache=retained ok=1\n']
  for bad in changes:
   r=t.verify_tls_pixels(bad,ppm,'HelloPending',C,consumer=ctx,window=(window[0],len(bad)));self.assertFalse(r['ok'],r)
  for window2 in[(0,window[0]),(window[0]+1,len(trace))]:self.assertFalse(t.verify_tls_pixels(trace,ppm,'HelloPending',C,consumer=ctx,window=window2)['ok'])
  for wrong in(t.Consumer(30,30,343),t.Consumer(30,300,300)):
   self.assertFalse(t.verify_tls_pixels(trace,ppm,'HelloPending',C,consumer=wrong,window=window)['ok'])
 def test_shell_return_log_may_follow_browser_layout_and_applied(self):
  trace,ppm,_,ctx=case();lines=trace.splitlines(keepends=True)
  shell=next(line for line in lines if line.startswith('[shell.document.viewport]'))
  trace=''.join(line for line in lines if line!=shell)+shell
  result=t.verify_tls_pixels(trace,ppm,'HelloPending',C,consumer=ctx,window=(trace.index('[browser.tls.layout]'),len(trace)))
  self.assertTrue(result['ok'],result)
 def test_full_global_stream_validation_precedes_capture_selection(self):
  trace,ppm,window,ctx=case()
  bad=trace+'[clock.source.snapshot] pd=13 task=130 slot=74 seconds=1793404800 attempts=1 precision=1 profile=q35_cmos_utc ordinal=1\n[certificate.time.snapshot] owner=31 task=31 seq=1 seconds=1793404800 source=q35_cmos_utc precision=1 receipt=retained\n'
  self.assertFalse(t.verify_tls_pixels(bad,ppm,'HelloPending',C,consumer=ctx,window=window)['ok'])
 def test_tiny_fallback_is_pixel_checkable_but_never_status_qualified(self):
  trace,_,window,ctx=case();old=t.tls_panel_frame(244,180,'HelloPending',C)['hash']
  lines=['Enlarge tile','TLS / local only','',''];data='\n'.join(line.ljust(17)for line in lines).encode();self.assertEqual(len(data),71)
  trace=trace.replace('w=244 h=180','w=127 h=80').replace('bytes=493','bytes=71').replace('cols=37 rows=13','cols=17 rows=4').replace('complete=1','complete=0').replace('hash='+str(old),'hash='+str(fnv(data)))
  start=trace.index('[browser.tls.layout]');width,height=320,260;raw=bytearray(width*height*3)
  for y in range(60,140):
   for x in range(20,147):raw[(y*width+x)*3:(y*width+x)*3+3]=bytes.fromhex('303860')
  from check_desktop_pixels import draw_cell
  for row,line in enumerate(lines):
   for col,char in enumerate(line):draw_cell(raw,width,28+col*6,94+row*9,char,1,bytes.fromhex('cdd6f4'),C['font'])
  ppm=b'P6\n320 260\n255\n'+raw
  self.assertFalse(t.verify_tls_pixels(trace,ppm,'HelloPending',C,consumer=ctx,window=(start,len(trace)))['ok'])
  r=t.verify_tls_pixels(trace,ppm,'HelloPending',C,consumer=ctx,window=(start,len(trace)),require_complete=False)
  self.assertTrue(r['ok'],r);self.assertFalse(r['status_qualified'])
 def test_only_exact_public_frame_metadata_can_carry_hash(self):
  trace,_,_,_=case();held=e.validate_inventory(trace,e.FUTURE_PLANS,require_version=True)
  for line in trace.splitlines():
   if line.startswith(('[browser.tls.layout]','[browser.tls.applied]')):
    self.assertTrue(t._public_tls_frame_metadata(line,held))
    for bad in(line+' secret=1',line.replace('bytes=493','bytes=492'),line.replace('state_code=2','state_code=8'),line.replace('task=150','task=0150'),line.replace('hash=','fingerprint='),line.replace('viewport=5','viewport=0'),line.replace('task=150','task=15'),line.replace('surface=300','surface=343'),line.replace('target=15','target=99')):self.assertFalse(t._public_tls_frame_metadata(bad,held),bad)
  self.assertFalse(t._public_tls_frame_metadata('[browser.tls.prepared] hash=123 bytes=32',held))
 def test_natural_scale2_glyph_cells_and_erasure(self):
  trace,_,_,ctx=case();old=t.tls_panel_frame(244,180,'HelloPending',C)['hash']
  lines=['Kaleidoscope / TLS','TLS preparation:','hello pending; no peer','Local only / no peer','L prep Q clear Y peer']+['']*7
  data='\n'.join(line.ljust(40)for line in lines).encode();self.assertEqual(len(data),491)
  trace=trace.replace('w=244 h=180','w=512 h=359').replace('bytes=493','bytes=491').replace('cols=37 rows=13','cols=40 rows=12').replace('hash='+str(old),'hash='+str(fnv(data)))
  width,height=600,450;raw=bytearray(width*height*3)
  for y in range(60,419):
   for x in range(20,532):raw[(y*width+x)*3:(y*width+x)*3+3]=bytes.fromhex('303860')
  from check_desktop_pixels import draw_cell
  for row,line in enumerate(lines):
   for col,char in enumerate(line):draw_cell(raw,width,28+col*12,94+row*18,char,2,bytes.fromhex('cdd6f4'),C['font'])
  ink=[i for i in range(0,len(raw),3)if raw[i:i+3]==bytes.fromhex('cdd6f4')]
  window=(trace.index('[browser.tls.layout]'),len(trace));head=b'P6\n600 450\n255\n'
  r=t.verify_tls_pixels(trace,head+raw,'HelloPending',C,consumer=ctx,window=window);self.assertTrue(r['ok'],r);self.assertEqual(r['geometry']['scale'],2)
  raw[ink[1]:ink[1]+3]=bytes.fromhex('303860');self.assertFalse(t.verify_tls_pixels(trace,head+raw,'HelloPending',C,consumer=ctx,window=window)['ok'])
 def test_exact_background_ink_and_erased_canvas(self):
  trace,ppm,window,ctx=case();head=len(b'P6\n320 260\n255\n')
  for x,y in[(28,94),(29,96),(100,220)]:
   changed=bytearray(ppm);at=head+(y*320+x)*3;changed[at:at+3]=b'\xff\0\0'
   self.assertFalse(t.verify_tls_pixels(trace,bytes(changed),'HelloPending',C,consumer=ctx,window=window)['ok'])

if __name__=='__main__':unittest.main()
