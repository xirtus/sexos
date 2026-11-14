"""New passive evidence never substitutes elapsed time for a timer expiry."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
import check_self_wait as gate
from test_self_wait_source import BASE,RETURN,copy_causal_sources

REGISTRATION='[boot.service.restartable] pd=13 task=13 pages=83 entry=0x40001234\n'
MODE='[input.payload.mode] component=shell mode=proof\n'
READY='[shell.wait_observer.ready] mode=proof target=13 sys=87 period=250 limit=512\n'
HANDOFF='[bootgraph.boot.handoff] target=sexnet id=13 entry=0x40001234\n'
EXPIRY='[scheduler.self_wait.expired] task=13 deadline=200 now=200 count=16\n'

def sample(seq,before,count,after=None):
    return f'[shell.wait_observer] seq={seq} target=13 before={before} after={before if after is None else after} syscalls={count} task=0 slot=262 epoch_before=1 life_before=1 epoch_after=1 life_after=1 ok=1\n'

TRACE=BASE.replace('[boot.self_wait.sealed]',REGISTRATION+'[boot.self_wait.sealed]')+HANDOFF+MODE+READY+RETURN+EXPIRY+sample(1,201,90)+sample(2,451,100)

class PassiveProof(unittest.TestCase):
    def inspect(self,trace=TRACE,root=ROOT):return gate.inspect(trace,source_root=root)
    def test_distinct_post_expiry_continuation_not_two_tick_envelope(self):
        result=self.inspect();self.assertTrue(result['ok'],result)
        self.assertEqual(result['observed_timer_user_joins'],[])
        self.assertEqual(result['inferred_timer_user_joins'],[])
        self.assertEqual(result['timer_user_joins'],[])
        witness=result['passive_continuation_joins'][0]
        self.assertEqual(witness['evidence_type'],'source_bound_up_expiry_later_ring3_syscall_continuation')
        self.assertFalse(witness['exact_two_tick_envelope'])
        self.assertEqual(witness['baseline']['syscalls'],90)
        self.assertEqual(witness['continuation']['syscalls'],100)
    def test_no_source_unknown_source_and_wrong_role_cannot_enable(self):
        self.assertFalse(gate.inspect(TRACE)['ok'])
        for trace in (TRACE.replace('[smp.online] cores=1 present=1','[smp.online] cores=2 present=2'),
                      TRACE.replace('id=13 path=/servers/sexnet','id=14 path=/servers/sexnet'),
                      TRACE.replace(HANDOFF,''),TRACE+HANDOFF,TRACE.replace(REGISTRATION,''),TRACE+REGISTRATION,
                      TRACE.replace(REGISTRATION,'')+REGISTRATION,TRACE.replace('task=13 pages=83','task=14 pages=83'),TRACE.replace('mode=proof','mode=redacted'),
                      TRACE.replace(READY,''),TRACE+READY,TRACE.replace('target=13 before','target=14 before')):
            with self.subTest(trace=trace):self.assertFalse(self.inspect(trace)['ok'])
        with tempfile.TemporaryDirectory()as name:
            root=Path(name);copy_causal_sources(root)
            for file in gate.PASSIVE_OBSERVER_SOURCES:
                p=root/file;p.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/file,p)
            self.assertTrue(self.inspect(root=root)['ok'])
            for file in gate.PASSIVE_OBSERVER_SOURCES:
                p=root/file;original=p.read_bytes();p.write_bytes(original+b'\n// unreviewed\n')
                self.assertFalse(self.inspect(root=root)['ok'],file);p.write_bytes(original)
    def test_both_samples_need_post_expiry_offset_and_before_tick(self):
        for trace in (TRACE.replace(EXPIRY,''),TRACE.replace('deadline=200 now=200','deadline=200 now=199'),
                      TRACE.replace('deadline=200 now=200','deadline=0 now=0'),
                      TRACE.replace(EXPIRY,'').replace(HANDOFF,EXPIRY+HANDOFF),
                      TRACE.replace('task=13 deadline=200','task=14 deadline=200'),
                      TRACE.replace('before=201 after=201','before=199 after=201'),
                      TRACE.replace(sample(1,201,90),'').replace(EXPIRY,sample(1,201,90)+EXPIRY),
                      TRACE.replace(EXPIRY,'')+EXPIRY):
            with self.subTest(trace=trace):self.assertFalse(self.inspect(trace)['ok'])
    def test_equal_reset_missing_duplicate_skipped_and_backwards_samples_reject(self):
        for trace in (TRACE.replace('syscalls=100','syscalls=90'),TRACE.replace('syscalls=100','syscalls=89'),
                      TRACE.replace(sample(2,451,100),''),TRACE+sample(2,451,100),
                      TRACE.replace('seq=2','seq=3'),TRACE.replace('before=451 after=451','before=450 after=450'),
                      TRACE.replace('before=201 after=201','before=201 after=200'),
                      TRACE.replace('syscalls=100','syscalls=18446744073709551616')):
            with self.subTest(trace=trace):self.assertFalse(self.inspect(trace)['ok'])
    def test_restart_fault_nonlive_peer_route_or_task_ambiguity_reject(self):
        for trace in (TRACE.replace('epoch_after=1','epoch_after=2'),TRACE.replace('life_before=1','life_before=7'),
                      TRACE.replace('slot=262','slot=18'),TRACE.replace('task=0 slot','task=13 slot'),
                      TRACE.replace('life_after=1 ok=1','life_after=1 ok=0'),TRACE+'[service.restart] pd=13 epoch=2\n',
                      TRACE+'fault.kill [general] pd=13\n',TRACE+'[paging.fault.completed] target=13 task=13\n',
                      TRACE+'[program.worker.start] target=13 worker=99\n',TRACE+'[app.private.retired] target=13 task=13\n'):
            with self.subTest(trace=trace):self.assertFalse(self.inspect(trace)['ok'])
    def test_original_exact_join_does_not_depend_on_new_observer(self):
        original=TRACE.replace(RETURN,'').replace(EXPIRY,'[scheduler.self_wait.park] task=13 ticks=2 deadline=200 count=2\n'+EXPIRY+'[sexnet.receive.wait] before=198 after=201 ticks=2 count=2 kind=timer_assisted_poll irq=0 device=1 reply_pending=0\n')
        original=original.replace(MODE,'').replace(READY,'').replace('life_after=1 ok=1','life_after=1 ok=0')
        self.assertTrue(self.inspect(original)['ok'])
        self.assertTrue(gate.inspect(original)['ok'])

class ProductionObserver(unittest.TestCase):
    def harness(self,proof):
        source=r'''
extern crate self as sex_pdx;
use std::sync::atomic::{AtomicU64,AtomicUsize,Ordering};
static NOW:AtomicU64=AtomicU64::new(0);static READS:AtomicUsize=AtomicUsize::new(0);static PEERS:AtomicUsize=AtomicUsize::new(0);static MODE:AtomicUsize=AtomicUsize::new(0);
#[macro_export]macro_rules! serial_record {($($a:tt)*)=>{println!($($a)*)};}
pub fn get_ticks()->u64{NOW.load(Ordering::Relaxed)}
pub mod web{pub const SHELL_SLOT:u64=262;}
pub mod crash_log{
 pub struct PeerState{pub epoch:u64,pub is_live:bool}impl PeerState{pub fn live(&self)->bool{self.is_live}}
 pub fn peer_state(slot:u64)->Result<PeerState,u64>{assert_eq!(slot,262);let n=crate::PEERS.fetch_add(1,crate::Ordering::Relaxed);let m=crate::MODE.load(crate::Ordering::Relaxed);if m==3{return Err(1)}Ok(PeerState{epoch:if m==1&&n%2==1{2}else{1},is_live:m!=2})}
}
pub mod system_stats{
 pub struct Domain{pub pd:u32,pub task:u32,pub syscalls:u64}
 pub fn domain(pd:u32)->Option<Domain>{assert_eq!(pd,13);let n=crate::READS.fetch_add(1,crate::Ordering::Relaxed);Some(Domain{pd,task:0,syscalls:n as u64})}
}
#[cfg(sexos_input_payload_proof)]#[path="__MODULE__"]mod observer;
fn poll(){#[cfg(sexos_input_payload_proof)]observer::poll();}
fn main(){let mode:usize=std::env::args().nth(1).unwrap_or("0".into()).parse().unwrap();MODE.store(mode,Ordering::Relaxed);
 for i in 0..600{NOW.store(i*250,Ordering::Relaxed);poll();NOW.store(i*250+249,Ordering::Relaxed);poll();}
 assert_eq!(READS.load(Ordering::Relaxed),if cfg!(sexos_input_payload_proof){512}else{0});
 assert_eq!(PEERS.load(Ordering::Relaxed),if cfg!(sexos_input_payload_proof){1024}else{0});
}
'''.replace('__MODULE__',str(ROOT/'servers/silk-shell/src/wait_observer.rs'))
        directory=tempfile.TemporaryDirectory();root=Path(directory.name);p=root/'probe.rs';p.write_text(source);exe=root/'probe'
        command=['rustc','--edition=2021','--check-cfg=cfg(sexos_input_payload_proof)',str(p),'-o',str(exe)]
        if proof:command+=['--cfg','sexos_input_payload_proof']
        subprocess.run(command,check=True,capture_output=True,timeout=60)
        return directory,exe
    def test_actual_diagnostic_module_throttle_budget_and_epoch_barrier(self):
        directory,exe=self.harness(True)
        try:
            text=subprocess.check_output([str(exe),'0'],text=True)
            self.assertEqual(text.count('[shell.wait_observer.ready]'),1);self.assertEqual(text.count('ok=1'),512)
            for mode in ('1','2','3'):
                text=subprocess.check_output([str(exe),mode],text=True);self.assertEqual(text.count('ok=1'),0);self.assertEqual(text.count('ok=0'),512)
        finally:directory.cleanup()
    def test_normal_cfg_omits_module_and_performs_no_observer_reads(self):
        directory,exe=self.harness(False)
        try:
            self.assertEqual(subprocess.check_output([str(exe)],text=True),'')
            self.assertNotIn(b'[shell.wait_observer',exe.read_bytes())
        finally:directory.cleanup()

class ProductionAccounting(unittest.TestCase):
    def test_actual_counter_read_indexes_caller_not_query_and_never_resets_target(self):
        with tempfile.TemporaryDirectory()as name:
            directory=Path(name)
            source=(ROOT/'kernel/src/cpu_account.rs').read_text()
            hardware='fn now() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }'
            self.assertEqual(source.count(hardware),1)
            # Only the architecture-specific TSC intrinsic is stubbed for the
            # ARM host. Production count/read code and SDK wire stay intact.
            cpu=directory/'cpu.rs';cpu.write_text(source.replace(hardware,'fn now() -> u64 { 1 }'))
            program='extern crate self as sex_pdx;\npub const SLOT_SHELL:u64=6;pub fn pdx_call(_:u64,_:u64,_:u64,_:u64,_:u64)->(u64,u64){panic!("unrelated end_program API must not run")}\nuse std::sync::atomic::{AtomicU32,Ordering};\nstatic CURRENT:AtomicU32=AtomicU32::new(3);\n#[path="__STATS__"]pub mod system_stats;\npub mod ipc{pub const MAX_DOMAINS:usize=64;}\npub mod smp{pub static ONLINE:std::sync::atomic::AtomicU32=std::sync::atomic::AtomicU32::new(1);}\npub mod core_local{pub struct CoreLocal;impl CoreLocal{pub fn get()->Self{Self}pub fn current_pd(&self)->u32{crate::CURRENT.load(crate::Ordering::Relaxed)}}}\npub mod program_frames{pub fn available()->u64{0}}\npub mod app_resources{pub fn held_pages(_:u32)->Option<(u32,u64)>{None}}\npub mod interrupts{#[derive(Default)]pub struct SyscallRegs{pub rdi:u64,pub rsi:u64,pub rdx:u64,pub r10:u64,pub r8:u64}}\n#[path="__CPU__"]mod cpu_account;\nfn read(pd:u32)->u64{cpu_account::count_syscall();let mut r=interrupts::SyscallRegs{rdi:system_stats::OP_DOMAIN,rsi:pd as u64,..Default::default()};assert_eq!(cpu_account::syscall(&mut r),system_stats::OK);assert_eq!(r.r10,0);r.r8}\nfn main(){assert_eq!(read(13),0);assert_eq!(read(13),0);CURRENT.store(13,Ordering::Relaxed);cpu_account::count_syscall();CURRENT.store(3,Ordering::Relaxed);assert_eq!(read(13),1);assert_eq!(read(13),1);assert!(read(3)>=5);}\n'.replace('__STATS__',str(ROOT/'crates/sex-pdx/src/system_stats.rs')).replace('__CPU__',str(cpu))
            probe=directory/'probe.rs';probe.write_text(program);exe=directory/'probe'
            compiled=subprocess.run(['rustc','--edition=2021',str(probe),'-o',str(exe)],capture_output=True,text=True,timeout=60)
            self.assertEqual(compiled.returncode,0,compiled.stderr)
            subprocess.run([str(exe)],check=True,capture_output=True,timeout=10)

if __name__=='__main__':unittest.main()
