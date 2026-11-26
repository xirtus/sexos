# SPDX-License-Identifier: GPL-3.0-only
import pathlib,struct,sys,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from moan_recovery_pcm import FRAMES,verify
class PCM(unittest.TestCase):
 def files(self,vst=False):
  req=b'MOANREQ1'+bytes(range(1,17))+struct.pack('<4I',48000,FRAMES,2,int(vst))
  out=b'MOANPCM1'+req[8:24]+struct.pack('<4I',48000,FRAMES,2,0)
  for i in range(FRAMES):
   right=.5 if i%2==0 else -.25;req+=struct.pack('<2f',.125,right);out+=struct.pack('<2f',*( (.03125,right*.25)if vst else(right,.125)))
  if vst:req+=struct.pack('<IId',0,0,.25)
  return req,out
 def test_actual_fixture_profiles_every_sample(self):
  for vst in(False,True):
   req,out=self.files(vst);self.assertTrue(verify(req,out,vst)['every_sample_exact'])
 def test_wrong_nonce_duration_bypass_and_last_sample_fail(self):
  req,out=self.files()
  for broken in(out[:8]+bytes(16)+out[24:],out[:-8],out+bytes(8),out[:40]+req[40:],out[:-4]+struct.pack('<f',0)):
   with self.assertRaises(ValueError):verify(req,broken)
 def test_changed_input_parameter_or_nonfinite_output_fail(self):
  req,out=self.files(True)
  for badreq,badout in((req[:-8]+struct.pack('<d',.5),out),(req[:40]+bytes(8)+req[48:],out),(req,out[:-4]+struct.pack('<f',float('nan')))):
   with self.assertRaises(ValueError):verify(badreq,badout,True)
if __name__=='__main__':unittest.main()
