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
use sex_gpu_nv::{chip::Chip, msgq, rm, rpc, Clock, Dma, Error, Mmio};
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
    /// After Booter: where the queues are, our next message slot, GSP's command read index.
    shm: u64,
    msg_slot: u32,
    cmd_read: u32,
    /// RM objects GSP-RM holds (handle, class) and every RPC function it answered.
    objects: Vec<(u32, u32)>,
    calls: Vec<u32>,
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
        self.shm = shm;
        self.cmd_read = 2;
        self.gsp_send(0x100c, b"log line");
        self.gsp_send(rpc::EV_GSP_RUN_CPU_SEQUENCER, &seq);
        if self.send_init_done { self.gsp_send(rpc::EV_GSP_INIT_DONE, &[0u8; 8]); }
    }

    /// GSP-RM after INIT_DONE: answer each command the driver queued.
    fn serve_rm(&mut self) {
        let l = msgq::shared_layout();
        let shm = self.shm;
        let wptr = r32(self.bus(shm + l.cmdq_off as u64 + 16, 4), 0) % msgq::ENTRIES;
        while self.cmd_read != wptr {
            let e = self.bus(shm + (l.cmdq_off + 0x1000 + self.cmd_read as usize * 0x1000) as u64, 0x1000).to_vec();
            assert_eq!(msgq::checksum(&e), 0, "command checksum");
            assert_eq!(r32(&e, 40), 1, "one-page commands");
            assert_eq!(r32(&e, 36), self.cmd_read, "command sequence numbers continue the boot's");
            let (function, len) = (r32(&e, 60), r32(&e, 56) as usize - 32);
            let body = e[80..80 + len].to_vec();
            self.cmd_read = (self.cmd_read + 1) % msgq::ENTRIES;
            let at = shm + l.msgq_off as u64 + 32;
            let read = self.cmd_read;
            w32(self.bus_mut(at, 4), 0, read);
            self.calls.push(function);
            self.answer(function, body);
        }
    }

    fn answer(&mut self, function: u32, mut body: Vec<u8>) {
        match function {
            rm::FN_GET_GSP_STATIC_INFO => {
                assert_eq!(body.len(), rm::STATIC_INFO_SIZE);
                // Interleaved notices and a sequencer script before the answer.
                self.gsp_send(rpc::EV_OS_ERROR_LOG, b"note");
                let mut seq = vec![0u8; 40 + 3 * 4];
                w32(&mut seq, 0, 16); w32(&mut seq, 4, 3); // buffer size, dwords written
                for (i, v) in [0u32, 0x4444, 7].iter().enumerate() { w32(&mut seq, 40 + i * 4, *v); }
                self.gsp_send(rpc::EV_GSP_RUN_CPU_SEQUENCER, &seq);
                let b = &mut body;
                let name = b"NVIDIA GeForce RTX 3070 Laptop GPU";
                b[1820..1820 + name.len()].copy_from_slice(name);
                b[1884..1884 + 5].copy_from_slice(b"GA104");
                w32(b, 292, 0b11111); // 5 GPCs
                for g in 0..5 { w32(b, 296 + g * 8 + 4, 0b1111); }
                w32(b, 840, 2); // FB regions: usable 0..7.5 GiB, then a reserved one
                w64(b, 848, 0); w64(b, 856, (7u64 << 30) + (1 << 29) - 1);
                w64(b, 896, 7u64 << 30 | 1 << 29); w64(b, 904, (8u64 << 30) - 1); w64(b, 912, 1);
                w64(b, 1752, 8 << 30); w32(b, 1764, 256); w32(b, 1768, 8); w32(b, 1776, 4 << 20);
                b[2084] = 1; b[2104] = 1; w32(b, 2108, 0x1028); w32(b, 2112, 0x0a4a);
                w64(b, 2088, 0x1_2000_0000);
                w32(b, 2152, 0xc1e0_0004); w32(b, 2156, 0xcaf0_0001); w32(b, 2160, 0xcaf0_0002);
                self.objects.push((0xcaf0_0002, rm::NV20_SUBDEVICE_0));
                self.gsp_send(function, &body);
            }
            rm::FN_GSP_RM_ALLOC => {
                let (client, parent, object, class) = (r32(&body, 0), r32(&body, 4), r32(&body, 8), r32(&body, 12));
                assert_eq!(r32(&body, 20) as usize, body.len() - rm::ALLOC_HDR, "paramsSize");
                let known_parent = class == rm::NV01_ROOT && parent == 0 && object == client || self.objects.iter().any(|o| o.0 == parent);
                let status = match class {
                    rm::NV01_ROOT => { assert_eq!(r32(&body, 32), object); 0 }
                    rm::NV01_DEVICE_0 => { assert_eq!(r32(&body, 36), client, "hClientShare"); 0 }
                    rm::NV20_SUBDEVICE_0 => 0,
                    _ => 0x56, // NV_ERR_INVALID_CLASS
                };
                let status = if known_parent { status } else { 0x1e /* NV_ERR_INVALID_OBJECT_PARENT */ };
                if status == 0 { self.objects.push((object, class)); }
                w32(&mut body, 16, status);
                self.gsp_send(function, &body);
            }
            rm::FN_GSP_RM_CONTROL => {
                let (object, cmd) = (r32(&body, 4), r32(&body, 8));
                let status = if cmd == rm::CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO && object == 0xcaf0_0002 {
                    let p = &mut body[rm::CONTROL_HDR..];
                    w32(p, 4, 0xff); w32(p, 12, 4); p[16] = 1; w32(p, 24, 0b1);
                    0
                } else { 0x22 }; // NV_ERR_NOT_SUPPORTED
                w32(&mut body, 12, status);
                self.gsp_send(function, &body);
            }
            rm::FN_FREE => {
                let object = r32(&body, 8);
                self.objects.retain(|o| o.0 != object);
                self.gsp_send(function, &body);
            }
            other => panic!("unexpected RPC {other}"),
        }
    }

    fn gsp_send(&mut self, function: u32, body: &[u8]) {
        let l = msgq::shared_layout();
        let shm = self.shm;
        let at = shm + (l.msgq_off + 0x1000 + self.msg_slot as usize * 0x1000) as u64;
        let e = self.bus_mut(at, 0x1000);
        e.fill(0);
        w32(e, 40, 1);
        w32(e, 48, 0x0300_0000); e[52..56].copy_from_slice(b"VRPC"); w32(e, 56, 32 + body.len() as u32); w32(e, 60, function);
        e[80..80 + body.len()].copy_from_slice(body);
        self.msg_slot += 1;
        let s = self.msg_slot;
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
            Some((b, F::QUEUE_HEAD0)) if b == regs::GSP_BASE => {
                self.doorbells += 1;
                if self.shm != 0 { self.serve_rm(); }
            }
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

fn run(fwsec_fuse: u32, fail_booter: bool, send_init_done: bool) -> (Result<boot::Report, boot::Failed>, Sim, Vec<Step>, Ticks) {
    run_then(fwsec_fuse, fail_booter, send_init_done, |_, _, _, _| {})
}

/// Boot, then (if it worked) hand the live simulation and memory to `after`.
fn run_then(fwsec_fuse: u32, fail_booter: bool, send_init_done: bool, after: impl FnOnce(&mut Sim, &mut Ticks, &mut Memory, &boot::Report)) -> (Result<boot::Report, boot::Failed>, Sim, Vec<Step>, Ticks) {
    let (bl, bt, gsp) = (bootloader(), booter(), gsp_elf(700)); // 700 pages: radix3 level 2 spans two pages
    let f = Firmware { booter_load: &bt, bootloader: &bl, gsp: &gsp };
    let chip = Chip::decode(0x174000a1).unwrap();
    let sizes = boot::Sizes::for_firmware(&chip, &f).unwrap();
    let mut arena = vec![0u8; sizes.total() + 64 * 4096];
    let mut sim = Sim {
        regs: HashMap::new(), rom: vbios(), arena: arena.as_mut_ptr(), arena_len: arena.len(),
        gsp: Falcon::new(), sec2: Falcon::new(), rm_image: sex_gpu_nv::fw::gsp_rm(&gsp, ".fwsignature_ga10x").unwrap().image.to_vec(),
        doorbells: 0, fwsec_fuse, fail_booter, send_init_done, notes: vec![],
        shm: 0, msg_slot: 0, cmd_read: 0, objects: vec![], calls: vec![],
    };
    sim.regs.insert(regs::PCI_ROM_SHADOW, 1);
    let mut scratch = vec![0u8; 2 * msgq::MAX_MSG];
    let mut mem = Memory::carve(&mut arena, ARENA_BUS, &sizes, &mut scratch).unwrap();
    let cfg = Config { system: rpc::SystemInfo { bar0: 0xa000_0000, bar1: 0x40_0000_0000, bar_inst: 0x42_0000_0000, bus: 1, devfn: 0, max_user_va: 0x7fff_ffff_f000 }, init_polls: 1000 };
    let mut vb = vec![0u8; 1 << 20];
    let mut steps = vec![];
    let mut t = Ticks(0);
    let r = boot::boot(&mut sim, &mut t, &cfg, &f, &mut vb, &mut mem, &mut |s: Step, _: &boot::Report| steps.push(s));
    if let Ok(rep) = &r { after(&mut sim, &mut t, &mut mem, rep); }
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
    let mut sim = Sim { regs: HashMap::new(), rom: vbios(), arena: std::ptr::null_mut(), arena_len: 0, gsp: Falcon::new(), sec2: Falcon::new(), rm_image: vec![], doorbells: 0, fwsec_fuse: 0, fail_booter: false, send_init_done: true, notes: vec![], shm: 0, msg_slot: 0, cmd_read: 0, objects: vec![], calls: vec![] };
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

#[test]
fn talks_to_gsp_rm_after_init_done() {
    let mut seen = None;
    let (r, sim, _, _) = run_then(0, false, true, |sim, t, mem, rep| {
        assert_eq!(rep.cmdq_seq, 2);
        let mut s = rm::Session::new(mem, rep.cmdq_seq, rep.app_version);
        let info = s.static_info(sim, t).unwrap();
        assert_eq!(info.name_str(), "NVIDIA GeForce RTX 3070 Laptop GPU");
        assert_eq!(info.short_name_str(), "GA104");
        assert_eq!((info.fb_length, info.fb_bus_width, info.fb_ram_type, info.l2_cache_size), (8 << 30, 256, 8, 4 << 20));
        assert_eq!((info.gpcs, info.tpcs, info.mobile, info.vbios_valid), (5, 20, true, true));
        assert_eq!((info.fb_regions, info.fb_usable), (2, (7u64 << 30) + (1 << 29)));
        assert_eq!((info.vbios_sub_vendor, info.vbios_sub_device, info.bar1_pde_base), (0x1028, 0x0a4a, 0x1_2000_0000));
        assert_eq!((info.internal_client, info.internal_device, info.internal_subdevice), (0xc1e0_0004, 0xcaf0_0001, 0xcaf0_0002));
        assert_eq!(s.other_events, 2, "a notice and a sequencer run came before the answer");
        s.open_device(sim, t).unwrap();
        let d = s.display_static_info(sim, t, &info).unwrap();
        assert_eq!((d.heads, d.window_mask, d.primary_vga, d.internal_active_mask), (4, 0xff, true, 1));
        // Errors carry RM's status.
        assert_eq!(s.alloc(sim, t, rm::CLIENT, rm::SUBDEVICE, 0x1234, 0xc57d, &[]), Err(Error::Rm { what: 0xc57d, status: 0x56 }));
        assert_eq!(s.control(sim, t, rm::CLIENT, rm::SUBDEVICE, 0x2080_0101, &[0; 8]).map(|_| ()), Err(Error::Rm { what: 0x2080_0101, status: 0x22 }));
        s.free(sim, t, rm::CLIENT, rm::SUBDEVICE).unwrap();
        seen = Some(info);
    });
    r.unwrap();
    assert!(seen.is_some());
    assert_eq!(sim.reg(0x4444), 7, "sequencer script during an RPC ran");
    assert_eq!(sim.calls, [65, 103, 103, 103, 76, 103, 76, 10]);
    let held: Vec<u32> = sim.objects.iter().map(|o| o.0).collect();
    assert_eq!(held, [0xcaf0_0002, rm::CLIENT, rm::DEVICE], "subdevice freed, client and device kept");
}

#[test]
fn request_layouts_match_nvidia() {
    let mut a = [0u8; rm::ALLOC_HDR + 4];
    rm::alloc_request(&mut a, 1, 2, 3, 0x2080, &[9, 0, 0, 0]);
    assert_eq!([r32(&a, 0), r32(&a, 4), r32(&a, 8), r32(&a, 12), r32(&a, 16), r32(&a, 20), r32(&a, 24), a[32] as u32], [1, 2, 3, 0x2080, 0, 4, 0, 9]);
    let mut c = [0u8; rm::CONTROL_HDR + 4];
    rm::control_request(&mut c, 1, 2, 0x2080_0a01, &[7, 0, 0, 0]);
    assert_eq!([r32(&c, 0), r32(&c, 4), r32(&c, 8), r32(&c, 12), r32(&c, 16), r32(&c, 20), c[24] as u32], [1, 2, 0x2080_0a01, 0, 4, 0, 7]);
}
