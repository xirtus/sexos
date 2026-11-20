// SPDX-License-Identifier: GPL-3.0-only
//! SYS_STATS operations for Mesh, answered before cpu_account (which keeps the rest):
//! each core's idle time and what it runs (read through `cpu_account::core_view`), program memory, and who holds what as numbers (a domain's PKU
//! key, how many capabilities it holds, which domains it holds a domain capability to).
//! Never the capabilities themselves. IPC v2 endpoints name objects, not domains, so
//! they are counted but not linked. `None` for every other operation.
use sex_pdx::mesh_stats as wire;

pub fn syscall(regs: &mut crate::interrupts::SyscallRegs) -> Option<u64> {
    let (op, arg, arg2) = (regs.rdi, regs.rsi, regs.rdx);
    if !matches!(op, wire::OP_CORE | wire::OP_MEMORY | wire::OP_ORGAN | wire::OP_LINK) { return None; }
    regs.rsi = 0; regs.rdx = 0; regs.r10 = 0; regs.r8 = 0;
    Some(match op {
        wire::OP_CORE => {
            let t = unsafe { core::arch::x86_64::_rdtsc() };
            let Some((idle, running)) = crate::cpu_account::core_view(arg as usize, t) else { return Some(wire::INVALID) };
            regs.rsi = idle;
            regs.rdx = running as u64;
            regs.r10 = t;
            wire::OK
        }
        wire::OP_MEMORY => {
            regs.rsi = crate::program_frames::lent();
            regs.rdx = crate::program_frames::peak();
            regs.r10 = crate::program_frames::base();
            wire::OK
        }
        wire::OP_ORGAN | wire::OP_LINK => {
            let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(arg as u32).filter(|_| arg != 0 && arg < crate::ipc::MAX_DOMAINS as u64) else { return Some(wire::INVALID) };
            let table = unsafe { &*pd.cap_table };
            let held = (1..=1024).filter_map(|id| table.find(id));
            if op == wire::OP_ORGAN {
                regs.rsi = pd.pku_key as u64;
                regs.rdx = held.count() as u64;
                return Some(wire::OK);
            }
            let next = held.filter_map(|c| match c.data {
                crate::capability::CapabilityData::Domain(d) => Some(d as u64),
                _ => None,
            }).filter(|&d| d > arg2).min();
            match next { Some(d) => { regs.rsi = d; wire::OK } None => wire::NONE }
        }
        _ => return None,
    })
}
