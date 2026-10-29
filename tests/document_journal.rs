#![allow(dead_code)]
extern crate self as sex_pdx;
pub const BLOCK_MAX_XFER:u64=4096;
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
    tear_prefix:Option<usize>,
    calls:Vec<(u8,u64,usize)>,
}
impl Disk {
    fn new() -> Self {
        Self {
            live: vec![0; CAPACITY as usize],
            durable: vec![0; CAPACITY as usize],
            op: 0,
            cut: None,
            tear: false,
            tear_prefix:None, calls:Vec::new(),
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
        self.calls.clear();
    }
}
impl Io for Disk {
    fn read(&mut self, o: u64, out: &mut [u8]) -> Result<(), u32> {
        self.calls.push((0,o,out.len()));
        self.point()?;
        out.copy_from_slice(&self.live[o as usize..o as usize + out.len()]);
        Ok(())
    }
    fn write(&mut self, o: u64, b: &[u8]) -> Result<(), u32> {
        self.calls.push((1,o,b.len()));
        if self.point().is_err() {
            if self.tear {
                let n = self.tear_prefix.unwrap_or(b.len()/2).min(b.len());
                self.durable[o as usize..o as usize + n].copy_from_slice(&b[..n]);
            }
            return Err(w::IO);
        }
        self.live[o as usize..o as usize + b.len()].copy_from_slice(b);
        Ok(())
    }
    fn sync(&mut self) -> Result<(), u32> {
        self.calls.push((2,0,0));
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

fn cooperative(d: &mut Disk, bytes: &[u8]) -> Result<journal::Receipt, u32> {
    let mut s = journal::Save::new(3, 0x40001);
    for _ in 0..1000 {
        let before = d.op;
        let r = s.step(d, bytes)?;
        assert!(d.op - before <= 1, "one logical I/O operation per step");
        if d.op>before{assert!(d.calls.last().unwrap().2<=4096,"bounded logical I/O");}
        if let Some(r) = r {
            return Ok(r);
        }
    }
    panic!("unbounded save")
}
#[test]
fn cooperative_cuts_and_tears_keep_complete_old_or_new() {
    let old = bytes(1);
    let new = bytes(9);
    let mut scratch = [0; w::MAX];
    let mut base = Disk::new();
    cooperative(&mut base, &old).unwrap();
    base.crash();
    let mut full = base.clone();
    cooperative(&mut full, &new).unwrap();
    for tear in [false, true] {
        for cut in 1..=full.op {
            let mut d = base.clone();
            d.cut = Some(cut);
            d.tear = tear;
            assert!(cooperative(&mut d, &new).is_err());
            d.crash();
            let r = journal::recover(&mut d, 3, 0x40001, &mut scratch)
                .unwrap()
                .unwrap();
            assert!(
                scratch[..r.len] == old || scratch[..r.len] == new,
                "cut {cut}"
            );
        }
    }
}
#[test]
fn cooperative_cancel_at_every_turn_preserves_recovery_and_does_not_replay() {
    let mut base = Disk::new();
    cooperative(&mut base, b"old").unwrap();
    base.crash();
    for stop in 0..45 {
        let mut d = base.clone();
        let mut s = journal::Save::new(3, 0x40001);
        let mut done = false;
        for _ in 0..stop {
            if s.step(&mut d, b"new").unwrap().is_some() {
                done = true;
                break;
            }
        }
        if done {
            assert_eq!(s.step(&mut d, b"new"), Err(w::STALE));
        }
        drop(s);
        d.crash();
        let mut scratch = [0; w::MAX];
        let r = journal::recover(&mut d, 3, 0x40001, &mut scratch)
            .unwrap()
            .unwrap();
        assert!(&scratch[..r.len] == b"old" || &scratch[..r.len] == b"new");
    }
    for b in [vec![], vec![0xED; w::MAX]] {
        cooperative(&mut base, &b).unwrap();
        base.crash();
        let mut scratch = [0; w::MAX];
        let r = journal::recover(&mut base, 3, 0x40001, &mut scratch)
            .unwrap()
            .unwrap();
        assert_eq!(&scratch[..r.len], b);
    }
}

#[test]
fn cooperative_io_error_fences_cursor_without_replaying_mutation() {
    let mut d = Disk::new();
    let mut s = journal::Save::new(3, 0x40001);
    d.cut = Some(1);
    assert_eq!(s.step(&mut d, b"hi"), Err(w::IO));
    let ops = d.op;
    d.cut = None;
    assert_eq!(s.step(&mut d, b"hi"), Err(w::STALE));
    assert_eq!(d.op, ops);
}

#[test]
fn page_chunks_keep_sector_metadata_exact_padding_and_three_flush_barriers(){
 for len in [0,1,511,512,513,4095,4096,4097,w::MAX-1,w::MAX] {
  let bytes:Vec<u8>=(0..len).map(|i|(i%251)as u8).collect();let mut d=Disk::new();
  d.live.fill(0xa5);d.durable=d.live.clone();
  let r=cooperative(&mut d,&bytes).unwrap();
  assert!(d.calls.iter().all(|c|c.2<=4096));assert_eq!(d.calls.iter().filter(|c|c.0==2).count(),3);
  let base=r.bank as usize*journal::STRIDE as usize;
  let writes:Vec<_>=d.calls.iter().filter(|c|c.0==1).collect();
  assert_eq!(writes.first().unwrap().1,base as u64);assert_eq!(writes.first().unwrap().2,512);
  assert_eq!(writes.last().unwrap().1,base as u64);assert_eq!(writes.last().unwrap().2,512);
  assert!(writes.iter().all(|c|c.1%512==0&&c.2%512==0));
  let padded=(len+511)/512*512;
  assert!(d.live[base+512+len..base+512+padded].iter().all(|b|*b==0));
  assert!(d.live[base+512+padded..base+journal::STRIDE as usize].iter().all(|b|*b==0xa5));
  d.crash();let mut scratch=[0;w::MAX];let got=journal::recover(&mut d,3,0x40001,&mut scratch).unwrap().unwrap();assert_eq!(&scratch[..got.len],bytes);
 }
}
#[test]
fn maximum_payload_cut_at_every_operation_with_partial_failed_writes_keeps_old_or_new(){
 let old:Vec<u8>=(0..w::MAX).map(|i|(i%251)as u8).collect();
 let new:Vec<u8>=(0..w::MAX-1).map(|i|((i*13+7)%251)as u8).collect();
 let mut base=Disk::new();cooperative(&mut base,&old).unwrap();base.crash();
 let mut full=base.clone();cooperative(&mut full,&new).unwrap();
 for prefix in [0,1,511,512,719,2047,2048,4095,4096] {for cut in 1..=full.op {
  let mut d=base.clone();d.cut=Some(cut);d.tear=true;d.tear_prefix=Some(prefix);
  let mut cursor=journal::Save::new(3,0x40001);let mut failed=false;
  for _ in 0..200 {match cursor.step(&mut d,&new){Err(w::IO)=>{failed=true;break},Ok(None)=>{},other=>panic!("unexpected result {other:?}")}}
  assert!(failed);let operations=d.op;assert_eq!(cursor.step(&mut d,&new),Err(w::STALE));assert_eq!(d.op,operations);
  d.crash();let mut scratch=[0;w::MAX];let r=journal::recover(&mut d,3,0x40001,&mut scratch).unwrap().unwrap();
  assert!(scratch[..r.len]==old||scratch[..r.len]==new,"cut={cut} prefix={prefix}");
 }}
}
#[test]
fn cancellation_at_every_maximum_payload_turn_preserves_complete_recovery(){
 let old=vec![0x31;w::MAX];let new=vec![0x75;w::MAX];let mut base=Disk::new();cooperative(&mut base,&old).unwrap();base.crash();
 let mut full=base.clone();let mut cursor=journal::Save::new(3,0x40001);let mut turns=0;
 loop{turns+=1;if cursor.step(&mut full,&new).unwrap().is_some(){break}assert!(turns<200);}
 for stop in 0..=turns {
  let mut d=base.clone();let mut cursor=journal::Save::new(3,0x40001);
  for _ in 0..stop {let _=cursor.step(&mut d,&new).unwrap();}drop(cursor);d.crash();
  let mut scratch=[0;w::MAX];let r=journal::recover(&mut d,3,0x40001,&mut scratch).unwrap().unwrap();
  assert!(scratch[..r.len]==old||scratch[..r.len]==new,"cancel turn={stop}");
 }
}
