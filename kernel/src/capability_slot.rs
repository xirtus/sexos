//! Decode register-width capability selectors before lookup or side effects.

pub const CAPABILITY_SLOT_COUNT: u64 = 1024;

pub fn decode_capability_slot(raw: u64) -> Option<u32> {
    if raw > 0 && raw <= CAPABILITY_SLOT_COUNT {
        Some(raw as u32)
    } else {
        None
    }
}

pub fn decode_pdx_slot(raw: u64) -> Option<u32> {
    if raw == 0 { Some(0) } else { decode_capability_slot(raw) }
}

/// The only operations reachable through slot 0 without a held capability.
/// Each is caller-scoped: BOOT display data is checked against its sanctioned
/// holder, init data is moved out of the calling task, and serial records are
/// bounded diagnostics. Everything else, including the retired framebuffer
/// disclosure (0xE3) and frame-present hook (0xBB), denies before effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeOp {
    BootDisplayQuery,
    SerialRecord,
    InitArg,
}

pub fn decode_bridge_op(number: u64) -> Option<BridgeOp> {
    match number {
        0xE6 => Some(BridgeOp::BootDisplayQuery),
        69 => Some(BridgeOp::SerialRecord),
        0x12 => Some(BridgeOp::InitArg),
        _ => None,
    }
}

pub fn decode_domain_id(raw: u64) -> Option<u32> {
    index_nospec(usize::try_from(raw).ok()?, crate::ipc::MAX_DOMAINS).map(|i| i as u32)
}

/// Spectre v1 (audit S10): `index` if it is below `len`, else None. The index
/// that comes back is ANDed with a mask made by cmp+sbb, which the compiler
/// cannot fold away and which is 0 when a mispredicted bounds branch runs ahead.
#[inline(always)]
pub fn index_nospec(index: usize, len: usize) -> Option<usize> {
    if index >= len { return None; }
    #[cfg(target_arch = "x86_64")]
    let mask: usize = unsafe { let m: usize; core::arch::asm!("cmp {i}, {n}", "sbb {m}, {m}", i = in(reg) index, n = in(reg) len, m = lateout(reg) m, options(pure, nomem, nostack)); m };
    #[cfg(not(target_arch = "x86_64"))]
    let mask = usize::MAX;
    Some(index & mask)
}

/// Validate selectors while they still have their full register width.
/// This is input validation only: callers must still check capability kind,
/// target existence, and operation-specific authority after this preflight.
pub fn valid_syscall_selectors(number: u64, selector: u64, destination: u64) -> bool {
    match number {
        0 | 28 | 100 | 102 => decode_pdx_slot(selector).is_some(),
        29 | 101 | 103 => decode_domain_id(selector).is_some(),
        43 | 51 | 68 | 69 => decode_capability_slot(selector).is_some(),
        50 => decode_capability_slot(selector).is_some()
            && decode_capability_slot(destination).is_some(),
        _ => true,
    }
}
