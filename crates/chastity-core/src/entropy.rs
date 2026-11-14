// SPDX-License-Identifier: GPL-3.0-only
//! Cryptographic entropy. Host uses the OS; SexOS fails closed when the CPU
//! generator is unavailable or exhausted. It never uses std's timestamp fallback.
pub fn fill(out: &mut [u8]) -> Result<(), getrandom::Error> {
    #[cfg(not(target_os = "hermit"))]
    {
        getrandom::fill(out)
    }
    #[cfg(target_os = "hermit")]
    {
        use zeroize::Zeroize;
        let seed = unsafe { core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 18) != 0 };
        let rand = unsafe { core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0 };
        if !seed && !rand {
            out.zeroize();
            return Err(getrandom::Error::UNSUPPORTED);
        }
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
                value.zeroize();
                out.zeroize();
                return Err(getrandom::Error::UNSUPPORTED);
            }
            let mut bytes = value.to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
            bytes.zeroize();
            value.zeroize();
        }
        Ok(())
    }
}
// The Chastity build stage selects this for all getrandom users in its graph,
// including keepass salts/IVs and UUIDs. No override affects other std programs.
#[cfg(target_os = "hermit")]
#[no_mangle]
pub unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    fill(unsafe { core::slice::from_raw_parts_mut(dest, len) })
}
