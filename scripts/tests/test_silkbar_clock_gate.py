"""Independent public-clock model fixtures; native evidence is separate."""
import datetime,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import silkbar_clock_gate as gate
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2];T=1793367907

def fixture(state='Snapshot',width=1280,late_enqueue=False):
 seq=1;value=T if state=='Snapshot'else 0;index={'Unknown':0,'Snapshot':1,'Unavailable':2,'Rollback':3,'Exhausted':4}[state];known=int(state in('Snapshot','Unavailable','Rollback'));observed=T if state=='Snapshot'else 0;attempts=int(observed>0);x=928 if width==1024 else 1090
 text='[boot.silkbar.clock.source] holder=6 task=6 slot=61 consumer=6 consumer_task=6 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0\n'
 if observed:text+=f'[clock.source.snapshot] pd=6 task=6 slot=61 seconds={T} attempts=1 precision=1 profile=q35_cmos_utc ordinal=1\n'
 text+=f'[silkbar.civil.sample] sequence=1 state={index} seconds={value} observed_seconds={observed} attempts={attempts} source_known={known} profile=q35_cmos_utc precision=1 snapshot=1\n[silkbar.civil.publish.try] sequence=1 state={index} seconds={value} held_display=1\n'
 enqueued=f'[silkbar.civil.enqueued] sequence=1 state={index} seconds={value} held_display=1\n'
 if not late_enqueue:text+=enqueued
 text+=f'[sexdisplay.clock.civil.applied] pd=6 task=6 sequence=1 state={state} seconds={value} high_water={value} stale=0 source_known={known} source={"q35_cmos_utc"if known else"unknown"} precision={known} snapshot_only=1 pixels=unconfirmed\n'
 if late_enqueue:text+=enqueued
 text+=f'[display.civil.draw] sequence=1 state={state} seconds={value} stale=0 pixels=written path=strip x={x} y=18 w=80 h=22\n'
 return text

def image(c,state='Snapshot',width=1280):
 h=768 if width==1024 else 800;raw=bytearray(width*h*3);x=928 if width==1024 else 1090;color=c['color'].to_bytes(3,'big');first='UTC '+datetime.datetime.fromtimestamp(T,datetime.timezone.utc).strftime('%H:%M:%S')if state=='Snapshot'else'UTC unknown';second={'Snapshot':'q35 1s snap','Unavailable':'q35 1s fail','Unknown':'src? prec?'}[state]
 for row,text in enumerate((first,second)):
  for col,ch in enumerate(text):
   for gy,bits in enumerate(glyph(c['font'],ch)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      at=((18+1+row*10+gy)*width+x+col*6+gx)*3;raw[at:at+3]=color
 return f'P6\n{width} {h}\n255\n'.encode()+raw

class CivilClockOracle(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=gate.source_contract(ROOT)
 def test_whole_two_line_cells_at_both_supported_sizes(self):
  for width in(1024,1280):
   r=gate.verify_pixels(fixture(width=width),image(self.c,width=width),self.c,(width,768 if width==1024 else 800),'Snapshot');self.assertTrue(r['ok'],r);self.assertEqual(r['checked_pixels'],1760)
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
  raw=bytearray(image(self.c));header=len(b'P6\n1280 800\n255\n');at=header+((18+20)*1280+1090+79)*3;raw[at:at+3]=self.c['color'].to_bytes(3,'big');self.assertFalse(gate.verify_pixels(fixture(),bytes(raw),self.c)['ok'])

if __name__=='__main__':unittest.main()
