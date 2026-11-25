"""Fault-inject Kaleidoscope's real state-file helper with a Files stand-in."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "apps/web/src/state_file.rs"


class StateFileTest(unittest.TestCase):
    def test_interrupted_replacement_recovers_old_or_new(self):
        harness = r'''
extern crate alloc;
extern crate self as sex_app;
use std::{collections::BTreeMap, sync::{Mutex, OnceLock, atomic::{AtomicUsize, Ordering}}};
pub struct Window;
static MAP: OnceLock<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>> = OnceLock::new();
static STEP: AtomicUsize = AtomicUsize::new(0);
static STOP: AtomicUsize = AtomicUsize::new(usize::MAX);
fn map() -> &'static Mutex<BTreeMap<Vec<u8>, Vec<u8>>> { MAP.get_or_init(|| Mutex::new(BTreeMap::new())) }
fn step() { let n = STEP.fetch_add(1, Ordering::SeqCst); if n == STOP.load(Ordering::SeqCst) { panic!("power cut"); } }
pub mod files {
    use super::*;
    pub fn remove(_: &mut Window, p: &[u8]) -> Result<(), u8> { let r=map().lock().unwrap().remove(p).map(|_|()).ok_or(1); step(); r }
    pub fn write(_: &mut Window, p: &[u8], b: &[u8]) -> Result<(), u8> { map().lock().unwrap().insert(p.to_vec(), b.to_vec()); step(); Ok(()) }
    pub fn sync(_: &mut Window) -> Result<(), u8> { step(); Ok(()) }
    pub fn size(_: &mut Window, p: &[u8]) -> Result<u64, u8> { let r=map().lock().unwrap().get(p).map(|v|v.len() as u64).ok_or(1); step(); r }
    pub fn read(_: &mut Window, p: &[u8], b: &mut [u8]) -> Result<usize, u8> { let m=map().lock().unwrap(); let v=m.get(p).ok_or(1)?; b[..v.len()].copy_from_slice(v); Ok(v.len()) }
    pub fn rename(_: &mut Window, a: &[u8], b: &[u8]) -> Result<(), u8> { let mut m=map().lock().unwrap(); let v=m.remove(a).ok_or(1)?; m.insert(b.to_vec(),v); drop(m); step(); Ok(()) }
}
#[path=SOURCE] mod state_file;
#[test]
fn every_power_cut_keeps_a_complete_copy() {
    let mut w=Window;
    for at in 0..8 {
        *map().lock().unwrap()=BTreeMap::new();
        map().lock().unwrap().insert(b"Kaleidoscope/session.tsv".to_vec(),b"old".to_vec());
        STEP.store(0,Ordering::SeqCst); STOP.store(at,Ordering::SeqCst);
        let _=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state_file::replace(&mut w,"Kaleidoscope/session.tsv",b"new")));
        STOP.store(usize::MAX,Ordering::SeqCst);
        let got=state_file::read(&mut w,"Kaleidoscope/session.tsv",100).unwrap();
        assert!(got=="old" || got=="new","cut at {at}: {got:?}");
        assert!(state_file::replace(&mut w,"Kaleidoscope/session.tsv",b"latest"));
        assert_eq!(state_file::read(&mut w,"Kaleidoscope/session.tsv",100).as_deref(),Some("latest"));
    }
}
'''.replace('SOURCE', json.dumps(str(SOURCE)))
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "test.rs"
            binary = Path(temp) / "test"
            source.write_text(harness)
            subprocess.run(["rustc", "--edition=2021", "--test", str(source), "-o", str(binary)], check=True, capture_output=True, text=True)
            subprocess.run([str(binary), "--quiet"], check=True, capture_output=True, text=True)


if __name__ == "__main__":
    unittest.main()
