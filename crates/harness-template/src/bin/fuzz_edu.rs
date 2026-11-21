// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The edu driver against a hostile device: every register read is the next
//! input word (identity pinned half the time so bring-up gets through), and
//! interrupt bits arrive whenever the input says.
//!   cargo run --release -p harness-template --features fuzz --bin fuzz_edu
//!   HARNESS_FUZZ_SECONDS=60 ...           run for a minute
//!   fuzz_edu crashes/fuzz_edu-*.bin        replay a finding
use edu::{Direction, Edu, Wait, ID};
use harness::fuzz::{FuzzIrq, FuzzRegs, Src};
use harness::mock::{MockClock, MockDma};
use harness::record::Quiet;

fn run(d: &[u8]) {
    let mut head = Src::new(d);
    let flags = head.u8();
    let bits = head.u64();
    let body = &d[d.len().min(9)..];
    let mut regs = FuzzRegs::new(body);
    if flags & 1 != 0 { regs = regs.pin(ID, 0x0100_00ed); }
    let mut clock = MockClock::stepping((flags >> 4) as u64 & 3);
    let mut out = Quiet;
    let Ok(mut edu) = Edu::start(regs, &mut clock, &mut out) else { return };
    let mut irq = FuzzIrq { bits };
    let mut dma = MockDma::new(8192, 0x10_0000);
    let mut pool = dma.pool();
    let buf = pool.take(4096, 64).expect("pool");
    for i in 0..8u32 {
        let by_irq = flags & 2 != 0 && i % 2 == 1;
        let _ = if by_irq { edu.factorial(i, &mut clock, Wait::Interrupt(&mut irq), &mut out) } else { edu.factorial(i, &mut clock, Wait::<FuzzIrq>::Poll, &mut out) };
        let dir = if i % 2 == 0 { Direction::ToDevice } else { Direction::ToRam };
        let _ = edu.dma(dir, &buf, i * 300, 100 + i * 400, &mut clock, if by_irq { Wait::Interrupt(&mut irq) } else { Wait::Poll }, &mut out);
        let _ = edu.acknowledge(&mut out);
    }
}

fn seeds() -> Vec<Vec<u8>> {
    // A healthy device: liveness inverses, then "done" status and result words.
    // Identity pinned (flags bit 0), then read from the input, then by interrupt.
    let healthy = |flags: u8, id: bool| {
        let mut v = vec![flags, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let words: &[u32] = if id { &[0x0100_00ed, !0x5a5a_a5a5, !0x0f0f_f0f0] } else { &[!0x5a5a_a5a5, !0x0f0f_f0f0] };
        for w in words { v.extend_from_slice(&w.to_le_bytes()); }
        for _ in 0..64 { for w in [0u32, 120, 0, 0x101] { v.extend_from_slice(&w.to_le_bytes()); } }
        v
    };
    vec![healthy(0x01, false), healthy(0x00, true), healthy(0x03, false), vec![0x80; 64]]
}

harness::fuzz_main!(run, seeds);
