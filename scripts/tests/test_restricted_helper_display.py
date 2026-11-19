"""Actual Display quiescence refuses borrowed/visible rows and preserves others."""
from pathlib import Path
import subprocess
import tempfile
import unittest
import sys
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_restricted_helper_files import function


class DisplayPolicy(unittest.TestCase):
    def test_actual_quiet_guard_and_scoped_cleanup(self):
        source = (ROOT / 'servers/sexdisplay/src/restricted.rs').read_text()
        rust = r'''
#![allow(static_mut_refs)]
#[derive(Clone,Copy,PartialEq,Eq)]struct Subject{pd:u32,task:u32}
struct Kernel;impl Kernel{fn catalog(&mut self,pd:u32)->Option<Subject>{(pd==43).then_some(Subject{pd:43,task:55})}}
mod catalog{pub struct Row{pub surface:u64}impl Row{pub fn frame(&self)->u64{self.surface}}pub fn by_target(pd:u32)->Option<Row>{(pd==43).then_some(Row{surface:360})}}
struct Scene{valid:bool}impl Scene{fn is_valid(&self)->bool{self.valid}}
struct Surface{owner_pd:u32,surface_id:u64,active:bool}
struct Stage{owner:u32,resets:u32}impl Stage{fn diagnostic(&self)->(u32,){(self.owner,)}fn reset(&mut self){self.owner=0;self.resets+=1;}}
static mut RENDER_SCENE:Scene=Scene{valid:false};
static mut SURFACES:[Surface;2]=[Surface{owner_pd:0,surface_id:0,active:false},Surface{owner_pd:44,surface_id:361,active:true}];
static mut TEXT_STAGES:[Stage;2]=[Stage{owner:43,resets:0},Stage{owner:44,resets:0}];
static mut TOKEN_BUF_OWNER:u32=43;static mut TOKEN_BUF_CALL1_RECEIVED:bool=true;
static mut TOKEN_BUF_ARG0:u64=0x1234;static mut TOKEN_BUF_ARG1:u64=0x5678;
''' + function(source, 'pub fn quiet(') + '\n' + function(source, 'fn row_busy(') + r'''
#[test]fn borrowed_scene_or_visible_target_never_ack_or_mutate(){unsafe{
 let subject=Subject{pd:43,task:55};RENDER_SCENE.valid=true;assert!(!quiet(subject));assert_eq!(TEXT_STAGES[0].resets,0);
 RENDER_SCENE.valid=false;SURFACES[0]=Surface{owner_pd:43,surface_id:777,active:true};assert!(!quiet(subject));assert_eq!(TEXT_STAGES[0].resets,0);
 SURFACES[0]=Surface{owner_pd:3,surface_id:360,active:true};assert!(!quiet(subject));
 SURFACES[0].active=false;assert!(!quiet(Subject{pd:43,task:56}));assert!(!quiet(Subject{pd:44,task:55}));
 assert!(quiet(subject));assert_eq!(TEXT_STAGES[0].resets,1);assert_eq!(TEXT_STAGES[1].resets,0);assert!(SURFACES[1].active);
 assert_eq!(TOKEN_BUF_OWNER,0);assert!(!TOKEN_BUF_CALL1_RECEIVED);assert_eq!(TOKEN_BUF_ARG0,0);assert_eq!(TOKEN_BUF_ARG1,0);
}}
#[test]fn own_content_frame_and_other_surfaces_are_distinct(){
 let s=Subject{pd:43,task:55};assert!(row_busy(s,360,460,43,999,true));assert!(row_busy(s,360,460,3,360,true));assert!(row_busy(s,360,460,3,460,true));
 assert!(!row_busy(s,360,460,44,361,true));assert!(!row_busy(s,360,460,43,360,false));
}
'''
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'display.rs'
            path.write_text(rust)
            binary = Path(d) / 'tests'
            for command in (['rustc', '--edition=2021', '--test', str(path), '-o', str(binary)], [str(binary)]):
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_production_guard_precedes_capture_and_all_mutation(self):
        source = (ROOT / 'servers/sexdisplay/src/main.rs').read_text()
        start = source.index('pub extern "C" fn _start()')
        self.assertLess(source.index('restriction.initialize(', start), source.index('display_boot::geometry()', start))
        loop = source.index('// 2. Listen for runtime')
        self.assertLess(source.index('restriction.poll(', loop), source.index('stream.poll()', loop))
        intake = source.index('if restriction.message(', loop)
        self.assertLess(intake, source.index('if msg.type_id==sex_pdx::screen_capture::OP', loop))
        self.assertLess(source.index('restricted::denied(msg.caller_pd)', intake), source.index('if !lifecycle.caller_live', intake))
        role = (ROOT / 'servers/sexdisplay/src/restricted.rs').read_text()
        self.assertNotIn('memory_region_view(', role, 'Display must not pretend to write read-only child regions')
        quiet = function(role, 'pub fn quiet(')
        self.assertLess(quiet.index('RENDER_SCENE'), quiet.index('stage.reset()'))
        self.assertLess(quiet.index('row_busy('), quiet.index('stage.reset()'))


if __name__ == '__main__':
    unittest.main()
