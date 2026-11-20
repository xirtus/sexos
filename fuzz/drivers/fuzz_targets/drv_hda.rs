// SPDX-License-Identifier: GPL-3.0-only
//! Intel High Definition Audio (Bell's sound driver, servers/sexbell/src/hda.rs,
//! the x17's Realtek codec) against a controller and codec model. The model keeps
//! the controller honest enough to bring up (reset, CORB/RIRB rings, stream
//! descriptors), and the input decides what matters: every codec answer (widget
//! graph, connection lists, pin configs, jack sense), unsolicited responses,
//! dropped answers, which codecs exist, stream positions, and now and then a
//! register read that returns garbage. Then playback, jack following and the
//! microphone run on the routes the driver found.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
#![allow(dead_code)]
#[path = "../../../servers/sexbell/src/hda.rs"] mod hda;
#[path = "../../../servers/sexbell/src/mic.rs"] mod mic;
#[path = "../../../servers/sexbell/src/jack.rs"] mod jack;
use core::mem::size_of;

mod device_model {
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub struct Model {
        pub data: Vec<u8>,
        pub at: usize,
        pub regs: HashMap<u32, u32>,
        pub dma: (usize, usize),
        pub rirbwp: u16,
        pub corbwp: u16,
        pub codecs: u16,
        pub gcap: u16,
        pub corbrp_reset: bool,
    }
    impl Model {
        pub fn byte(&mut self) -> Option<u8> { let b = self.data.get(self.at).copied(); self.at += 1; b }
        pub fn u32(&mut self) -> u32 { u32::from_le_bytes([0; 4].map(|_| self.byte().unwrap_or(0))) }
        fn reg(&self, o: u32) -> u32 { self.regs.get(&o).copied().unwrap_or(0) }
        fn dma_ptr(&self, pa: u64, len: usize) -> Option<*mut u8> {
            let (base, n) = self.dma;
            let pa = pa as usize;
            (pa >= base && pa + len <= base + n).then_some(pa as *mut u8)
        }
    }
    thread_local! { pub static M: RefCell<Option<Model>> = const { RefCell::new(None) }; }

    pub fn read(o: u32, width: u32) -> u32 {
        sexos_driver_fuzz::tick();
        M.with(|m| {
            let mut g = m.borrow_mut();
            let m = g.as_mut().unwrap();
            let honest = match o {
                0x00 => m.gcap as u32,
                0x08 => m.reg(0x08) & 1,
                0x0e => m.codecs as u32,
                0x4a => if m.corbrp_reset { 0x8000 } else { m.corbwp as u32 },
                0x4e | 0x5e => 0x40 | (m.reg(o) & 3),
                0x58 => m.rirbwp as u32,
                0x5d => 0,
                // Stream descriptors: CTL echoes what was written; LPIB (the play
                // or record position) is whatever the controller says.
                o if o >= 0x80 && (o - 0x80) % 0x20 == 4 => m.u32(),
                o if o >= 0x80 && (o - 0x80) % 0x20 == 0 => m.reg(o),
                _ => m.reg(o),
            };
            // Now and then a read returns garbage (a flaky or hostile controller).
            let v = match m.byte() { Some(b) if b >= 0xf8 => m.u32(), _ => honest };
            if width == 4 { v } else { v & ((1u32 << (8 * width)) - 1) }
        })
    }

    pub fn write(o: u32, width: u32, v: u32) {
        M.with(|m| {
            let mut g = m.borrow_mut();
            let m = g.as_mut().unwrap();
            match o {
                0x0e => m.codecs &= !(v as u16),
                0x4a => m.corbrp_reset = v & 0x8000 != 0,
                0x58 => if v & 0x8000 != 0 { m.rirbwp = 0 },
                0x48 => {
                    // New commands: answer each into the RIRB.
                    let new = (v & 0xff) as u16;
                    let corb = m.reg(0x40) as u64 | (m.reg(0x44) as u64) << 32;
                    let rirb = m.reg(0x50) as u64 | (m.reg(0x54) as u64) << 32;
                    let mut wp = m.corbwp;
                    let mut guard = 0;
                    while wp != new && guard < 256 {
                        guard += 1;
                        wp = (wp + 1) & 0xff;
                        let Some(c) = m.dma_ptr(corb + wp as u64 * 4, 4) else { continue };
                        let verb = unsafe { (c as *const u32).read_unaligned() };
                        // Control byte: drop the answer, or send unsolicited
                        // responses (sometimes many) ahead of it.
                        let ctl = m.byte().unwrap_or(0);
                        if ctl == 0xff { continue; }
                        let unsolicited = if ctl >= 0xf0 { 1 + (ctl & 0x0f) as usize } else { 0 };
                        for k in 0..=unsolicited {
                            let answer = m.u32();
                            let extra = (verb >> 28) | if k < unsolicited { 0x10 } else { 0 };
                            m.rirbwp = (m.rirbwp + 1) & 0xff;
                            if let Some(r) = m.dma_ptr(rirb + m.rirbwp as u64 * 8, 8) {
                                unsafe { (r as *mut u32).write_unaligned(answer); (r.add(4) as *mut u32).write_unaligned(extra); }
                            }
                        }
                    }
                    m.corbwp = new;
                }
                _ => {}
            }
            let keep = if width == 4 { v } else { (m.reg(o & !3) & !(((1u32 << (8 * width)) - 1) << (8 * (o & 3)))) | v << (8 * (o & 3)) };
            if width == 4 || o & 3 == 0 { m.regs.insert(o, v); } else { m.regs.insert(o & !3, keep); m.regs.insert(o, v); }
        })
    }
}

const MMIO_LEN: usize = 0x4000;

fn run(d: &[u8]) {
    let mut s = sexos_driver_fuzz::Src::new(d);
    let gcap = s.u16();
    let codecs = s.u16();
    let rest = s.rest().to_vec();
    let mut dma = vec![0u8; 256 * 1024];
    let (base, len) = (dma.as_mut_ptr() as usize, dma.len());
    device_model::M.with(|m| *m.borrow_mut() = Some(device_model::Model {
        data: rest, at: 0, regs: Default::default(), dma: (base, len), rirbwp: 0, corbwp: 0,
        codecs, gcap, corbrp_reset: false,
    }));
    let view = sex_pdx::memory_region::RegionView { region_id: 1, virtual_address: 0x1000, length: MMIO_LEN as u64, dma_address: None };
    let pool = sex_pdx::memory_region::RegionView { region_id: 2, virtual_address: base as u64, length: len as u64, dma_address: Some(base as u64) };
    if let Ok(mut h) = hda::Hda::open(view, pool) {
        let _ = (h.has_mic(), h.mic_inputs());
        for k in 0..hda::MAX_OUTPUTS { let _ = h.output(k); }
        let mut period = [0u8; hda::PERIOD_BYTES];
        let mut mic = vec![0u8; hda::PERIOD_BYTES];
        let mut ops = device_model::M.with(|m| m.borrow_mut().as_mut().unwrap().u32());
        for _ in 0..24 {
            match ops & 7 {
                0 => { let _ = h.select(ops >> 3); }
                1 => h.follow_jack(),
                2 => { let _ = h.mic_start(); }
                3 => { let _ = h.mic_take(&mut mic); }
                4 => { let _ = h.mic_stop(); }
                5 => { if let Some(t) = h.submit(&period) { let _ = h.done(t); } }
                6 => { let _ = h.room(); }
                _ => { let _ = h.done((ops >> 8) as u16); }
            }
            period[0] = period[0].wrapping_add(1);
            ops = ops.rotate_right(3) ^ device_model::M.with(|m| m.borrow_mut().as_mut().unwrap().u32());
        }
    }
    device_model::M.with(|m| *m.borrow_mut() = None);
    drop(dma);
    let _ = size_of::<u8>();
}

/// A one-codec machine with a speaker and a headphone jack on DAC 0x02, a mic
/// pin into ADC 0x08: the codec answers in the order the driver asks.
fn seeds() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    v.extend_from_slice(&0x1100u16.to_le_bytes()); // GCAP: 1 output, 1 input stream
    v.extend_from_slice(&0x0001u16.to_le_bytes()); // codec 0
    // Plenty of plausible answers: function group at node 1 with nodes 2..0x1f.
    let answers: [u32; 12] = [0x10ec0236, 0x00010001, 0x00000001, 0x00020018, 0x00400000, 0x00000010, 0x90170110, 0x00000001, 0x00000002, 0x0021101f, 0x80000000, 0x00300000];
    for i in 0..600 { v.push(0); v.extend_from_slice(&answers[i % answers.len()].to_le_bytes()); }
    vec![v]
}

sexos_driver_fuzz::driver_target!(run, seeds);
