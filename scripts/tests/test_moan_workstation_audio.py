# SPDX-License-Identifier: GPL-3.0-only
import array, math, struct, sys, tempfile, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from moan_workstation_gate import capture_oracle
RATE=48000
class AudioOracle(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        def tone(hz,gain): return [int(gain*math.sin(2*math.pi*hz*i/RATE)) for i in range(4*RATE)]
        def decay(duration):
            # Independent time-scaled chirp/linear decay, not Moan's sampler code.
            return [int(4500*(1-i/(duration*RATE))*math.sin(2*math.pi*(40*i/(duration*RATE)+30*(i/(duration*RATE))**2))) for i in range(round(duration*RATE))]
        cls.slow=decay(.4);cls.fast=decay(.2)
        cls.samples=array.array('h',tone(220,6000)+[0]*(2*RATE)+tone(440,3000)+[0]*(2*RATE)+cls.slow+[0]*(2*RATE)+cls.fast+[0]*RATE)
    def run_audio(self,samples=None,placeholder=False,malformed=False):
        samples=self.samples if samples is None else samples
        stereo=array.array('h',(x for f in samples for x in (f,f)))
        if sys.byteorder!='little':stereo.byteswap()
        body=stereo.tobytes();header=b'RIFF'+struct.pack('<I',0 if placeholder else len(body)+36)+b'WAVEfmt '+struct.pack('<IHHIIHH',16,1,2,RATE,RATE*4,4,16)+b'data'+struct.pack('<I',0 if placeholder else len(body))
        if malformed:header=header[:40]+struct.pack('<I',len(body)-4)
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'capture.wav';p.write_bytes(header+body)
            original=p.read_bytes();result=capture_oracle(p);self.assertEqual(p.read_bytes(),original);return result
    def test_independent_scaled_decay_and_placeholder_header(self):
        normal=self.run_audio();unfinished=self.run_audio(placeholder=True)
        self.assertEqual(normal,unfinished);self.assertTrue(normal['ok'])
    def test_isolated_late_decay_island_is_not_a_fifth_onset(self):
        samples=self.samples[:]
        begin=12*RATE+int(.36*RATE);end=12*RATE+int(.37*RATE)
        samples[begin:end]=array.array('h',[0]*(end-begin))
        result=self.run_audio(samples)
        self.assertEqual(result['bursts'][2]['threshold_tail_islands'],1)
    def test_malformed_nonplaceholder_size_rejected(self):
        with self.assertRaisesRegex(AssertionError,'malformed recording lengths'):self.run_audio(malformed=True)
    def test_realistic_short_dropout_does_not_merge_into_valid_synth(self):
        samples=self.samples[:];samples[RATE:RATE+RATE//10]=array.array('h',[0]*(RATE//10))
        with self.assertRaisesRegex(AssertionError,'synth dropout'):self.run_audio(samples)
    def test_incorrect_velocity_energy_rejected(self):
        samples=self.samples[:]
        for i in range(6*RATE,10*RATE):samples[i]*=2
        with self.assertRaisesRegex(AssertionError,'velocity'):self.run_audio(samples)
    def test_additional_independent_onset_rejected(self):
        samples=self.samples[:]
        for i in range(4*RATE+RATE//2,4*RATE+RATE//2+RATE//4):samples[i]=int(2000*math.sin(2*math.pi*330*i/RATE))
        with self.assertRaisesRegex(AssertionError,'four isolated'):self.run_audio(samples)
if __name__=='__main__':unittest.main()
