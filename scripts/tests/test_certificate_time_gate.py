"""Public UTC/checker fixtures only, never native RTC or rollback evidence."""
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import certificate_time_gate as gate
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]
SECONDS=1793367907
def command(mode='normal'):
 return ['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-rtc',gate.RTC_INVALID if mode=='invalid-year'else gate.RTC_NORMAL]
def metadata(mode='normal',pending=0):
 lines=['[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[sexnet.clock.source] held=1 source=q35_cmos_utc no_ticks_fallback=1 precision=1']
 for seq in range(1,2 if mode=='close'else 3):
  lines.append('[browser.clock.intent] accepted=1 explicit=1 no_ticks_fallback=1')
  if mode=='invalid-year':lines.append('[browser.clock.view] state=Unavailable source=q35_cmos_utc snapshot=1')
  else:
   lines.extend([f'[clock.source.snapshot] pd=13 task=13 slot=61 seconds={SECONDS+seq-1} attempts=2 precision=1 profile=q35_cmos_utc ordinal={seq}',f'[certificate.time.snapshot] owner=15 task=15 seq={seq} seconds={SECONDS+seq-1} source=q35_cmos_utc precision=1 receipt=retained'])
   if mode!='close'or not pending:lines.append(f'[browser.clock.view] state=Ready({SECONDS+seq-1}) source=q35_cmos_utc snapshot=1')
  if mode!='close'and seq==1:lines.append('[browser.clock.view] state=Cancelled source=q35_cmos_utc snapshot=1')
 if mode=='close':lines.extend([f'[browser.clock.closed] target=15 task=15 pending={pending} terminal=1','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1','[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1'])
 return '\n'.join(lines)+'\n'
def trace(state,c,w=900,h=650):
 n=len(c['prefix'])+50;base=n+258
 return f'[browser.preview.layout] bytes={n} newline=1 cached_or_sample=1\n[shell.document.viewport] target=15 task=15 surface=300 sequence=7 x=20 y=100 w={w} h={h} enqueued=1\n[browser.viewport] target=15 task=15 sequence=7 w={w} h={h} origin=held_shell\n[browser.clock.layout] base={base} bytes=64 viewport=7 state={state} pixels=unconfirmed\n[browser.view.applied] bytes={base+64} http=0 body_bytes=0 pixels=unconfirmed\n'
def pixels(text,state,c):
 g=gate.clock_geometry(text,state,c);raw=bytearray(1280*800*3);scale=g['scale'];color=c['color'].to_bytes(3,'big')
 for row in g['rows']:
  for col,ch in enumerate(row['text']):
   for gy,bits in enumerate(glyph(c['font'],ch)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      for dy in range(scale):
       for dx in range(scale):
        x=row['x']+(col*c['advance']+gx)*scale+dx;y=row['y']+gy*scale+dy;at=(y*1280+x)*3;raw[at:at+3]=color
 return b'P6\n1280 800\n255\n'+raw

class CertificateTimeOracle(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=gate.clock_contract(ROOT)
 def test_exact_normal_and_invalid_year_rtc_profiles(self):
  for mode in('normal','invalid-year','close'):self.assertTrue(gate.command_profile(command(mode),mode)['ok'])
  self.assertFalse(gate.command_profile(command('invalid-year'),'normal')['ok'])
  for args in[('-localtime',),('-icount','1'),('-incoming','defer'),('-rtc',gate.RTC_NORMAL),('-machine','pc,rr=replay')]:self.assertFalse(gate.command_profile(command()+list(args),'normal')['ok'])
 def test_python_independent_utc_calendar_and_public_range(self):
  self.assertEqual(gate.utc_label(f'Ready({SECONDS})',self.c),b'2026-10-30T13:45:07Z')
  self.assertEqual(gate.utc_label('Ready(1577836800)',self.c),b'2020-01-01T00:00:00Z')
  self.assertEqual(gate.utc_label('Ready(4102444799)',self.c),b'2099-12-31T23:59:59Z')
  for value in('Ready(0)','Ready(4102444800)','Ready(-1)','other'):
   with self.assertRaises(ValueError):gate.utc_label(value,self.c)
 def test_each_actual_metadata_workflow_and_close_pending_is_reported(self):
  for mode in('normal','invalid-year','close'):
   r=gate.inspect_clock(metadata(mode),mode,command(mode),SECONDS-5,SECONDS+5);self.assertTrue(r['ok'],r)
  for pending in(0,1):self.assertEqual(gate.inspect_clock(metadata('close',pending),'close',command(),SECONDS-5,SECONDS+5)['closed_pending'],pending)
 def test_host_utc_bracket_catches_timezone_or_civil_conversion_error(self):
  self.assertFalse(gate.inspect_clock(metadata(),'normal',command(),SECONDS+3600,SECONDS+3605)['ok'])
  self.assertFalse(gate.inspect_clock(metadata(),'normal',command(),SECONDS-3605,SECONDS-3600)['ok'])
 def test_fresh_sequence_source_ordinal_progress_and_client_join_required(self):
  text=metadata()
  for bad in[text.replace('ordinal=2','ordinal=1'),text.replace('seq=2','seq=1'),text.replace(f'Ready({SECONDS+1})',f'Ready({SECONDS+2})'),text.replace(f'seconds={SECONDS+1}',f'seconds={SECONDS}').replace(f'Ready({SECONDS+1})',f'Ready({SECONDS})'),text.replace('task=13 slot=61','task=15 slot=61'),text.replace('attempts=2 precision','attempts=5 precision')]:self.assertFalse(gate.inspect_clock(bad,'normal',command(),SECONDS-5,SECONDS+5)['ok'])
 def test_invalid_civil_year_has_info_but_no_positive_reads_or_ticks_fallback(self):
  text=metadata('invalid-year');extra=f'[clock.source.snapshot] pd=13 task=13 slot=61 seconds={SECONDS} attempts=1 precision=1 profile=q35_cmos_utc ordinal=1\n'
  self.assertFalse(gate.inspect_clock(text+extra,'invalid-year',command('invalid-year'),SECONDS-5,SECONDS+5)['ok'])
  self.assertFalse(gate.inspect_clock(text.replace('no_ticks_fallback=1','no_ticks_fallback=0'),'invalid-year',command('invalid-year'),SECONDS-5,SECONDS+5)['ok'])
 def test_close_requires_exact_cache_retirement_and_does_not_invent_pending(self):
  text=metadata('close')
  for bad in[text.replace('target=15 task=15 before_cleanup','target=15 task=99 before_cleanup'),text.replace('high_water_retained=1','high_water_retained=0'),text.replace('pending=0','pending=1')]:self.assertFalse(gate.inspect_clock(bad,'close',command(),SECONDS-5,SECONDS+5)['ok'])
 def test_whole_utc_and_clear_glyphs_padding_and_trailing_erasure(self):
  for state in(f'Ready({SECONDS})','Cancelled','Unavailable','Rollback'):
   t=trace(state,self.c);r=gate.verify_clock_pixels(t,pixels(t,state,self.c),state,self.c,(1280,800));self.assertTrue(r['ok'],r)
 def test_stale_date_or_wrong_latest_layout_cannot_qualify_clear(self):
  state=f'Ready({SECONDS})';t=trace(state,self.c);raw=pixels(t,state,self.c)
  self.assertFalse(gate.verify_clock_pixels(trace('Cancelled',self.c),raw,'Cancelled',self.c)['ok'])
  self.assertFalse(gate.verify_clock_pixels(t,raw,f'Ready({SECONDS+1})',self.c)['ok'])
 def test_clipped_or_unapplied_clock_pixels_fail_without_skip(self):
  for t in[trace('Cancelled',self.c,h=100),trace('Cancelled',self.c).replace('[browser.view.applied]','[not.applied]'),trace('Cancelled',self.c).replace('viewport=7','viewport=8')]:self.assertFalse(gate.verify_clock_pixels(t,b'','Cancelled',self.c)['ok'])

if __name__=='__main__':unittest.main()
