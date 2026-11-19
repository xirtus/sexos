// SPDX-License-Identifier: GPL-3.0-only
//! NVMe (apps/sexdrive: the x17's Samsung SSD): the parts of the driver that read
//! what the drive says, built from the real sources. Controller enable/disable and
//! shutdown against CSTS values from the input (stuck RDY, fatal status, all ones),
//! completion waits against a completion queue entry stream (wrong command ids,
//! flipped phase, stuck clocks), and the power-state table (APST) in the Identify
//! Controller data. The whole-driver NVMe model follows once the driver's queue
//! rework lands (see README.md).
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
#![allow(dead_code)]
#[path = "../../../apps/sexdrive/src/controller_ready.rs"] mod controller_ready;
#[path = "../../../apps/sexdrive/src/completion_wait.rs"] mod completion_wait;
#[path = "../../../apps/sexdrive/src/apst.rs"] mod apst;
use sexos_driver_fuzz::{tick, Src};
use std::cell::{Cell, RefCell};

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let mode = s.u8();
    let cap_to = s.u8() as u32;
    let rtd3e = s.u32();
    let vendor = s.u16();
    let device = s.u16();
    let skip = mode & 0x40 != 0;
    let id = s.take(4096).to_vec();
    if let Some((states, npss)) = apst::parse(&id) {
        let _ = apst::plan(&states, npss, skip || apst::skip_deepest(vendor, device));
    }
    let src = RefCell::new(s);
    let t = Cell::new(0u64);
    // The clock moves by what the input says (0 = stuck, 0xff = a huge jump);
    // past the input it runs fast, so a dead drive's timeouts end quickly.
    let ticks = || { tick(); let step = if src.borrow().is_empty() { 100 } else { src.borrow_mut().u8() as u64 }; let now = t.get().saturating_add(if step == 0xff { 1 << 40 } else { step }); t.set(now); now };
    let csts = || { tick(); if src.borrow().is_empty() { 0xffff_ffff } else { src.borrow_mut().u32() } };
    let _ = controller_ready::deadline_ticks(cap_to);
    let _ = controller_ready::wait(mode & 1 != 0, cap_to, csts, ticks, || tick());
    let _ = controller_ready::wait_shutdown(controller_ready::shutdown_ticks(rtd3e), csts, ticks, || tick());
    let cqe = || { tick(); let mut b = src.borrow_mut(); if b.is_empty() { (0, 0) } else { (b.u32(), b.u32()) } };
    let phase = (mode >> 1) as u32 & 1;
    let r = completion_wait::wait(7, phase, cqe, ticks, || { tick(); Ok(()) });
    let _ = (r.polls, r.turns, r.elapsed, r.last_dw3);
    let _ = completion_wait::wait_for(|cid| cid & 3 == 1, phase ^ 1, cqe, ticks, || { tick(); if src.borrow_mut().u8() == 0xee { Err(5) } else { Ok(()) } });
}

fn seeds() -> Vec<Vec<u8>> {
    let mut v = vec![0x01, 20, 0x40, 0x0d, 0x03, 0x00, 0x4d, 0x14, 0x08, 0xa8];
    let mut id = vec![0u8; 4096];
    id[apst::ID_NPSS] = 4;
    id[apst::ID_APSTA] = 1;
    for (i, (flags, enlat, exlat)) in [(0u8, 0u32, 0u32), (0, 0, 0), (0, 0, 0), (2, 2000, 1200), (2, 8000, 22000)].iter().enumerate() {
        let p = apst::ID_PSD + i * apst::PSD_BYTES;
        id[p + 3] = *flags;
        id[p + 4..p + 8].copy_from_slice(&enlat.to_le_bytes());
        id[p + 8..p + 12].copy_from_slice(&exlat.to_le_bytes());
    }
    v.extend_from_slice(&id);
    for i in 0..64u32 { v.extend_from_slice(&[1, 0, 0, 0]); v.extend_from_slice(&(if i > 8 { 1 } else { 0u32 }).to_le_bytes()); }
    for _ in 0..8 { v.extend_from_slice(&[0, 0, 0, 0, 7, 0, 1, 0]); }
    vec![v]
}

sexos_driver_fuzz::driver_target!(run, seeds);
