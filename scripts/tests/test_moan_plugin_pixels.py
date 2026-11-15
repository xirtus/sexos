import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from moan_plugin_pixels import expected_status,model_view,visible_pixels
class StatusPixels(unittest.TestCase):
 def fixture(self,text='CLAP stereo swap verified',uniform=False):
  pixels=bytes([40,40,40])*32 if uniform else bytes(range(96));value=0xcbf29ce484222325
  # Own client8x4, visible rectangle x1/y1/w5/h2.
  for row in [1,2]:
   for byte in pixels[(row*8+1)*3:(row*8+6)*3]:value=((value^byte)*0x100000001b3)&((1<<64)-1)
  trace=f'[moanrack.paint] target=42 task=111 surface=359 frame=2 width=8 height=4 x=1 y=1 w=5 h=2 hash=0x{value:016x} text_hex={text.encode().hex()}\n'
  ppm=b'P6\n8 4\n255\n'+pixels
  return trace,ppm
 def view(self,trace):return model_view(trace,42,111,359,(8,4),expected_status(0))
 def test_correct_model_and_exact_rgb(self):
  trace,ppm=self.fixture();self.assertTrue(visible_pixels(ppm,(0,0,8,4),self.view(trace))['ok'])
 def test_wrong_stale_and_set_label_only_model_refused(self):
  for text in ['Wrong','Ready','Offline render running']:
   trace,ppm=self.fixture(text)
   with self.assertRaises(ValueError):self.view(trace)
  trace,_=self.fixture()
  for bad in [trace.replace('task=111','task=112'),trace.replace('width=8','width=7'),trace.replace('frame=2','frame=0')]:
   with self.assertRaises(ValueError):self.view(bad)
  with self.assertRaises(ValueError):model_view(trace,42,111,359,(8,4),expected_status(0),len(trace))
 def test_blank_or_stale_scanout_cannot_pass_valid_model_text(self):
  trace,ppm=self.fixture(uniform=True)
  with self.assertRaises(ValueError):visible_pixels(ppm,(0,0,8,4),self.view(trace))
  trace,ppm=self.fixture();bad=bytearray(ppm);point=ppm.index(b'255\n')+4+(8+1)*3;bad[point]^=1
  with self.assertRaises(ValueError):visible_pixels(bytes(bad),(0,0,8,4),self.view(trace))
 def test_latest_visible_model_is_authoritative(self):
  good,_=self.fixture();wrong,_=self.fixture('Wrong')
  with self.assertRaises(ValueError):self.view(good+wrong)
if __name__=='__main__':unittest.main()
