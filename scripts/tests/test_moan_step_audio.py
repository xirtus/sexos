# SPDX-License-Identifier: GPL-3.0-only
import array, math, struct, sys, tempfile, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from moan_step_gate import timing_oracle, stop_oracle
RATE=48000
class StepAudio(unittest.TestCase):
    def fixture(self,offset_second=0,gate=.75,last_gain=.5):
        pcm=array.array('h',[0]*(6*RATE))
        for index,step in enumerate((1,3,7,13)):
            start=step*RATE//4+(offset_second if index==1 else 0)
            n=round(RATE/4*(gate if index<3 else .5))
            gain=last_gain if index==3 else 1
            for i in range(n):pcm[start+i]=int(6000*gain*min(i/48,1)*min((n-1-i)/240,1)*math.sin(2*math.pi*261.6255653*i/RATE))
        return pcm
    def wav(self,pcm,directory):
        stereo=array.array('h',(x for value in pcm for x in (value,value)))
        if sys.byteorder!='little':stereo.byteswap()
        data=stereo.tobytes();p=Path(directory)/'capture.wav';p.write_bytes(b'RIFF'+struct.pack('<I',len(data)+36)+b'WAVEfmt '+struct.pack('<IHHIIHH',16,1,2,RATE,RATE*4,4,16)+b'data'+struct.pack('<I',len(data))+data);return p
    def test_independent_musical_onsets_pitch_gate_velocity(self):
        with tempfile.TemporaryDirectory() as d:self.assertTrue(timing_oracle(self.wav(self.fixture(),d))['ok'])
    def test_shifted_onset_rejected_even_when_notes_and_energy_match(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(AssertionError,'musical onset'):timing_oracle(self.wav(self.fixture(offset_second=480),d))
    def test_wrong_gate_and_velocity_rejected(self):
        for kwargs,reason in ((dict(gate=.6),'actual note gate'),(dict(last_gain=1),'velocity energy')):
            with tempfile.TemporaryDirectory() as d:
                with self.assertRaisesRegex(AssertionError,reason):timing_oracle(self.wav(self.fixture(**kwargs),d))
    def test_stop_allows_card_tail_but_rejects_uncancelled_prefill(self):
        marker=RATE//2
        for remaining,passes in ((.4,True),(.8,False)):
            pcm=array.array('h',[1000]*round(marker+remaining*RATE))
            with tempfile.TemporaryDirectory() as d:
                p=self.wav(pcm,d)
                if passes:self.assertTrue(stop_oracle(p,marker)['ok'])
                else:
                    with self.assertRaisesRegex(AssertionError,'cancel queued'):stop_oracle(p,marker)
    def test_stop_probe_after_entire_prebuffer_is_not_evidence(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(AssertionError,'after prebuffer'):stop_oracle(self.wav(array.array('h',[1000]*500),d),64512)
if __name__=='__main__':unittest.main()
