// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! LibOS boot arguments: the memory regions GSP-RM finds at start (three log
//! buffers and the RM arguments), and `GSP_ARGUMENTS_CACHED`.

use crate::{wr32, wr64, Dma};

pub const ARG_SIZE: usize = 32;
pub const KIND_CONTIGUOUS: u8 = 1;
pub const LOC_SYSMEM: u8 = 1;
pub const LOG_BUFFER_SIZE: usize = 0x1_0000;
pub const RMARGS_SIZE: usize = 0x1000;

/// LibOS region tag: up to 8 ASCII bytes, big-endian packed.
pub fn id8(name: &str) -> u64 { name.bytes().take(8).fold(0u64, |id, b| (id << 8) | b as u64) }

/// Write one `LibosMemoryRegionInitArgument` at index `i`.
pub fn write_region(args: &mut [u8], i: usize, name: &str, pa: u64, size: u64) {
    let o = i * ARG_SIZE;
    args[o..o + ARG_SIZE].fill(0);
    wr64(args, o, id8(name));
    wr64(args, o + 8, pa);
    wr64(args, o + 16, size);
    args[o + 24] = KIND_CONTIGUOUS;
    args[o + 25] = LOC_SYSMEM;
}

/// A log buffer: word 0 is GSP-RM's put pointer, then the bus address of
/// each of its own pages (GSP-RM wants the "PTE array" even when contiguous).
pub fn init_log_buffer(buf: &mut Dma) {
    buf.cpu.fill(0);
    let pages = buf.len().div_ceil(4096);
    for i in 0..pages { wr64(buf.cpu, 8 + i * 8, buf.bus + (i * 4096) as u64); }
}

/// `GSP_ARGUMENTS_CACHED` for a cold boot.
pub fn write_rmargs(out: &mut [u8], shm_bus: u64, pte_count: u32, cmdq_off: u64, msgq_off: u64) {
    out.fill(0);
    wr64(out, 0, shm_bus);
    wr32(out, 8, pte_count);
    wr64(out, 16, cmdq_off);
    wr64(out, 24, msgq_off);
    // lockless queues, srInitArguments (oldLevel 0, flags 0, not in PM
    // transition), gpuInstance 0, profiler args: all zero.
}

/// Decode a LibOS log buffer's put pointer: how many u64 words of log are valid.
pub fn log_words(buf: &[u8]) -> u64 { crate::rd64(buf, 0).unwrap_or(0) }
