// SPDX-License-Identifier: GPL-3.0-only
//! Intel Tiger Lake display (the x17's iGPU panel and the Type-C/HDMI ports):
//! every register read answers with the next fuzz word, so status bits flap, AUX
//! and GMBUS transactions fail half way, pipes report absurd timings and
//! monitors send broken EDID. Covers AUX/DPCD, GMBUS EDID, pipe readout to the
//! KMS model, hotplug, backlight, page flips, vblank and the cursor plane.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_gpu_intel::{aux, backlight, gmbus, hotplug, plane, readout, Mmio};
use sex_kms::edid::Edid;
use sexos_driver_fuzz::FuzzRegs;

struct Dev<'a>(FuzzRegs<'a>);
impl Mmio for Dev<'_> {
    fn read(&mut self, _offset: u32) -> u32 { self.0.read32() }
    fn write(&mut self, _offset: u32, _value: u32) { self.0.write32() }
}

fn run(d: &[u8]) {
    let mut m = Dev(FuzzRegs::new(d));
    let mut ports = Vec::new();
    for _ in 0..12 {
        if m.0.src.is_empty() { break; }
        let port = m.0.src.below(10) as u32;
        let p = m.0.src.below(4) as u32;
        let mut edid = [0u8; 256];
        match m.0.src.u8() % 12 {
            0 => if let Ok(n) = aux::read_edid(&mut m, port, &mut edid) {
                let e = Edid::parse(&edid[..n.min(256)]).ok();
                ports.push(readout::Port { port, kind: readout::kind(port, 0, Some(0)), live: true, edid: e });
            },
            1 => { let _ = aux::caps(&mut m, port, port == 0); }
            2 => if let Ok(n) = gmbus::read_edid(&mut m, port, &mut edid) { let _ = Edid::parse(&edid[..n.min(256)]); },
            3 => { let mut buf = [0u8; 16]; let _ = aux::dpcd_read(&mut m, port, 0x200, &mut buf); }
            4 => { let _ = hotplug::live(&mut m, port); let _ = hotplug::scan(&mut m); }
            5 => { let b = backlight::read(&mut m); let _ = b.percent(); let _ = backlight::set_percent(&mut m, p * 33); }
            6 => { let _ = plane::flip(&mut m, p, 0x10000); let _ = plane::flip_done(&mut m, p, 0x10000); }
            7 => { let _ = plane::wait_vblank(&mut m, p, 1000); let _ = plane::scanline(&mut m, p); }
            8 => { let _ = plane::cursor_set(&mut m, p, 64, 0x20000, -5, 7); plane::cursor_move(&mut m, p, 3000, -3000); plane::cursor_off(&mut m, p); }
            9 => { let s = plane::Surface { ggtt: 0x40000, stride_bytes: 7680, width: 1920, height: 1080 }; let _ = plane::set_primary(&mut m, p, &s, 0, 0); let _ = plane::wait_pipe_on(&mut m, p, 1000); }
            10 => { let _ = aux::dpcd_write(&mut m, port, 0x600, &[1]); }
            _ => {
                let pipes = readout::read_all(&mut m);
                for r in &pipes { let _ = r.plane.linear_xrgb(); let _ = readout::running_mode(r, ports.first().and_then(|p: &readout::Port| p.edid.as_ref()), 60_000); }
                let measured: Vec<u32> = pipes.iter().map(|_| 59_940).collect();
                let _ = readout::to_kms(&pipes, &ports, &measured);
                let _ = readout::refresh_mhz(m.0.src.u32() as u64, m.0.src.u32() as u64);
            }
        }
    }
    // The EDID parser directly, on the whole input (monitors are hostile too).
    let _ = Edid::parse(d);
    let _ = sex_kms::edid::detailed(d);
}

fn seeds() -> Vec<Vec<u8>> {
    // A plausible 1920x1080 EDID block, then register words that mostly say "done".
    let mut e = vec![0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x4d, 0x10, 0x6f, 0x14, 0, 0, 0, 0, 0x01, 0x1e, 0x01, 0x04, 0xa5, 0x22, 0x13, 0x78, 0x07];
    e.resize(54, 0);
    e.extend_from_slice(&[0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c, 0x45, 0x00, 0x58, 0xc2, 0x10, 0x00, 0x00, 0x1e]);
    e.resize(127, 0);
    let sum = e.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    e.push(0u8.wrapping_sub(sum));
    let mut regs = Vec::new();
    for _ in 0..512 { regs.extend_from_slice(&0x4000_0000u32.to_le_bytes()); regs.extend_from_slice(&0xffff_ffffu32.to_le_bytes()); }
    vec![e, regs]
}

sexos_driver_fuzz::driver_target!(run, seeds);
