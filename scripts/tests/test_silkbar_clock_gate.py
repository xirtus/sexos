"""Independent public-clock model fixtures; native evidence is separate."""
from source_boot_fixture import with_inventory,TAGS
import datetime,re,sys,unittest
from unittest import mock
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import silkbar_clock_gate as gate
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2];T=1793367907

def runtime_only(text):
 tags=TAGS+('boot.app.catalog','boot.network.tls')
 return ''.join(line for line in text.splitlines(keepends=True)if not any('['+tag+']'in line for tag in tags))

@with_inventory
def fixture(state='Snapshot',width=1280,late_enqueue=False,seconds=T,profile=gate.SLIM_PROFILE):
 seq=1;value=seconds if state=='Snapshot'else 0;index={'Unknown':0,'Snapshot':1,'Unavailable':2,'Rollback':3,'Exhausted':4}[state];known=int(state in('Snapshot','Unavailable','Rollback'));observed=seconds if state=='Snapshot'else 0;attempts=int(observed>0);x=928 if width==1024 else 1090
 text='[boot.silkbar.clock.source] holder=6 task=6 slot=61 consumer=6 consumer_task=6 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0\n'
 if observed:text+=f'[clock.source.snapshot] pd=6 task=6 slot=61 seconds={seconds} attempts=1 precision=1 profile=q35_cmos_utc ordinal=1\n'
 text+=f'[silkbar.civil.sample] sequence=1 state={index} seconds={value} observed_seconds={observed} attempts={attempts} source_known={known} profile=q35_cmos_utc precision=1 snapshot=1\n[silkbar.civil.publish.try] sequence=1 state={index} seconds={value} held_display=1\n'
 enqueued=f'[silkbar.civil.enqueued] sequence=1 state={index} seconds={value} held_display=1\n'
 if not late_enqueue:text+=enqueued
 text+=f'[sexdisplay.clock.civil.applied] pd=6 task=6 sequence=1 state={state} seconds={value} high_water={value} stale=0 source_known={known} source={"q35_cmos_utc"if known else"unknown"} precision={known} snapshot_only=1 pixels=unconfirmed\n'
 if late_enqueue:text+=enqueued
 text+=draw_record(state,value,width=width,profile=profile)
 return text

def draw_record(state,seconds,stale=False,width=1280,profile=gate.SLIM_PROFILE):
 if profile==gate.SLIM_PROFILE:
  metric=f'[display.chrome.metrics] scale=1 bar=28 title=26 inset=8 body_top=34 target=24 width={width} height={768 if width==1024 else 800} publication=1\n'
  rect=f'x={width-152} y=2 w=144 h=24'
 else:metric='';rect=f'x={928 if width==1024 else 1090} y=18 w=80 h=22'
 return metric+f'[display.civil.draw] sequence=1 state={state} seconds={seconds} stale={int(stale)} pixels=written path=strip {rect}\n'

def archive_contract(current,profile=gate.LEGACY_PROFILE):
 # Explicit old 80x22/9px Sans profile for archived evidence, independent of
 # today's live chrome dimensions. Native current-profile proofs are separate.
 import geist_cells
 background=0x314772
 return dict(current,caption_profile=profile,font_profile='geist-sans-caption-v1',
             captions=geist_cells.captions(),background=background,color=0xffffff,
             dimensions=dict(CHIP_X3=1090,CHIP_Y=18,CLOCK_W=80,CHIP_H=22))

ENGLISH=('JAN','FEB','MAR','APR','MAY','JUN','JUL','AUG','SEP','OCT','NOV','DEC')

def image(c,state='Snapshot',width=1280,seconds=T,stale=False):
 h=768 if width==1024 else 800;raw=bytearray(width*h*3)
 if c['caption_profile']==gate.SLIM_PROFILE:
  import geist_cells
  if state=='Snapshot':
   date=datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc)
   text=f'{date.hour:02d}:{date.minute:02d} UTC | {date.day:02d} {ENGLISH[date.month-1]}  '
  else:text={'Unknown':'UTC stale'if stale else'UTC unknown','Unavailable':'UTC unavailable','Rollback':'UTC blocked','Exhausted':'UTC ended'}[state].ljust(20)
  alpha=geist_cells.ui_bitmap(geist_cells.ui_labels(),text,144,24,left=4,top=4)
  for yy in range(24):
   for xx in range(144):
    a=alpha[yy*144+xx];at=((2+yy)*width+width-152+xx)*3
    raw[at:at+3]=bytes((255*a+bg*(255-a))//255 for bg in(24,38,56))
  return f'P6\n{width} {h}\n255\n'.encode()+raw
 x=928 if width==1024 else 1090;color=c['color'].to_bytes(3,'big')
 if state=='Snapshot':
  date=datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc)
  first=f'UTC {date.hour:02d}:{date.minute:02d}:{date.second:02d}'
  second=f'{date.day:02d} {ENGLISH[date.month-1]} {date.year:04d}'if c['caption_profile']=='date-caption-v1'else'q35 1s snap'
 elif c['caption_profile']=='date-caption-v1':
  first,second={'Unknown':('UTC stale','Sample stale')if stale else('UTC unknown','Awaiting UTC'),'Unavailable':('UTC unknown','Source error'),'Rollback':('UTC blocked','Time blocked'),'Exhausted':('UTC ended','Updates ended')}[state]
 else:first,second={'Unknown':('UTC stale','q35 1s old')if stale else('UTC unknown','src? prec?'),'Unavailable':('UTC unknown','q35 1s fail'),'Rollback':('UTC blocked','q35 1s stop'),'Exhausted':('UTC ended','src? prec?')}[state]
 if c.get('font_profile')in('geist-sans-cells-v1','geist-sans-caption-v1'):
  chip=gate.clock_cell_pixels(c,(first,second))
  for yy in range(22):
   at=((18+yy)*width+x)*3;raw[at:at+80*3]=chip[yy*80*3:(yy+1)*80*3]
  return f'P6\n{width} {h}\n255\n'.encode()+raw
 for row,text in enumerate((first,second)):
  for col,ch in enumerate(text):
   for gy,bits in enumerate(glyph(c['font'],ch)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      at=((18+1+row*10+gy)*width+x+col*6+gx)*3;raw[at:at+3]=color
 return f'P6\n{width} {h}\n255\n'.encode()+raw

# Deliberately explicit archived caption source fixture. This is a parser
# shape test; renderer behavior is covered separately by full chip pixels.
LEGACY_LINES='''fn civil_clock_lines(clock:&silkbar_model::civil_clock::CivilClock)->[[u8;13];2]{
    use silkbar_model::civil_clock::State;
    let mut lines=[[b' ';13];2];
    let (first,second):(&[u8],&[u8])=match clock.state(){
        State::Snapshot=>(b"UTC 00:00:00",b"q35 1s snap"),
        State::Rollback=>(b"UTC blocked",b"q35 1s stop"),
        State::Exhausted=>(b"UTC ended",b"src? prec?"),
        State::Unavailable=>(b"UTC unknown",b"q35 1s fail"),
        State::Unknown if clock.is_stale()=>(b"UTC stale",b"q35 1s old"),
        State::Unknown=>(b"UTC unknown",b"src? prec?"),
    };
    lines[0][..first.len()].copy_from_slice(first);lines[1][..second.len()].copy_from_slice(second);
    if clock.state()==State::Snapshot{
        let day=clock.seconds()%86_400;
        for(at,value)in[(4,day/3600),(7,day/60%60),(10,day%60)]{
            lines[0][at]=b'0'+(value/10)as u8;lines[0][at+1]=b'0'+(value%10)as u8;
        }
    }
    lines
}
if bar.civil_enabled {return civil_clock_fg_at(x,y,bar);}
'''

class CivilClockOracle(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=gate.source_contract(ROOT)
 def test_whole_two_line_cells_at_both_supported_sizes(self):
  for width in(1024,1280):
   r=gate.verify_pixels(fixture(width=width),image(self.c,width=width),self.c,(width,768 if width==1024 else 800),'Snapshot');self.assertTrue(r['ok'],r);self.assertEqual(r['checked_pixels'],3456)
 def test_unknown_and_unavailable_remove_every_numeric_calendar_value(self):
  for state in('Unknown','Unavailable'):
   trace=fixture(state);r=gate.verify_pixels(trace,image(self.c,state),self.c,expected_state=state);self.assertTrue(r['ok'],r);self.assertFalse(gate.verify_pixels(trace,image(self.c),self.c)['ok'])
 def test_sender_return_log_may_follow_actual_receiver_admission(self):
  self.assertTrue(gate.verify_pixels(fixture(late_enqueue=True),image(self.c),self.c)['ok'])
 def test_missing_intent_wrong_sender_and_changed_receipt_fail(self):
  text=fixture()
  for bad in(text.replace('[silkbar.civil.publish.try]','[missing]'),text.replace('pd=6 task=6 sequence','pd=15 task=15 sequence'),text.replace('state=Snapshot seconds='+str(T),'state=Snapshot seconds='+str(T+1)),text.replace('[boot.silkbar.clock.source]','[missing.grant]'),text.replace('pixels=written','pixels=unconfirmed')):
   self.assertFalse(gate.verify_pixels(bad,image(self.c),self.c)['ok'])
 def test_source_or_wire_sequence_reuse_is_not_a_fresh_clock(self):
  for bad in(fixture().replace('ordinal=1','ordinal=2'),fixture().replace('sequence=1','sequence=2'),fixture().replace('observed_seconds='+str(T),'observed_seconds='+str(T+1))):self.assertFalse(gate.verify_pixels(bad,image(self.c),self.c)['ok'])
 def test_entire_chip_checks_stale_ink_beyond_both_text_rows(self):
  raw=bytearray(image(self.c));header=len(b'P6\n1280 800\n255\n');at=header+((2+23)*1280+1128+143)*3;raw[at:at+3]=b'\xff\xff\xff';self.assertFalse(gate.verify_pixels(fixture(),bytes(raw),self.c)['ok'])
 def test_real_admissions_skip_repeated_seconds_but_not_changed_text(self):
  base=fixture().replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=1\n');trace=base
  for seq,value,redraw in[(2,T,0),(3,T+1,1)]:
   part=runtime_only(fixture()).replace('sequence=1','sequence='+str(seq)).replace('ordinal=1','ordinal='+str(seq)).replace(str(T),str(value)).replace('pixels=unconfirmed\n','pixels=unconfirmed repaint='+str(redraw)+'\n')
   # A suppressed admission has no fabricated draw; a different dirty cause
   # may nevertheless really draw that same sequence later.
   if seq==2:part=part[:part.index('[display.civil.draw]')]
   trace+=part
  r=gate.repaint_metrics(trace);self.assertEqual((r['admissions'],r['clock_repaint_requests'],r['unchanged_admissions'],r['completed_civil_draws']),(3,2,1,2))
  for bad in(trace.replace('repaint=0','repaint=1'),trace.replace('repaint=1','repaint=0',1),trace.replace(' repaint=0',''),trace.replace('repaint=0','repaint=2')):
   with self.assertRaises(AssertionError):gate.repaint_metrics(bad)
 def test_actual_local_expiry_requires_same_second_recovery_redraw(self):
  base=fixture().replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=1\n')
  expiry=f'[sexdisplay.clock.civil.stale] sequence=1 high_water={T} state=Unknown seconds=0 stale=1 ticks=620 synthesized=0\n'
  next_sample=runtime_only(fixture()).replace('sequence=1','sequence=2').replace('ordinal=1','ordinal=2').replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=1\n')
  self.assertEqual(gate.repaint_metrics(base+expiry+next_sample)['clock_repaint_requests'],2)
  for bad in(expiry.replace('sequence=1','sequence=2'),expiry.replace('high_water='+str(T),'high_water=1')):
   with self.assertRaises(AssertionError):gate.repaint_metrics(base+bad+next_sample)
 def test_source_precision_row_changes_are_not_coalesced(self):
  base=fixture('Unknown').replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=0\n')
  next_sample=runtime_only(fixture('Unavailable')).replace('sequence=1','sequence=2').replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=1\n')
  self.assertEqual(gate.repaint_metrics(base+next_sample)['clock_repaint_requests'],1)
  with self.assertRaises(AssertionError):gate.repaint_metrics(base+next_sample.replace('repaint=1','repaint=0'))
 def test_unrelated_actual_draw_can_follow_suppressed_admission(self):
  base=fixture().replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=1\n')
  next_sample=runtime_only(fixture()).replace('sequence=1','sequence=2').replace('ordinal=1','ordinal=2').replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=0\n')
  self.assertEqual(gate.repaint_metrics(base+next_sample)['completed_civil_draws'],2)
  for bad in(next_sample.replace('[display.civil.draw] sequence=2','[display.civil.draw] sequence=1'),next_sample.replace('state=Snapshot seconds='+str(T)+' stale=0 pixels=written','state=Unknown seconds=0 stale=1 pixels=written')):
   with self.assertRaises(AssertionError):gate.repaint_metrics(base+bad)

 def test_independent_utc_calendar_boundaries_and_english_months(self):
  vectors=[((2020,1,1,0,0,0),'01 JAN 2020'),((2020,2,28,23,59,59),'28 FEB 2020'),((2020,2,29,0,0,0),'29 FEB 2020'),((2020,3,1,0,0,0),'01 MAR 2020'),((2024,2,29,12,34,56),'29 FEB 2024'),((2026,12,31,23,59,59),'31 DEC 2026'),((2027,1,1,0,0,0),'01 JAN 2027'),((2099,12,31,23,59,59),'31 DEC 2099')]
  for date,expected in vectors:
   seconds=int(datetime.datetime(*date,tzinfo=datetime.timezone.utc).timestamp());rows=gate.expected_clock_lines(self.c,'Snapshot',seconds)
   self.assertTrue(rows[0].endswith(expected[:6]+'  '));self.assertTrue(gate.verify_pixels(fixture(seconds=seconds),image(self.c,seconds=seconds),self.c)['ok'])
  for month,name in enumerate(ENGLISH,1):
   seconds=int(datetime.datetime(2026,month,1,tzinfo=datetime.timezone.utc).timestamp())
   self.assertEqual(gate.expected_clock_lines(self.c,'Snapshot',seconds)[0],f'00:00 UTC | 01 {name}  ')
 def test_out_of_source_range_cannot_be_rendered_as_a_snapshot(self):
  for seconds in(self.c['bounds']['MIN_SECONDS']-1,self.c['bounds']['MAX_SECONDS']+1,True):
   with self.assertRaises(AssertionError):gate.expected_clock_lines(self.c,'Snapshot',seconds)
 def test_caption_profiles_are_explicit_and_legacy_remains_readable(self):
  self.assertEqual(self.c['caption_profile'],gate.SLIM_PROFILE)
  self.assertEqual(gate._caption_profile(LEGACY_LINES),gate.LEGACY_PROFILE)
  legacy=archive_contract(self.c)
  for state in('Snapshot','Unknown','Unavailable'):
   result=gate.verify_pixels(fixture(state,profile=gate.LEGACY_PROFILE),image(legacy,state),legacy);self.assertTrue(result['ok'],result)
  self.assertFalse(gate.verify_pixels(fixture(),image(legacy),self.c)['ok'])
  self.assertFalse(gate.verify_pixels(fixture(profile=gate.LEGACY_PROFILE),image(self.c),legacy)['ok'])
 def test_unknown_hybrid_duplicate_and_uncached_source_forms_fail_closed(self):
  display=(ROOT/'servers/sexdisplay/src/main.rs').read_text()
  defects=[display.replace('b"Source error"','b"q35 1s fail"',1),display.replace('lines[1]=civil_clock_date(clock.seconds());','',1),display.replace('b"JAN"','b"JAX"',1),display.replace('text:&[u8;20]','text:&[u8;19]'),display.replace('let clock_lines=civil_clock_caption(&bar.civil_clock);','let clock_lines=[0;20];',1),display+gate._function(display,'civil_clock_lines')]
  for name in('civil_clock_fg_at_scaled','clock_fg_at_scaled','top_strip_pixel'):
   for call in('civil_clock_date(bar.civil_clock.seconds())','civil_clock_lines(&bar.civil_clock)'):
    defects.append(re.sub(r'(fn '+name+r'\b[^{]*\{)',lambda m:m[0]+'let _='+call+';',display,count=1))
  for defect in defects:
   self.assertNotEqual(defect,display)
   with self.assertRaises(AssertionError):gate._caption_profile(defect)
  for bad in(LEGACY_LINES.replace('q35 1s snap','q35 1s other'),LEGACY_LINES.replace('    lines\n}', '    civil_clock_date(clock.seconds());lines\n}')):
   with self.assertRaises(AssertionError):gate._caption_profile(bad)
 def test_calendar_dependency_is_bound_and_missing_cargo_edge_fails(self):
  for name in('servers/sexdisplay/Cargo.toml','crates/sex-time/Cargo.toml','crates/sex-time/src/lib.rs'):
   self.assertIn(name,self.c['source_sha256'])
  original=Path.read_text
  def missing_edge(path,*args,**kwargs):
   value=original(path,*args,**kwargs)
   return value.replace('sex-time =','not-sex-time =',1)if path==ROOT/'servers/sexdisplay/Cargo.toml'else value
  with mock.patch.object(Path,'read_text',missing_edge):
   with self.assertRaises(AssertionError):gate.source_contract(ROOT)
 def test_same_time_next_day_changes_date_only_for_new_profile(self):
  first=fixture().replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=1\n')
  next_part=runtime_only(fixture(seconds=T+86400)).replace('sequence=1','sequence=2').replace('ordinal=1','ordinal=2')
  modern=first+next_part.replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=1\n')
  self.assertEqual(gate.repaint_metrics(modern,self.c)['clock_repaint_requests'],2)
  legacy=archive_contract(self.c)
  old=first+next_part.replace('pixels=unconfirmed\n','pixels=unconfirmed repaint=0\n')
  self.assertEqual(gate.repaint_metrics(old,legacy)['clock_repaint_requests'],1)
  with self.assertRaises(AssertionError):gate.repaint_metrics(old,self.c)
 def test_stale_date_is_erased_over_the_entire_chip(self):
  trace=fixture()+f'[sexdisplay.clock.civil.stale] sequence=1 high_water={T} state=Unknown seconds=0 stale=1 ticks=620 synthesized=0\n'+draw_record('Unknown',0,stale=True)
  result=gate.verify_pixels(trace,image(self.c,'Unknown',stale=True),self.c);self.assertTrue(result['ok'],result)
  self.assertFalse(gate.verify_pixels(trace,image(self.c),self.c)['ok'])
 def test_every_unhealthy_caption_has_no_old_calendar_ink(self):
  # Isolate the pixel layer for otherwise terminal/long-history states. These
  # mocks do NOT claim a real rollback or 2^32 publication native scenario.
  for state,expected in(('Rollback','UTC blocked'),('Exhausted','UTC ended')):
   trace=draw_record(state,0)
   admission={1:dict(state=state,seconds=0,offset=-1,high_water=T)}
   with mock.patch.object(gate,'inspect_publications',return_value=admission):
    good=gate.verify_pixels(trace,image(self.c,state),self.c);self.assertTrue(good['ok'],good);self.assertEqual(good['lines'][0].rstrip(),expected)
    self.assertFalse(gate.verify_pixels(trace,image(self.c),self.c)['ok'])

if __name__=='__main__':unittest.main()
