// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The whole GSP-RM boot against a simulated GA104: the simulation checks
//! what real hardware checks (signatures patched where the boot ROM looks,
//! FWSEC told to run FRTS at the right place, Booter handed a valid
//! `GspFwWprMeta` whose radix3 table really maps the GSP-RM image, the first
//! RPCs queued with good checksums) and answers like the GPU would.
mod common;
use common::*;
use sex_gpu_nv::boot::{self, Config, Firmware, Memory, Step};
use sex_gpu_nv::regs::{self, falcon as F};
use sex_gpu_nv::{chip::Chip, msgq, rpc, Clock, Dma, Error, Mmio};
use std::collections::HashMap;

const ARENA_BUS: u64 = 0x2_0000_0000;

struct Falcon { imem: Vec<u8>, dmem: Vec<u8>, ran: u32 }
impl Falcon { fn new() -> Self { Falcon { imem: vec![0; 0x10000], dmem: vec![0; 0x10000], ran: 0 } } }

struct Sim {
    regs: HashMap<u32, u32>,
    rom: Vec<u8>,
    arena: *mut u8,
    arena_len: usize,
    gsp: Falcon,
    sec2: Falcon,
    rm_image: Vec<u8>,
    doorbells: u32,
    fwsec_fuse: u32,
    fail_booter: bool,
    send_init_done: bool,
    notes: Vec<String>,
}

impl Sim {
    fn bus(&self, addr: u64, len: usize) -> &[u8] {
        let o = (addr - ARENA_BUS) as usize;
        assert!(o + len <= self.arena_len, "DMA outside system memory: {addr:#x}+{len:#x}");
        // SAFETY: test-only view of the arena the boot borrows; reads are
        // volatile-equivalent because each one goes through a fresh slice.
        unsafe { std::slice::from_raw_parts(self.arena.add(o), len) }
    }
    fn bus_mut(&mut self, addr: u64, len: usize) -> &mut [u8] {
        let o = (addr - ARENA_BUS) as usize;
        assert!(o + len <= self.arena_len);
        unsafe { std::slice::from_raw_parts_mut(self.arena.add(o), len) }
    }
    fn reg(&self, a: u32) -> u32 { *self.regs.get(&a).unwrap_or(&0) }
    fn which(&self, a: u32) -> Option<(u32, u32)> {
        for b in [regs::GSP_BASE, regs::SEC2_BASE] { if (b..b + 0x2000).contains(&a) { return Some((b, a - b)); } }
        None
    }

    fn dma(&mut self, base: u32) {
        let f = |r| self.reg(base + r);
        let cmd = f(F::DMATRFCMD);
        let src = ((f(F::DMATRFBASE) as u64) << 8) + f(F::DMATRFFBOFFS) as u64;
        let dst = f(F::DMATRFMOFFS) as usize;
        assert_eq!((cmd >> 8) & 7, 6, "256-byte transfers");
        assert_eq!(self.reg(base + F::FBIF_TRANSCFG0) & 7, 5, "physical sysmem context");
        let data = self.bus(src, 256).to_vec();
        let fal = if base == regs::GSP_BASE { &mut self.gsp } else { &mut self.sec2 };
        let mem = if cmd & 0x10 != 0 { &mut fal.imem } else { &mut fal.dmem };
        mem[dst..dst + 256].copy_from_slice(&data);
    }

    fn start(&mut self, base: u32) {
        assert_eq!(self.reg(base + F::ADDR2 + F::BROM_MOD_SEL), 1, "heavy-secure boot goes through the boot ROM");
        if base == regs::GSP_BASE { self.run_fwsec() } else { self.run_booter() }
        let c = self.reg(base + F::CPUCTL);
        self.regs.insert(base + F::CPUCTL, c | F::CPUCTL_HALTED);
    }

    fn run_fwsec(&mut self) {
        self.gsp.ran += 1;
        let a2 = regs::GSP_BASE + F::ADDR2;
        assert_eq!((self.reg(a2 + F::BROM_ENGIDMASK), self.reg(a2 + F::BROM_CURR_UCODE_ID)), (0x400, 1));
        let pkc = self.reg(a2 + F::BROM_PARAADDR0) as usize;
        assert_eq!(pkc, FWSEC_PKC);
        let want = if self.fwsec_fuse == 0 { 0xa0 } else { 0xa1 };
        assert!(self.gsp.dmem[pkc..pkc + SIG].iter().all(|&b| b == want), "FWSEC signature for the fuse");
        assert_eq!(self.gsp.imem[0], 0x5a, "FWSEC code in IMEM");
        let d = &self.gsp.dmem;
        assert_eq!(r32(d, FWSEC_MAPPER + 0x2c), 0x15, "FRTS command");
        let cb = FWSEC_CMDBUF + 24;
        let (addr, size) = ((r32(d, cb + 8) as u64) << 12, (r32(d, cb + 12) as u64) << 12);
        assert_eq!(size, 0x10_0000);
        assert_eq!(addr, (8u64 << 30) - 0x20_0000);
        // WPR2 = [heap bottom .. FRTS end); FWSEC only knows FRTS, so the lo
        // bound here is a stand-in, as on hardware the RM computes the rest.
        self.regs.insert(regs::PFB_WPR2_ADDR_LO, (((addr - (160 << 20)) >> 12) << 4) as u32);
        self.regs.insert(regs::PFB_WPR2_ADDR_HI, (((addr + size - 0x1000) >> 12) << 4) as u32);
        self.regs.insert(regs::GSP_BASE + F::MAILBOX0, 0);
        self.notes.push(format!("fwsec frts at {addr:#x}"));
    }

    fn run_booter(&mut self) {
        self.sec2.ran += 1;
        let a2 = regs::SEC2_BASE + F::ADDR2;
        assert_eq!((self.reg(a2 + F::BROM_ENGIDMASK), self.reg(a2 + F::BROM_CURR_UCODE_ID)), (1, 3));
        let sign = self.reg(a2 + F::BROM_PARAADDR0) as usize;
        assert!(self.sec2.dmem[sign..sign + SIG].iter().all(|&b| b == 0xb0), "Booter signature for fuse 1");
        // Booter gets GspFwWprMeta's address in the mailboxes.
        let meta_bus = self.reg(regs::SEC2_BASE + F::MAILBOX0) as u64 | (self.reg(regs::SEC2_BASE + F::MAILBOX1) as u64) << 32;
        let meta = self.bus(meta_bus, 256).to_vec();
        assert_eq!(r64(&meta, 0), 0xdc3aae21371a60b3, "WPR meta magic");
        assert!(self.reg(regs::PFB_WPR2_ADDR_HI) != 0, "FWSEC ran first");
        // Walk radix3 and compare every page with the GSP-RM image.
        let (l0, size) = (r64(&meta, 16), r64(&meta, 24) as usize);
        let l1 = r64(self.bus(l0, 8), 0);
        let pages = size.div_ceil(4096);
        for p in 0..pages {
            let l2 = r64(self.bus(l1 + (p / 512 * 8) as u64, 8), 0);
            let page = r64(self.bus(l2 + (p % 512 * 8) as u64, 8), 0);
            assert_eq!(self.bus(page, 4096), &self.rm_image[p * 4096..(p + 1) * 4096], "radix3 page {p}");
        }
        assert!(self.bus(r64(&meta, 72), r64(&meta, 80) as usize).iter().all(|&b| b == 0x5e), "GSP-RM signature");
        assert_eq!(r64(&meta, 40), 0x1000, "bootloader size");
        assert!(self.bus(r64(&meta, 32), 0x1000).iter().all(|&b| b == 0x11), "bootloader image");
        // GSP must be in RISC-V mode, pointing at LibOS args.
        assert_eq!(self.reg(regs::GSP_BASE + F::ADDR2 + F::RISCV_BCR_CTRL) & 0x111, 0x111);
        let libos = self.reg(regs::GSP_BASE + F::MAILBOX0) as u64 | (self.reg(regs::GSP_BASE + F::MAILBOX1) as u64) << 32;
        let args = self.bus(libos, 128).to_vec();
        let names: Vec<u64> = (0..4).map(|i| r64(&args, i * 32)).collect();
        assert_eq!(names, ["LOGINIT", "LOGINTR", "LOGRM", "RMARGS"].map(sex_gpu_nv::libos::id8));
        let rmargs = self.bus(r64(&args, 3 * 32 + 8), 80).to_vec();
        let shm = r64(&rmargs, 0);
        let l = msgq::shared_layout();
        assert_eq!((r32(&rmargs, 8), r64(&rmargs, 16) as usize, r64(&rmargs, 24) as usize), (l.pte_count, l.cmdq_off, l.msgq_off));
        // The two RPCs GSP-RM reads at start.
        let q = self.bus(shm, l.total).to_vec();
        assert_eq!(r32(&q, l.cmdq_off + 16), 2, "two command elements");
        for (slot, f) in [(0usize, rpc::FN_GSP_SET_SYSTEM_INFO), (1, rpc::FN_SET_REGISTRY)] {
            let e = &q[l.cmdq_off + 0x1000 + slot * 0x1000..][..0x1000];
            assert_eq!(msgq::checksum(e), 0);
            assert_eq!((r32(e, 36), r32(e, 40), &e[52..56], r32(e, 60)), (slot as u32, 1, &b"VRPC"[..], f));
        }
        assert!(self.doorbells >= 1);
        if self.fail_booter { self.regs.insert(regs::SEC2_BASE + F::MAILBOX0, 0xbad); return; }
        self.regs.insert(regs::SEC2_BASE + F::MAILBOX0, 0);
        self.regs.insert(regs::GSP_BASE + F::ADDR2 + F::RISCV_CPUCTL, F::RISCV_CPUCTL_ACTIVE);
        // GSP-RM starts, asks for a sequencer run, then says it is ready.
        let mut seq = vec![0u8; 40 + 6 * 4];
        w32(&mut seq, 0, 16); w32(&mut seq, 4, 5);
        let cmd = [0u32, 0x1234, 0xcafe, 3, 5, 0];
        for (i, v) in cmd.iter().enumerate() { w32(&mut seq, 40 + i * 4, *v); }
        let mut slot = 0;
        self.gsp_send(shm, &mut slot, 0x100c, b"log line");
        self.gsp_send(shm, &mut slot, rpc::EV_GSP_RUN_CPU_SEQUENCER, &seq);
        if self.send_init_done { self.gsp_send(shm, &mut slot, rpc::EV_GSP_INIT_DONE, &[0u8; 8]); }
    }

    fn gsp_send(&mut self, shm: u64, slot: &mut u32, function: u32, body: &[u8]) {
        let l = msgq::shared_layout();
        let at = shm + (l.msgq_off + 0x1000 + *slot as usize * 0x1000) as u64;
        let e = self.bus_mut(at, 0x1000);
        e.fill(0);
        w32(e, 40, 1);
        w32(e, 48, 0x0300_0000); e[52..56].copy_from_slice(b"VRPC"); w32(e, 56, 32 + body.len() as u32); w32(e, 60, function);
        e[80..80 + body.len()].copy_from_slice(body);
        *slot += 1;
        let s = *slot;
        w32(self.bus_mut(shm + l.msgq_off as u64 + 16, 4), 0, s);
    }
}

impl Mmio for Sim {
    fn read(&mut self, a: u32) -> u32 {
        if (regs::PROM..regs::PROM + regs::PROM_SIZE).contains(&a) {
            assert_eq!(self.reg(regs::PCI_ROM_SHADOW) & 1, 0, "ROM read with the shadow on");
            let i = (a - regs::PROM) as usize;
            return if i + 4 <= self.rom.len() { r32(&self.rom, i) } else { 0xffff_ffff };
        }
        match a {
            regs::PMC_BOOT_0 => 0x174000a1,
            regs::PFB_VIDMEM_SIZE_MB => 8192,
            0x824148 => 1,              // SEC2 ucode 3 fuse
            0x8241c0 => self.fwsec_fuse, // GSP ucode 1 fuse
            _ => match self.which(a) {
                Some((_, F::HWCFG2)) => F::HWCFG2_RESET_READY,
                Some((b, F::DMATRFCMD)) => self.reg(b + F::DMATRFCMD) | F::DMATRFCMD_IDLE,
                _ => self.reg(a),
            },
        }
    }
    fn write(&mut self, a: u32, v: u32) {
        self.regs.insert(a, v);
        match self.which(a) {
            Some((b, F::DMATRFCMD)) => self.dma(b),
            Some((b, F::CPUCTL)) if v & F::CPUCTL_STARTCPU != 0 => self.start(b),
            Some((b, F::QUEUE_HEAD0)) if b == regs::GSP_BASE => self.doorbells += 1,
            Some((b, F::ENGINE_RESET)) if v & 1 != 0 => {
                // A reset clears the BROM setup and the halted bit.
                for r in [F::ADDR2 + F::BROM_MOD_SEL, F::CPUCTL] { self.regs.remove(&(b + r)); }
            }
            _ => {}
        }
    }
}

struct Ticks(u64);
impl Clock for Ticks { fn delay_us(&mut self, us: u32) { self.0 += us as u64; } }

struct Carve<'a> { rest: &'a mut [u8], bus: u64 }
impl<'a> Carve<'a> {
    fn take(&mut self, n: usize) -> Dma<'a> {
        let n = n.div_ceil(4096) * 4096;
        let rest = std::mem::take(&mut self.rest);
        let (a, b) = rest.split_at_mut(n);
        self.rest = b;
        let d = Dma::new(a, self.bus);
        self.bus += n as u64;
        d
    }
}

fn run(fwsec_fuse: u32, fail_booter: bool, send_init_done: bool) -> (Result<boot::Report, boot::Failed>, Sim, Vec<Step>, Ticks) {
    let (bl, bt, gsp) = (bootloader(), booter(), gsp_elf(700)); // 700 pages: radix3 level 2 spans two pages
    let f = Firmware { booter_load: &bt, bootloader: &bl, gsp: &gsp };
    let chip = Chip::decode(0x174000a1).unwrap();
    let sizes = boot::Sizes::for_firmware(&chip, &f).unwrap();
    let mut arena = vec![0u8; sizes.total() + 64 * 4096];
    let mut sim = Sim {
        regs: HashMap::new(), rom: vbios(), arena: arena.as_mut_ptr(), arena_len: arena.len(),
        gsp: Falcon::new(), sec2: Falcon::new(), rm_image: sex_gpu_nv::fw::gsp_rm(&gsp, ".fwsignature_ga10x").unwrap().image.to_vec(),
        doorbells: 0, fwsec_fuse, fail_booter, send_init_done, notes: vec![],
    };
    sim.regs.insert(regs::PCI_ROM_SHADOW, 1);
    let mut c = Carve { rest: &mut arena, bus: ARENA_BUS };
    let mut scratch = vec![0u8; 2 * msgq::MAX_MSG];
    let mut mem = Memory {
        fwsec: c.take(sizes.fwsec_max), booter: c.take(sizes.booter), rm_image: c.take(sizes.rm_image),
        radix3_l0: c.take(4096), radix3_l1: c.take(4096), radix3_l2: c.take(sizes.radix3_l2),
        signature: c.take(sizes.signature), bootloader: c.take(sizes.bootloader), wpr_meta: c.take(4096),
        libos: c.take(4096), loginit: c.take(0x10000), logintr: c.take(0x10000), logrm: c.take(0x10000),
        rmargs: c.take(4096), shm: c.take(sizes.shm), scratch: &mut scratch,
    };
    let cfg = Config { system: rpc::SystemInfo { bar0: 0xa000_0000, bar1: 0x40_0000_0000, bar_inst: 0x42_0000_0000, bus: 1, devfn: 0, max_user_va: 0x7fff_ffff_f000 }, init_polls: 1000 };
    let mut vb = vec![0u8; 1 << 20];
    let mut steps = vec![];
    let mut t = Ticks(0);
    let r = boot::boot(&mut sim, &mut t, &cfg, &f, &mut vb, &mut mem, &mut |s: Step, _: &boot::Report| steps.push(s));
    (r, sim, steps, t)
}

#[test]
fn boots_to_init_done() {
    let (r, sim, steps, _) = run(0, false, true);
    let r = r.unwrap();
    assert_eq!(steps, [Step::Chip, Step::Vbios, Step::Layout, Step::Prepare, Step::Fwsec, Step::Riscv, Step::Booter, Step::InitDone]);
    assert!(r.init_done);
    assert_eq!((r.chipset, r.fb_size, r.vbios_version), (0x174, 8 << 30, [0x94, 0x30, 0x04, 0x05, 0x07]));
    assert_eq!((r.fwsec_sig, r.booter_sig, r.events, r.sequencer_runs), (0, 0, 3, 1));
    assert_eq!(sim.reg(0x1234), 0xcafe, "sequencer REG_WRITE ran");
    assert_eq!((sim.gsp.ran, sim.sec2.ran), (1, 1));
    assert_eq!(sim.reg(regs::GSP_BASE + F::OS), BL_APP_VERSION, "app version handed to GSP-RM");
    assert_eq!(sim.reg(regs::PCI_ROM_SHADOW) & 1, 1, "ROM shadow restored");
    assert!(r.wpr2_hi != 0 && r.layout.frts.addr == (8u64 << 30) - 0x20_0000);
    eprintln!("{:?}", sim.notes);
}

#[test]
fn fuse_picks_second_fwsec_signature() {
    let (r, _, _, _) = run(1, false, true);
    assert_eq!(r.unwrap().fwsec_sig, 1);
}

#[test]
fn booter_failure_names_the_step() {
    let (r, _, steps, _) = run(0, true, true);
    let e = r.unwrap_err();
    assert_eq!(e.step, Step::Booter);
    assert!(matches!(e.error, Error::FalconFailed { falcon: "sec2", mbox0: 0xbad, .. }));
    assert_eq!(steps.last(), Some(&Step::Riscv));
}

#[test]
fn silent_gsp_times_out() {
    let (r, _, _, t) = run(0, false, false);
    let e = r.unwrap_err();
    assert_eq!((e.step, e.error), (Step::InitDone, Error::Gsp("timed out waiting for GSP_INIT_DONE")));
    assert!(t.0 >= 1000 * 10);
}

#[test]
fn refuses_when_wpr2_is_already_set() {
    // A previous driver left GSP-RM running: Booter-unload must come first.
    let (bl, bt, gsp) = (bootloader(), booter(), gsp_elf(4));
    let f = Firmware { booter_load: &bt, bootloader: &bl, gsp: &gsp };
    let mut sim = Sim { regs: HashMap::new(), rom: vbios(), arena: std::ptr::null_mut(), arena_len: 0, gsp: Falcon::new(), sec2: Falcon::new(), rm_image: vec![], doorbells: 0, fwsec_fuse: 0, fail_booter: false, send_init_done: true, notes: vec![] };
    sim.regs.insert(regs::PFB_WPR2_ADDR_HI, 0x7fff0);
    let mut scratch = vec![0u8; 4096];
    let mut z: Vec<Vec<u8>> = (0..15).map(|_| vec![]).collect();
    let mut it = z.iter_mut();
    let mut d = || Dma::new(it.next().unwrap(), 0);
    let mut mem = Memory { fwsec: d(), booter: d(), rm_image: d(), radix3_l0: d(), radix3_l1: d(), radix3_l2: d(), signature: d(), bootloader: d(), wpr_meta: d(), libos: d(), loginit: d(), logintr: d(), logrm: d(), rmargs: d(), shm: d(), scratch: &mut scratch };
    let cfg = Config { system: Default::default(), init_polls: 1 };
    let e = boot::boot(&mut sim, &mut Ticks(0), &cfg, &f, &mut [0u8; 16], &mut mem, &mut |_: Step, _: &boot::Report| {}).unwrap_err();
    assert_eq!((e.step, e.error), (Step::Chip, Error::Wpr2Busy(0x7fff0)));
}
