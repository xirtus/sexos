//! SMAP in audit mode (kernel pass 2, K7a; audit S11).
//!
//! The kernel still reads and writes user pages directly in many places, so
//! SMAP cannot be enforced yet. Audit mode turns SMAP on and finds those places
//! on a running system. When a kernel data access to a user page faults, the
//! fault handler records the instruction once, as `[smap.site] rip=..`. It then
//! sets RFLAGS.AC in the interrupted frame and retries. AC stays set until that
//! kernel entry returns to ring 3 (SFMASK clears it again on the next syscall),
//! so each entry pays at most one fault. Once every site goes through one
//! user-copy module, the same fault becomes fatal.
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ON: AtomicBool = AtomicBool::new(false);
const SITES: usize = 256;
static RIP: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
/// Fault address, plus bit 0 = write, bit 1 = printed, bit 2 = recorded (the low bits are dropped).
static INFO: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];

/// Turns SMAP on for this core when the CPU has it (after the IDT is loaded).
pub fn enable_core() -> bool {
    use x86_64::registers::control::{Cr4, Cr4Flags};
    if unsafe { core::arch::x86_64::__cpuid_count(7, 0) }.ebx & (1 << 20) == 0 { return false; }
    unsafe { Cr4::update(|f| f.insert(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION)); }
    ON.store(true, Ordering::Release);
    true
}

pub fn mode() -> &'static str { if ON.load(Ordering::Acquire) { "audit" } else { "0" } }

/// A kernel-mode page fault: true when it was an SMAP stop, now recorded, and the
/// access can be retried with AC set. `frame` points at the saved RIP.
pub unsafe fn audit(frame: *mut u64, addr: u64, error: u64) -> bool {
    // Present, supervisor, data access; no reserved-bit or protection-key cause.
    if !ON.load(Ordering::Acquire) || error & 1 == 0 || error & 0x3C != 0 { return false; }
    let rflags = frame.add(2);
    if *rflags & (1 << 18) != 0 || !user_page(addr) { return false; }
    *rflags |= 1 << 18;
    let rip = *frame;
    let mut i = (rip as usize ^ (rip >> 12) as usize) % SITES;
    for _ in 0..SITES {
        match RIP[i].compare_exchange(0, rip, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => { INFO[i].store((addr & !7) | 4 | (error >> 1 & 1), Ordering::Release); break; }
            Err(r) if r == rip => break,
            Err(_) => i = (i + 1) % SITES,
        }
    }
    // Print what is not printed yet; a held serial lock (the fault came from inside
    // a print) leaves it for the next stop.
    for i in 0..SITES {
        let (rip, info) = (RIP[i].load(Ordering::Acquire), INFO[i].load(Ordering::Acquire));
        if rip == 0 || info == 0 || info & 2 != 0 { continue; }
        // `from_audit` lets the host name the function: rip = address of smap::audit + it.
        let from_audit = rip.wrapping_sub(audit as usize as u64) as i64;
        if !crate::serial::try_print(format_args!("[smap.site] rip={:#x} from_audit={} addr={:#x} write={}\n",
            rip, from_audit, info & !7, info & 1)) { break; }
        INFO[i].fetch_or(2, Ordering::AcqRel);
    }
    true
}

/// Whether every level of the walk for `va` allows user access.
unsafe fn user_page(va: u64) -> bool {
    let hhdm = crate::HHDM_REQUEST.response().unwrap().offset;
    let mut table = x86_64::registers::control::Cr3::read().0.start_address().as_u64();
    for shift in [39u64, 30, 21, 12] {
        let e = *((table + hhdm) as *const u64).add((va >> shift & 0x1FF) as usize);
        if e & 0x5 != 0x5 { return false; }
        if shift == 12 || e & (1 << 7) != 0 { return true; }
        table = e & 0x000F_FFFF_FFFF_F000;
    }
    false
}
