"""Actual periodic Bell predicates cannot read a restricted native-owned ring."""
from pathlib import Path
import subprocess
import tempfile
import unittest
import sys
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_restricted_helper_files import function


class BellPolicy(unittest.TestCase):
    def test_actual_streaming_and_mic_predicates_short_circuit_before_ring_read(self):
        source = (ROOT / 'servers/sexbell/src/sound.rs').read_text()
        rust = r'''
mod restricted{pub fn denied(pd:u32)->bool{pd==43}}
mod ring{pub const PAUSED:usize=0;pub const LISTEN:usize=1;}
#[derive(Clone,Copy)]struct Ring{forbidden:bool}
impl Ring{fn load(self,field:usize)->u64{assert!(!self.forbidden,"restricted ring read");if field==ring::LISTEN{1}else{0}}}
#[derive(Clone,Copy)]struct Stream{pd:u32,active:bool,ring:Ring}
struct Mixer{mic:[bool;2]}fn settings()->Mixer{Mixer{mic:[true;2]}}
struct Player{streams:[Option<Stream>;2]}
impl Player {
''' + function(source, 'fn streaming(') + '\n' + function(source, 'fn listener(') + r'''
}
#[test]fn adversarial_play_listen_bits_never_read_for_restricted_row(){
 let restricted=Stream{pd:43,active:true,ring:Ring{forbidden:true}};
 let normal=Stream{pd:44,active:true,ring:Ring{forbidden:false}};
 let mut p=Player{streams:[Some(restricted),None]};assert!(!p.streaming());assert!(!p.listener(0));
 p.streams[1]=Some(normal);assert!(p.streaming());assert!(p.listener(1));assert!(!p.listener(0));
}
'''
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'bell.rs'
            path.write_text(rust)
            binary = Path(d) / 'tests'
            for command in (['rustc', '--edition=2021', '--test', str(path), '-o', str(binary)], [str(binary)]):
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_production_loop_capture_and_full_scrub_are_fenced(self):
        source = (ROOT / 'servers/sexbell/src/main.rs').read_text()
        sound = (ROOT / 'servers/sexbell/src/sound.rs').read_text()
        self.assertLess(source.index('restriction.initialize('), source.index('    sound::init();'))
        self.assertLess(source.index('restriction.message('), source.index('app_custody.reborn'))
        self.assertLess(source.index('restricted::denied(msg.caller_pd)'), source.index('sound::mixer_request('))
        self.assertIn('if !sound::busy()&&!restriction.pending()', source)
        self.assertIn('if sound::busy()||restriction.pending()', source)
        ring_of = function(sound, 'fn ring_of(')
        self.assertLess(ring_of.index('restricted::denied(caller)'), ring_of.index('sex_pdx::sound_ring_bell(k)'))
        render = function(sound, 'fn render(')
        self.assertLess(render.index('restricted::denied(st.pd)'), render.index('r.load(ring::WRITTEN)'))
        mic = function(sound, 'fn listen_pump(')
        self.assertLess(mic.index('restricted::denied(st.pd)'), mic.index('st.ring.load(ring::LISTEN)'))
        self.assertIn('if crate::restricted::denied(st.pd){continue;} st.ring.store', function(sound, 'fn pump('))
        self.assertIn('if crate::restricted::row_denied(a as usize)', sound)
        clean = function(sound, 'pub fn quarantine(')
        self.assertIn('p.streams[index]=None', clean)
        self.assertIn('period[index]=0', clean)
        # Full grant wipe is separate from legacy Player/card availability.
        self.assertLess(source.index('sound::quarantine(subject.pd)'), source.index('restricted::scrub(subject)'))


if __name__ == '__main__':
    unittest.main()
