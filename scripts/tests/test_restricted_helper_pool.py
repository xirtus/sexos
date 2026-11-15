"""Compile the actual durable Shell allocator fence, not a parallel model."""
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT = Path(__file__).resolve().parents[2]
class RestrictedPool(unittest.TestCase):
    def test_actual_durable_pool(self):
        with tempfile.TemporaryDirectory() as d:
            folder=Path(d)
            source='extern crate self as sex_pdx;\n'
            for module,path in [('restricted_helper','crates/sex-pdx/src/restricted_helper.rs'),('restricted_helper_journal','crates/sex-pdx/src/restricted_helper_journal.rs'),('pool','servers/silk-shell/src/restricted_pool.rs')]:
                source+='#[path="'+str(ROOT/path)+'"] mod '+module+';\n'
            source+=r'''
use restricted_helper::Subject;use restricted_helper_journal::{Journal,Ledger};
fn s(pd:u32,task:u32)->Subject{Subject{pd,task}}
#[test]fn exact_two_permanent_rows_generation_and_restart_fence(){
 let journal=Journal::new();let mut p=pool::Pool::new();let rows=[42,43,44];assert!(!p.normal());assert!(p.reserved(42));p.initialize(&journal,Some(1),true,&rows);assert!(p.normal());
 let first=p.reserve(&journal,s(42,42),&rows).unwrap();assert_eq!(first,1);assert!(p.reserved(42));assert!(!p.reserved(44));assert_eq!(p.reserve(&journal,s(43,43),&rows),Some(2));assert_eq!(p.reserve(&journal,s(44,44),&rows),None);assert_eq!(p.reserve(&journal,s(9,9),&rows),None);
 assert!(!p.bound(&journal,s(42,41),s(42,100),first));assert!(!p.bound(&journal,s(42,42),s(43,100),first));assert!(p.bound(&journal,s(42,42),s(42,100),first));
 let mut restarted=pool::Pool::new();restarted.initialize(&journal,Some(2),true,&rows);assert!(restarted.recovering());assert!(!restarted.normal());assert_eq!(restarted.reserve(&journal,s(42,100),&rows),None);assert!(restarted.reserved(42));assert!(restarted.reserved(43));
 restarted.recovered();assert!(restarted.normal());assert_eq!(restarted.reserve(&journal,s(42,100),&rows),Some(3));assert!(restarted.reserved(42));assert_eq!(restarted.reserve(&journal,s(44,44),&rows),None);
}
#[test]fn empty_cold_checkpoint_recovers_and_missing_or_unknown_journal_denies(){
 let journal=Journal::new();let rows=[42,43];let mut p=pool::Pool::new();p.initialize(&journal,Some(1),true,&rows);let mut restarted=pool::Pool::new();restarted.initialize(&journal,Some(2),true,&rows);assert!(restarted.normal());assert!(!restarted.recovering());
 restarted.initialize(&journal,Some(2),false,&rows);assert!(!restarted.normal());assert!(!restarted.recovering());assert!(restarted.reserved(42));
 let bad=Journal::new();let mut l=Ledger::empty();l.quarantine(s(999,999),1);bad.persist(&l);let mut p=pool::Pool::new();p.initialize(&bad,Some(2),true,&rows);assert!(!p.normal());assert!(!p.recovering());assert!(p.reserved(42));p.recovered();assert!(!p.normal());
 let many=Journal::new();let mut l=Ledger::empty();for pd in [42,43,44]{l.quarantine(s(pd,pd),pd as u64);}many.persist(&l);p.initialize(&many,Some(2),true,&[42,43,44]);assert!(!p.normal());
}
'''
            (folder/'pool.rs').write_text(source)
            built=subprocess.run(['rustc','--edition=2021','--test',str(folder/'pool.rs'),'-o',str(folder/'pool')],capture_output=True,text=True)
            self.assertEqual(built.returncode,0,built.stdout+built.stderr)
            ran=subprocess.run([str(folder/'pool')],capture_output=True,text=True)
            self.assertEqual(ran.returncode,0,ran.stdout+ran.stderr)
if __name__=='__main__':unittest.main()
