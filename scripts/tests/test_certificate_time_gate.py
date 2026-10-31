"""Public UTC/checker fixtures only, never native RTC or rollback evidence."""
import contextlib,io,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import certificate_time_gate as gate
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]
SECONDS=1793367907
def command(mode='normal'):
 return ['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-rtc',gate.RTC_INVALID if mode=='invalid-year'else gate.RTC_NORMAL]
def metadata(mode='normal',pending=0):
 lines=['[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.silkbar.clock.source] holder=6 task=6 slot=61 consumer=6 consumer_task=6 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[sexnet.clock.source] held=1 source=q35_cmos_utc no_ticks_fallback=1 precision=1']
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
 f=gate.calendar_frame(w,h,state,c);n=len(f['data']);digest=f['hash']
 return f'[shell.document.viewport] target=15 task=15 surface=300 sequence=7 x=20 y=100 w={w} h={h} enqueued=1\n[browser.viewport] target=15 task=15 sequence=7 w={w} h={h} origin=held_shell\n[browser.clock.layout] bytes={n} hash={digest} cols={f["cols"]} rows={f["rows"]} viewport=7 state={state} snapshot_only=1 pixels=unconfirmed\n[browser.clock.applied] bytes={n} hash={digest} rows={f["rows"]} viewport=7 sequence=9 positive_ack=1\n'
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
 def test_global_ordinals_include_only_the_second_exact_boot_holder(self):
  text=metadata().replace('ordinal=2','ordinal=3');needle=f'[clock.source.snapshot] pd=13 task=13 slot=61 seconds={SECONDS+1}'
  bar=f'[clock.source.snapshot] pd=6 task=6 slot=61 seconds={SECONDS} attempts=1 precision=1 profile=q35_cmos_utc ordinal=2\n';text=text.replace(needle,bar+needle)
  self.assertTrue(gate.inspect_clock(text,'normal',command(),SECONDS-5,SECONDS+5)['ok'])
  for bad in(text.replace('pd=6 task=6 slot=61 seconds','pd=6 task=99 slot=61 seconds'),text.replace('ordinal=3','ordinal=4'),text.replace('attempts=1 precision=1 profile=q35_cmos_utc ordinal=2','attempts=5 precision=1 profile=q35_cmos_utc ordinal=2'),text.replace('[boot.silkbar.clock.source]','[missing.bar.grant]')):
   self.assertFalse(gate.inspect_clock(bad,'normal',command(),SECONDS-5,SECONDS+5)['ok'])
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
  for t in[trace('Cancelled',self.c,h=100),trace('Cancelled',self.c).replace('[browser.clock.applied]','[not.applied]'),trace('Cancelled',self.c).replace('viewport=7','viewport=8')]:self.assertFalse(gate.verify_clock_pixels(t,b'','Cancelled',self.c)['ok'])
 def test_independent_native_calendar_views_and_provenance(self):
  f=gate.calendar_frame(640,375,f'Ready({SECONDS})',self.c);lines=[s.rstrip()for s in f['lines']]
  self.assertEqual(lines[:4],[b'Clock / last query snapshot',b'2026-10-30T13:45:07Z',b'SexTime 437319907 s',b'Long Count 13.0.14.1.1'])
  self.assertEqual(lines[4:8],[b'UTC day / GMT 584283',b'q35 CMOS / 1-second precision',b'Snapshot only / not live',b'T sample  Q clear'])
 def test_calendar_measure_does_not_lose_rows_when_window_grows(self):
  for w,h in[(1280,712),(1024,680),(640,375),(512,359)]:
   with self.subTest(w=w):
    f=gate.calendar_frame(w,h,f'Ready({SECONDS})',self.c);self.assertTrue(f['full_calendar']);self.assertLessEqual(f['cols'],63);self.assertGreaterEqual(f['rows'],8);self.assertLessEqual(len(f['data']),512)
 def restore_fixture(self):
  text='[shell.document.viewport] target=15 task=15 surface=300 sequence=2 x=640 y=425 w=640 h=375 enqueued=1\n[shell.window.action] action=ZoomToggle frame=13 sid=300 ok=1\n';captures=[];checked=[]
  for phase in range(3):
   text+=f'[capture] phase={phase}\n';captures.append(dict(serial_end=len(text)));checked.append(dict(geometry=dict(tile=dict(x=0,y=50,w=1280,h=750),viewport=3)))
  text+='[shell.window.action] action=ZoomToggle frame=13 sid=300 ok=1\n[capture] phase=3\n';captures.append(dict(serial_end=len(text)));checked.append(dict(geometry=dict(tile=dict(x=640,y=425,w=640,h=375),viewport=4)))
  return text,captures,checked
 def test_actual_zoom_restore_requires_the_exact_initial_held_tile(self):
  text,captures,checked=self.restore_fixture();proof=gate.verify_clock_restore(text,captures,checked,(1280,800));self.assertTrue(proof['ok']);self.assertEqual(proof['restored_viewport'],4)
 def test_fullscreen_reuse_foreign_tile_and_missing_restore_action_fail(self):
  for fault in('reused_full','foreign_tile','old_viewport','late_action','foreign_action','no_original'):
   text,captures,checked=self.restore_fixture()
   if fault=='reused_full':checked[3]['geometry']['tile']=checked[0]['geometry']['tile']
   if fault=='foreign_tile':checked[3]['geometry']['tile']['x']=0
   if fault=='old_viewport':checked[3]['geometry']['viewport']=3
   if fault=='late_action':captures[3]['serial_end']=captures[2]['serial_end']+1
   if fault=='foreign_action':text=text.replace('sid=300','sid=999',1)
   if fault=='no_original':text=text.replace('target=15','target=99')
   with self.subTest(fault=fault),self.assertRaisesRegex(ValueError,'clock'):gate.verify_clock_restore(text,captures,checked,(1280,800))
 def test_nonready_and_tiny_panels_never_retain_calendar_values(self):
  for state in('Idle','Pending','Unavailable','Cancelled','Uncertain','Rollback','Ready(0)'):
   f=gate.calendar_frame(640,375,state,self.c);self.assertNotIn(b'2026-10-30',f['data']);self.assertNotIn(b'437319907',f['data']);self.assertIsNone(f['utc_text'])
  for w,h in[(200,100),(640,100),(0,0)]:
   f=gate.calendar_frame(w,h,f'Ready({SECONDS})',self.c);self.assertFalse(f['full_calendar']);self.assertNotIn(b'2026-10-30',f['data']);self.assertNotIn(b'437319907',f['data'])
 def test_exact_whole_frame_hash_and_applied_viewport_are_required(self):
  state=f'Ready({SECONDS})';t=trace(state,self.c);raw=pixels(t,state,self.c)
  for bad in[t.replace('positive_ack=1','positive_ack=0'),t.replace('sequence=9','sequence=0'),t.replace('rows=8 viewport=7 sequence','rows=7 viewport=7 sequence'),t.replace('hash=0x','hash=0xf',1)]:
   self.assertFalse(gate.verify_clock_pixels(bad,raw,state,self.c)['ok'])
 def test_final_glyph_row_fits_without_requiring_invisible_line_spacing(self):
  state=f'Ready({SECONDS})';t=trace(state,self.c,w=640,h=176);self.assertEqual(len(gate.clock_geometry(t,state,self.c)['rows']),8)
  self.assertTrue(gate.verify_clock_pixels(t,pixels(t,state,self.c),state,self.c)['ok'])
  self.assertFalse(gate.calendar_frame(640,175,state,self.c)['full_calendar'])
 def test_stale_ink_to_right_or_below_capped_calendar_measure_is_rejected(self):
  state=f'Ready({SECONDS})';t=trace(state,self.c);raw=pixels(t,state,self.c);header=len(b'P6\n1280 800\n255\n');color=self.c['color'].to_bytes(3,'big')
  for x,y in[(900,140),(40,700)]:
   bad=bytearray(raw);at=header+(y*1280+x)*3;bad[at:at+3]=color
   self.assertFalse(gate.verify_clock_pixels(t,bytes(bad),state,self.c)['ok'])
 def test_cli_raw_check_needs_no_source_disk_and_never_starts_a_guest(self):
  with tempfile.TemporaryDirectory()as name:
   out=Path(name);iso=out/'tested.iso';binding=out/'source-binding.json';iso.write_bytes(b'fixture');binding.write_text('{}')
   with patch.object(sys,'argv',['certificate_time_gate.py','--iso',str(iso),'--binding',str(binding),'--out',name,'--check']),patch.object(gate,'replay_evidence',return_value=dict(ok=True,mode='normal',captures=3))as replay,patch.object(gate.subprocess,'Popen',side_effect=AssertionError('read-only replay')),contextlib.redirect_stdout(io.StringIO()):gate.main()
   self.assertEqual(replay.call_count,1)
 def test_failed_native_result_cannot_be_relabelled_by_raw_replay(self):
  with tempfile.TemporaryDirectory()as name:
   out=Path(name);(out/'manifest.json').write_text(json.dumps({'error':'native attempt failed'}));(out/'result.json').write_text(json.dumps({'ok':False}))
   with patch.object(gate,'frozen_binding',return_value={}),self.assertRaisesRegex(ValueError,'remains diagnostic'):gate.replay_evidence(ROOT,out/'tested.iso',out/'source-binding.json',out)
 def test_capture_excludes_only_incomplete_trailing_utf8_and_matches_later_prefix(self):
  with tempfile.TemporaryDirectory()as name:
   p=Path(name)/'serial.log';prefix='[sample] café €\r\n';scalar='🙂'.encode()
   for count in range(1,len(scalar)):
    raw=prefix.encode()+scalar[:count];p.write_bytes(raw);text=gate.clock_serial_text(p);self.assertEqual(text,prefix.replace('\r\n','\n'));self.assertEqual(p.read_bytes(),raw)
    frame=dict(serial_end=len(text),serial_sha256=gate.hashlib.sha256(text.encode()).hexdigest(),requested_anchor=None);p.write_bytes(prefix.encode()+scalar+b' done\r\n');self.assertEqual(gate.frame_prefix(gate.clock_serial_text(p),frame),text)
   p.write_bytes(b'[bad]\xff\n')
   with self.assertRaisesRegex(ValueError,'interior UTF-8'):gate.clock_serial_text(p)
 def test_every_calendar_source_and_transitive_runner_must_be_sealed(self):
  from https_asset_gate import script_dependencies
  names=set(self.c['source_sha256'])|set(script_dependencies(ROOT,('scripts/certificate_time_gate.py','scripts/tests/test_certificate_time_gate.py')));hashes={n:gate.hashlib.sha256((ROOT/n).read_bytes()).hexdigest()for n in names};held=dict(sources=hashes.copy(),runner_sha256={});self.assertEqual(gate.require_clock_inputs(ROOT,held,self.c),hashes)
  for name in('apps/kaleidoscope/src/clock_view.rs','crates/sex-time/src/lib.rs','scripts/certificate_time_gate.py','scripts/tests/test_certificate_time_gate.py','scripts/https_asset_gate.py'):
   bad=dict(sources=hashes.copy(),runner_sha256={});bad['sources'].pop(name)
   with self.subTest(name=name),self.assertRaisesRegex(ValueError,'dependency missing/changed'):gate.require_clock_inputs(ROOT,bad,self.c)

if __name__=='__main__':unittest.main()
