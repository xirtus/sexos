"""Compile actual ticket authorization; mock only lifecycle facts and disk read."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def function(source, signature):
    start = source.index(signature)
    at = source.index('{', start)
    depth, end = 1, at + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


class PreparedTicketOwner(unittest.TestCase):
    def test_actual_task_owner_and_single_use_authorization(self):
        source = (ROOT / 'servers/sexfiles/src/fs_service.rs').read_text()
        shell = (ROOT / 'servers/silk-shell/src/main.rs').read_text()
        actual = '\n'.join(function(source, name) for name in (
            'fn bound_sender(', 'fn current_ticket_owner(', 'pub fn read_ticket('))
        actual += '\n' + function(shell, 'fn launch_caller_task(')
        prefix = r'''
extern crate self as sex_pdx;
#[macro_export] macro_rules! serial_record {($($t:tt)*)=>{};}
pub mod program_store {
 pub const BIN:&[u8]=b"/bin/";
 pub fn pack_name(_: &[u8])->Option<[u64;2]>{Some([0,0])}
}
#[derive(Debug)]pub enum Program{Absent,Failed(u32),Loaded(u32)}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]struct TicketOwner{target:u32,task:u32}
#[derive(Clone,Copy)]struct Binding{task:u32}
static mut BINDINGS:[Option<Binding>;2]=[Some(Binding{task:77}),Some(Binding{task:88})];
static mut CURRENT:[u32;2]=[77,88];
static mut LIVE:[bool;2]=[true,true];
static mut PREPARED:([u8;64],usize,u32)=([0;64],0,0);
static mut PREPARED_ARGS:([u8;64],usize)=([0;64],0);
static mut PREPARED_OWNER:Option<TicketOwner>=None;
static mut PREPARED_LAUNCHER:Option<usize>=None;
static mut LOADED:([u8;32],usize)=([0;32],0);
static mut LOADED_ARGS:([u8;64],usize)=([0;64],0);
static mut LOADED_LAUNCHER:Option<usize>=None;
fn program_index(pd:u32)->Option<usize>{match pd{42=>Some(0),43=>Some(1),_=>None}}
fn program_live(pd:u32,task:u32)->bool{program_index(pd).is_some_and(|k|unsafe{CURRENT[k]==task&&LIVE[k]})}
fn read_program(_: &[u8],_: &mut[u8],_:bool)->Program{Program::Loaded(1)}
unsafe fn seed(owner:Option<TicketOwner>,relay:Option<usize>){
 PREPARED.0[..8].copy_from_slice(b"/bin/app");PREPARED.1=8;PREPARED.2=7;
 PREPARED_OWNER=owner;PREPARED_LAUNCHER=relay;
 PREPARED_ARGS.0[..10].copy_from_slice(b"@/old-only");PREPARED_ARGS.1=10;
 LOADED_ARGS.1=0;LOADED_LAUNCHER=None;
}
fn denied(requester:u32){assert!(matches!(read_ticket(7,requester,&mut[]).0,Program::Absent));unsafe{assert_eq!(PREPARED.1,8);assert_eq!(LOADED_ARGS.1,0);}}
'''
        tests = r'''
#[test]fn exact_current_parent_spindle_and_terminal_work_but_foreign_or_reused_tasks_do_not(){unsafe{
 let owner=Some(TicketOwner{target:42,task:77});
 // A generic ticket cannot be consumed under Spindle's requester0 or another app.
 seed(owner,None);denied(0);denied(43);denied(99);
 assert!(matches!(read_ticket(7,42,&mut[]).0,Program::Loaded(1)));
 assert_eq!(LOADED_ARGS.1,10);assert!(LOADED_LAUNCHER.is_none());
 assert!(matches!(read_ticket(7,42,&mut[]).0,Program::Absent));
 // Exact old parent becomes invalid when the row changes tasks, even if bound anew.
 seed(owner,Some(0));CURRENT[0]=99;denied(42);
 BINDINGS[0]=Some(Binding{task:99});denied(42);
 CURRENT[0]=77;BINDINGS[0]=Some(Binding{task:77});
 assert!(matches!(read_ticket(7,42,&mut[]).0,Program::Loaded(1)));
 assert_eq!(LOADED_LAUNCHER,Some(0));
 // Liveness, missing bindings and reserved zero identity fail closed.
 seed(owner,None);LIVE[0]=false;denied(42);LIVE[0]=true;
 BINDINGS[0]=None;denied(42);BINDINGS[0]=Some(Binding{task:0});CURRENT[0]=0;denied(42);
 BINDINGS[0]=Some(Binding{task:77});CURRENT[0]=77;
 // Relay metadata is never allowed to override exact task ownership.
 seed(owner,Some(1));denied(42);
 seed(None,None);denied(42);assert!(matches!(read_ticket(7,0,&mut[]).0,Program::Loaded(1)));
 seed(owner,None);assert!(matches!(read_ticket((1u64<<32)|7,42,&mut[]).0,Program::Absent));
}}
#[test]fn stale_main_calls_cannot_mint_or_control_launches_and_worker_data_io_still_works(){
 assert!(bound_sender(42,77,77,true));assert!(!bound_sender(42,99,77,true));
 assert!(!bound_sender(42,0,77,true));assert!(!bound_sender(42,77,99,true));
 assert!(bound_sender(42,1234,77,false)); // current worker data I/O is unchanged
 assert!(!bound_sender(42,1234,99,false));
 assert!(!bound_sender(42,1234,0,false));
 assert!(launch_caller_task(77,77));assert!(!launch_caller_task(99,77));
 assert!(!launch_caller_task(77,99));assert!(!launch_caller_task(0,0));
}
'''
        with tempfile.TemporaryDirectory() as directory:
            src, binary = Path(directory) / 'actual.rs', Path(directory) / 'actual'
            src.write_text(prefix + actual + tests)
            for command in (['rustc', '--edition=2021', '-A', 'dead_code', '-A', 'static_mut_refs', '--test', str(src), '-o', str(binary)],
                            [str(binary), '--test-threads=1']):
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_actual_prepare_and_shell_paths_keep_owner_and_stamp(self):
        source = (ROOT / 'servers/sexfiles/src/fs_service.rs').read_text()
        handle = function(source, 'pub fn handle(')
        self.assertIn('bound_sender(caller, caller_task, b.task, r.kind == w::PREPARE)', handle)
        request = function(source, 'fn handle_request(')
        self.assertIn('current_ticket_owner(caller)', request)
        self.assertIn('run(fs, r, path, data, owner, relay)', request)
        prepare = function(source, 'fn prepare(')
        self.assertIn('PREPARED_OWNER = owner', prepare)
        self.assertNotIn('PREPARING_FOR', source)  # no stale implicit context after failed prepare
        trampoline = (ROOT / 'servers/sexfiles/src/trampoline.rs').read_text()
        self.assertIn('fs_service::handle(caller,msg.caller_task,msg.arg0,msg.arg1,msg.arg2)', trampoline)
        shell = (ROOT / 'servers/silk-shell/src/main.rs').read_text()
        launch = function(shell, 'unsafe fn shell_launch_message(')
        check = 'launch_caller_task(task,msg.caller_task)'
        load = 'pdx_call(store::SHELL_ROUTE_SLOT,store::OP,store::TICKET,ticket,caller as u64)'
        self.assertLess(launch.index(check), launch.index(load))
        run = function(shell, 'unsafe fn shell_program_message(')
        self.assertIn('program_run(msg.caller_pd).is_some_and(|(_,task)|launch_caller_task(task,msg.caller_task))', run)


if __name__ == '__main__': unittest.main()
