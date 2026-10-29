#![allow(dead_code)]
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/document.rs"]
pub mod document;
#[path = "../servers/sexfiles/src/document_journal.rs"]
mod journal;
use document as w;
use journal::{Io, CAPACITY};
#[derive(Clone)]
struct Disk {
    live: Vec<u8>,
    durable: Vec<u8>,
    op: usize,
    cut: Option<usize>,
    tear: bool,
}
impl Disk {
    fn new() -> Self {
        Self {
            live: vec![0; CAPACITY as usize],
            durable: vec![0; CAPACITY as usize],
            op: 0,
            cut: None,
            tear: false,
        }
    }
    fn point(&mut self) -> Result<(), u32> {
        self.op += 1;
        if self.cut == Some(self.op) {
            Err(w::IO)
        } else {
            Ok(())
        }
    }
    fn crash(&mut self) {
        self.live = self.durable.clone();
        self.cut = None;
        self.op = 0;
    }
}
impl Io for Disk {
    fn read(&mut self, o: u64, out: &mut [u8]) -> Result<(), u32> {
        self.point()?;
        out.copy_from_slice(&self.live[o as usize..o as usize + out.len()]);
        Ok(())
    }
    fn write(&mut self, o: u64, b: &[u8]) -> Result<(), u32> {
        if self.point().is_err() {
            if self.tear {
                let n = b.len() / 2;
                self.durable[o as usize..o as usize + n].copy_from_slice(&b[..n]);
            }
            return Err(w::IO);
        }
        self.live[o as usize..o as usize + b.len()].copy_from_slice(b);
        Ok(())
    }
    fn sync(&mut self) -> Result<(), u32> {
        self.point()?;
        self.durable = self.live.clone();
        Ok(())
    }
}
fn bytes(seed: u8) -> Vec<u8> {
    (0..1301)
        .map(|i| (i as u8).wrapping_mul(37).wrapping_add(seed))
        .collect()
}
#[test]
fn every_operation_cut_recovers_complete_old_or_new_including_torn_journal_sector() {
    let old = bytes(1);
    let new = bytes(9);
    let mut scratch = [0; w::MAX];
    let mut base = Disk::new();
    journal::save(&mut base, 3, 0x40001, &old, &mut scratch).unwrap();
    base.crash();
    let mut complete = base.clone();
    journal::save(&mut complete, 3, 0x40001, &new, &mut scratch).unwrap();
    let operations = complete.op;
    for tear in [false, true] {
        for cut in 1..=operations {
            let mut d = base.clone();
            d.cut = Some(cut);
            d.tear = tear;
            let result = journal::save(&mut d, 3, 0x40001, &new, &mut scratch);
            assert!(result.is_err(), "cut {cut}");
            d.crash();
            let r = journal::recover(&mut d, 3, 0x40001, &mut scratch)
                .unwrap()
                .unwrap();
            assert!(
                scratch[..r.len] == old || scratch[..r.len] == new,
                "cut {cut} tear {tear}"
            );
        }
    }
}
#[test]
fn empty_maximum_and_multiple_commits_round_trip() {
    let mut d = Disk::new();
    let mut scratch = [0; w::MAX];
    for (i, b) in [vec![], vec![0xFE; w::MAX], b"short".to_vec()]
        .iter()
        .enumerate()
    {
        let r = journal::save(&mut d, 4, 0x50002, b, &mut scratch).unwrap();
        assert_eq!(r.generation, i as u64 + 1);
        d.crash();
        let r = journal::recover(&mut d, 4, 0x50002, &mut scratch)
            .unwrap()
            .unwrap();
        assert_eq!(&scratch[..r.len], b);
    }
}
#[test]
fn foreign_and_recycled_identity_cannot_recover_bytes() {
    let mut d = Disk::new();
    let mut scratch = [0; w::MAX];
    journal::save(&mut d, 3, 0x40001, b"private binding", &mut scratch).unwrap();
    assert!(journal::recover(&mut d, 4, 0x50001, &mut scratch)
        .unwrap()
        .is_none());
    assert!(scratch.iter().all(|&b| b == 0));
    assert!(journal::recover(&mut d, 3, 0x40003, &mut scratch)
        .unwrap()
        .is_none());
}
#[test]
fn first_bank_conversion_keeps_legacy_prefix_until_commit() {
    let mut base = Disk::new();
    base.live[..21].copy_from_slice(b"QP01legacy-old-data!!");
    base.durable = base.live.clone();
    let mut scratch = [0; w::MAX];
    let mut full = base.clone();
    journal::save(&mut full, 2, 0x30001, b"new", &mut scratch).unwrap();
    for cut in 1..=full.op {
        let mut d = base.clone();
        d.cut = Some(cut);
        let _ = journal::save(&mut d, 2, 0x30001, b"new", &mut scratch);
        d.crash();
        assert_eq!(&d.live[..21], &base.live[..21]);
    }
}
