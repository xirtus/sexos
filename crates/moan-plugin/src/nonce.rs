//! Hardware CSPRNG for fresh job nonces. No timestamp/counter fallback. The nonce
//! is not a bearer capability; launch custody and exact retirement remain required.
pub fn fresh() -> Option<[u8; 16]> {
    let seed = core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 18) != 0;
    let random = core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0;
    if !seed && !random {
        return None;
    }
    let mut out = [0; 16];
    for chunk in out.chunks_mut(8) {
        let mut value = 0u64;
        let mut ok = 0u8;
        for _ in 0..128 {
            unsafe {
                if seed {
                    core::arch::asm!("rdseed {v}","setc {ok}",v=out(reg)value,ok=out(reg_byte)ok,options(nomem,nostack));
                } else {
                    core::arch::asm!("rdrand {v}","setc {ok}",v=out(reg)value,ok=out(reg_byte)ok,options(nomem,nostack));
                }
            }
            if ok != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        if ok == 0 {
            return None;
        }
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    if out == [0; 16] {
        None
    } else {
        Some(out)
    }
}
