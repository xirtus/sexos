"""Compile actual row quarantine logic; keep foreign flows and wait math intact."""
from pathlib import Path
import subprocess
import tempfile
import unittest
import sys
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_restricted_helper_files import function


class NetPolicy(unittest.TestCase):
    def test_actual_all_task_row_flow_revocation(self):
        source = (ROOT / 'servers/sexnet/src/web.rs').read_text()
        rust = r'''
#[derive(Clone,Copy,PartialEq,Eq)]struct Identity{pd:u32,task:u32}
struct Engine<'a>{closed:Vec<(u8,usize)>,_l:core::marker::PhantomData<&'a ()>}
impl Engine<'_>{fn open_udp_close(&mut self,index:usize){self.closed.push((1,index));}}
struct Slot{owner:Option<(Identity,u8)>}
impl Slot{fn free(&mut self,index:usize,engine:Option<&mut Engine<'static>>){self.owner=None;if let Some(e)=engine{e.closed.push((0,index));}}}
struct Service{bound:[u32;2],slots:Vec<Slot>,sockets:Vec<Option<(Identity,u8)>>,resolving:Option<(Identity,u64)>}
fn program_index(pd:u32)->Option<usize>{match pd{43=>Some(0),44=>Some(1),_=>None}}
impl Service {
''' + function(source, 'pub fn quarantine(') + r'''
}
#[test]fn all_target_tasks_closed_foreign_owners_untouched(){
 let own=Identity{pd:43,task:1};let stale=Identity{pd:43,task:2};let foreign=Identity{pd:44,task:3};
 let mut state=Service{bound:[2,3],slots:vec![Slot{owner:Some((own,0))},Slot{owner:Some((stale,0))},Slot{owner:Some((foreign,0))}],sockets:vec![Some((own,0)),Some((stale,0)),Some((foreign,0))],resolving:Some((stale,1))};
 let mut engine=Engine{closed:Vec::new(),_l:core::marker::PhantomData};state.quarantine(43,Some(&mut engine));
 assert_eq!(state.bound,[0,3]);assert!(state.slots[0].owner.is_none()&&state.slots[1].owner.is_none());assert!(state.slots[2].owner.is_some());
 assert!(state.sockets[0].is_none()&&state.sockets[1].is_none()&&state.sockets[2].is_some());assert!(state.resolving.is_none());assert_eq!(engine.closed,vec![(0,0),(0,1),(1,0),(1,1)]);
 state.quarantine(43,Some(&mut engine));assert_eq!(engine.closed.len(),4);
 state.resolving=Some((foreign,1));state.quarantine(43,None);assert!(state.resolving.is_some());
}
'''
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'net.rs'
            path.write_text(rust)
            binary = Path(d) / 'tests'
            for command in (['rustc', '--edition=2021', '--test', str(path), '-o', str(binary)], [str(binary)]):
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_production_intake_egress_and_maintenance_fences(self):
        source = (ROOT / 'servers/sexnet/src/main.rs').read_text()
        self.assertLess(source.index('restriction.initialize('), source.index('    command_probe();'))
        owner = function(source, 'fn network_owner(')
        self.assertLess(owner.index('restricted::denied(caller)'), owner.index('for slot'))
        intake = source.index('        if restriction.message(')
        self.assertLess(intake, source.index('app_custody.reborn', intake))
        self.assertLess(intake, source.index('web.handle(', intake))
        self.assertIn('req.caller_task==3&&!restricted::denied(req.arg0 as u32)', source)
        self.assertIn('let pending=restriction.pending()||preparing||polling||reply_pending||seed.busy();', source)
        self.assertIn('if restricted::denied(pd)||pdx_reply(pd,word)==0', source)
        # Existing bounded wait is retained; this hook adds no wait implementation.
        self.assertIn('sex_pdx::sys_wait_self_ticks(2).expect("BOOT_SELF_WAIT_AUTHORITY")', source)
        self.assertIn('if timed_waits<=8||timed_waits.is_power_of_two()', source)


if __name__ == '__main__':
    unittest.main()
