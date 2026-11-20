// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Talking to GSP-RM once it runs (stage N4c): one RPC at a time over the queues the
//! boot set up, the static configuration GSP-RM gathered while starting, and RM
//! objects (client, device, subdevice) and controls on them.
//!
//! Display (N5) and acceleration (N6) are RM objects too; this is the layer they use.
//! Layouts are NVIDIA's 535.113.01 structures as nouveau's `nvrm` excerpts give them
//! (`rpc_gsp_rm_alloc_v03_00`, `rpc_gsp_rm_control_v03_00`, `rpc_free_v03_00`,
//! `GspStaticConfigInfo`); `tests/structs.rs` pins the offsets checked against them with a
//! C compiler.

use crate::boot::Memory;
use crate::falcon::GSP;
use crate::msgq::{self, Queues};
use crate::{regs, rd32, rd64, rpc, sequencer, wr32, Clock, Error, Mmio};

pub const FN_FREE: u32 = 10;
pub const FN_GET_GSP_STATIC_INFO: u32 = 65;
pub const FN_GSP_RM_CONTROL: u32 = 76;
pub const FN_GSP_RM_ALLOC: u32 = 103;

/// RM classes.
pub const NV01_ROOT: u32 = 0x0;
pub const NV01_DEVICE_0: u32 = 0x80;
pub const NV20_SUBDEVICE_0: u32 = 0x2080;

/// Handles, as nouveau picks them (`nvkm/subdev/gsp/rm/handles.h`).
pub const CLIENT: u32 = 0xc1d0_0001;
pub const DEVICE: u32 = 0xde1d_0000;
pub const SUBDEVICE: u32 = 0x5d1d_0000;

pub const ALLOC_HDR: usize = 32;
pub const CONTROL_HDR: usize = 24;
pub const STATIC_INFO_SIZE: usize = 2168;

/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO`, on GSP-RM's internal subdevice.
pub const CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO: u32 = 0x2080_0a01;
pub const DISPLAY_STATIC_INFO_SIZE: usize = 28;

/// Messages are awaited this many times 10 us (nouveau allows seconds per RPC).
pub const DEFAULT_POLLS: u32 = 200_000;

/// The driver's half of the RPC channel after `GSP_INIT_DONE`.
pub struct Session<'m, 'a> {
    mem: &'m mut Memory<'a>,
    q: Queues,
    resume: sequencer::Resume,
    saved: [u32; 8],
    pub polls: u32,
    /// Messages that were not the awaited reply (logs, notices, sequencer runs).
    pub other_events: u32,
}

impl<'m, 'a> Session<'m, 'a> {
    /// `cmdq_seq` and `app_version` come from the boot's [`crate::boot::Report`].
    pub fn new(mem: &'m mut Memory<'a>, cmdq_seq: u32, app_version: u32) -> Self {
        let resume = sequencer::Resume { libos_bus: mem.libos.bus, app_version };
        let mut q = Queues::new(msgq::shared_layout());
        q.seq = cmdq_seq;
        Session { mem, q, resume, saved: [0; 8], polls: DEFAULT_POLLS, other_events: 0 }
    }

    /// Send `function` with `payload` and wait for GSP-RM's answer to it; returns the
    /// answer's payload. Sequencer scripts are run meanwhile, other messages skipped.
    pub fn call(&mut self, m: &mut impl Mmio, c: &mut impl Clock, function: u32, payload: &[u8]) -> Result<&[u8], Error> {
        let half = self.mem.scratch.len() / 2;
        {
            let (tx, _) = self.mem.scratch.split_at_mut(half);
            self.q.push(self.mem.shm.cpu, tx, function, payload)?;
        }
        GSP.wr(m, regs::falcon::QUEUE_HEAD0, 0);
        for _ in 0..self.polls.max(1) {
            let rx = &mut self.mem.scratch[half..];
            match self.q.recv(self.mem.shm.cpu, rx)? {
                Some(msg) if msg.function == function => {
                    if msg.result != 0 { return Err(Error::Rpc { function, status: msg.result }); }
                    return Ok(&self.mem.scratch[half..half + msg.len]);
                }
                Some(msg) => {
                    self.other_events += 1;
                    if msg.function == rpc::EV_GSP_RUN_CPU_SEQUENCER {
                        sequencer::run(m, c, &self.mem.scratch[half..half + msg.len], &self.resume, &mut self.saved)?;
                    }
                }
                None => c.delay_us(10),
            }
        }
        Err(Error::Gsp("timed out waiting for an RPC reply"))
    }

    /// What GSP-RM found while starting: the GPU's name, memory, units, handles.
    pub fn static_info(&mut self, m: &mut impl Mmio, c: &mut impl Clock) -> Result<StaticInfo, Error> {
        let req = [0u8; STATIC_INFO_SIZE];
        let r = self.call(m, c, FN_GET_GSP_STATIC_INFO, &req)?;
        StaticInfo::parse(r)
    }

    /// Allocate an RM object; `params` is the class's allocation structure.
    pub fn alloc(&mut self, m: &mut impl Mmio, c: &mut impl Clock, client: u32, parent: u32, object: u32, class: u32, params: &[u8]) -> Result<(), Error> {
        let mut req = [0u8; ALLOC_HDR + 128];
        let n = ALLOC_HDR + params.len();
        if n > req.len() { return Err(Error::BufferTooSmall { need: n, have: req.len() }); }
        alloc_request(&mut req[..n], client, parent, object, class, params);
        let r = self.call(m, c, FN_GSP_RM_ALLOC, &req[..n])?;
        match rd32(r, 16) {
            Some(0) => Ok(()),
            Some(status) => Err(Error::Rm { what: class, status }),
            None => Err(Error::Gsp("RM_ALLOC reply too short")),
        }
    }

    /// Run an RM control on `object`; returns the control's parameters as GSP-RM
    /// filled them in.
    pub fn control(&mut self, m: &mut impl Mmio, c: &mut impl Clock, client: u32, object: u32, cmd: u32, params: &[u8]) -> Result<&[u8], Error> {
        let mut req = [0u8; CONTROL_HDR + 512];
        let n = CONTROL_HDR + params.len();
        if n > req.len() { return Err(Error::BufferTooSmall { need: n, have: req.len() }); }
        control_request(&mut req[..n], client, object, cmd, params);
        let want = params.len();
        let r = self.call(m, c, FN_GSP_RM_CONTROL, &req[..n])?;
        match rd32(r, 12) {
            Some(0) if r.len() >= CONTROL_HDR + want => Ok(&r[CONTROL_HDR..CONTROL_HDR + want]),
            Some(0) => Err(Error::Gsp("RM_CONTROL reply too short")),
            Some(status) => Err(Error::Rm { what: cmd, status }),
            None => Err(Error::Gsp("RM_CONTROL reply too short")),
        }
    }

    /// Free an RM object (and everything under it).
    pub fn free(&mut self, m: &mut impl Mmio, c: &mut impl Clock, client: u32, object: u32) -> Result<(), Error> {
        let mut req = [0u8; 16];
        wr32(&mut req, 0, client);
        wr32(&mut req, 8, object);
        self.call(m, c, FN_FREE, &req).map(|_| ())
    }

    /// Our own client, its device and subdevice: what display and channels hang off.
    pub fn open_device(&mut self, m: &mut impl Mmio, c: &mut impl Clock) -> Result<(), Error> {
        let mut root = [0u8; 108];
        wr32(&mut root, 0, CLIENT);
        wr32(&mut root, 4, 0xffff_ffff); // processID: the kernel's
        self.alloc(m, c, CLIENT, 0, CLIENT, NV01_ROOT, &root)?;
        let mut device = [0u8; 56];
        wr32(&mut device, 4, CLIENT); // hClientShare
        self.alloc(m, c, CLIENT, CLIENT, DEVICE, NV01_DEVICE_0, &device)?;
        self.alloc(m, c, CLIENT, DEVICE, SUBDEVICE, NV20_SUBDEVICE_0, &[0u8; 4])
    }

    /// The display engine as GSP-RM sees it (heads, windows), from its internal subdevice.
    pub fn display_static_info(&mut self, m: &mut impl Mmio, c: &mut impl Clock, s: &StaticInfo) -> Result<DisplayInfo, Error> {
        let r = self.control(m, c, s.internal_client, s.internal_subdevice, CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO, &[0u8; DISPLAY_STATIC_INFO_SIZE])?;
        Ok(DisplayInfo {
            hw_sys_cap: rd32(r, 0).unwrap_or(0),
            window_mask: rd32(r, 4).unwrap_or(0),
            heads: rd32(r, 12).unwrap_or(0),
            primary_vga: r.get(16).copied().unwrap_or(0) != 0,
            i2c_port: rd32(r, 20).unwrap_or(0),
            internal_active_mask: rd32(r, 24).unwrap_or(0),
        })
    }
}

/// `rpc_gsp_rm_alloc_v03_00` + params.
pub fn alloc_request(out: &mut [u8], client: u32, parent: u32, object: u32, class: u32, params: &[u8]) {
    out[..ALLOC_HDR].fill(0);
    wr32(out, 0, client);
    wr32(out, 4, parent);
    wr32(out, 8, object);
    wr32(out, 12, class);
    wr32(out, 20, params.len() as u32);
    out[ALLOC_HDR..ALLOC_HDR + params.len()].copy_from_slice(params);
}

/// `rpc_gsp_rm_control_v03_00` + params.
pub fn control_request(out: &mut [u8], client: u32, object: u32, cmd: u32, params: &[u8]) {
    out[..CONTROL_HDR].fill(0);
    wr32(out, 0, client);
    wr32(out, 4, object);
    wr32(out, 8, cmd);
    wr32(out, 16, params.len() as u32);
    out[CONTROL_HDR..CONTROL_HDR + params.len()].copy_from_slice(params);
}

/// The parts of `GspStaticConfigInfo` the driver uses so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticInfo {
    pub name: [u8; 64],
    pub short_name: [u8; 64],
    pub fb_length: u64,
    pub fb_bus_width: u32,
    pub fb_ram_type: u32,
    pub l2_cache_size: u32,
    pub gpcs: u32,
    pub tpcs: u32,
    pub mobile: bool,
    pub vbios_valid: bool,
    pub vbios_sub_vendor: u32,
    pub vbios_sub_device: u32,
    pub bar1_pde_base: u64,
    pub bar2_pde_base: u64,
    /// Video memory the driver may use (regions neither reserved nor protected).
    pub fb_usable: u64,
    pub fb_regions: u32,
    pub internal_client: u32,
    pub internal_device: u32,
    pub internal_subdevice: u32,
}

impl StaticInfo {
    pub fn parse(b: &[u8]) -> Result<StaticInfo, Error> {
        if b.len() < STATIC_INFO_SIZE { return Err(Error::Gsp("GET_GSP_STATIC_INFO reply too short")); }
        let mut name = [0u8; 64];
        name.copy_from_slice(&b[1820..1884]);
        let mut short_name = [0u8; 64];
        short_name.copy_from_slice(&b[1884..1948]);
        let gpc_mask = rd32(b, 292).unwrap_or(0);
        let (mut gpcs, mut tpcs) = (0, 0);
        for g in 0..32 {
            if gpc_mask & (1 << g) != 0 {
                gpcs += 1;
                tpcs += rd32(b, 296 + g * 8 + 4).unwrap_or(0).count_ones();
            }
        }
        // NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_PARAMS: count, then 16 regions of 48 bytes.
        let regions = rd32(b, 840).unwrap_or(0).min(16);
        let mut usable = 0u64;
        for i in 0..regions as usize {
            let r = 848 + i * 48;
            let (base, limit, reserved) = (rd64(b, r).unwrap_or(0), rd64(b, r + 8).unwrap_or(0), rd64(b, r + 16).unwrap_or(0));
            if reserved == 0 && b[r + 30] == 0 && limit >= base { usable += limit + 1 - base; }
        }
        Ok(StaticInfo {
            name, short_name,
            fb_length: rd64(b, 1752).unwrap_or(0),
            fb_bus_width: rd32(b, 1764).unwrap_or(0),
            fb_ram_type: rd32(b, 1768).unwrap_or(0),
            l2_cache_size: rd32(b, 1776).unwrap_or(0),
            gpcs, tpcs,
            mobile: b[2084] != 0,
            vbios_valid: b[2104] != 0,
            vbios_sub_vendor: rd32(b, 2108).unwrap_or(0),
            vbios_sub_device: rd32(b, 2112).unwrap_or(0),
            bar1_pde_base: rd64(b, 2088).unwrap_or(0),
            bar2_pde_base: rd64(b, 2096).unwrap_or(0),
            fb_usable: usable,
            fb_regions: regions,
            internal_client: rd32(b, 2152).unwrap_or(0),
            internal_device: rd32(b, 2156).unwrap_or(0),
            internal_subdevice: rd32(b, 2160).unwrap_or(0),
        })
    }

    /// The marketing name ("NVIDIA GeForce RTX 3070 Laptop GPU"), up to its NUL.
    pub fn name_str(&self) -> &str { c_str(&self.name) }
    pub fn short_name_str(&self) -> &str { c_str(&self.short_name) }
}

fn c_str(b: &[u8]) -> &str {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..end]).unwrap_or("?")
}

/// `NV2080_CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO_PARAMS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DisplayInfo {
    pub hw_sys_cap: u32,
    pub window_mask: u32,
    pub heads: u32,
    pub primary_vga: bool,
    pub i2c_port: u32,
    pub internal_active_mask: u32,
}
