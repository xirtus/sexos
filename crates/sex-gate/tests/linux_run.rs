// SPDX-License-Identifier: GPL-3.0-only
//! Runs real gate stubs on the Linux host under real protection keys (`pkey_alloc`).
//! The host plays the kernel: it lays out the frame and slot pages, tags them with keys,
//! points the GS base at the frame page and maps the stub next to the callee.
//! Skipped (passes) when the host CPU or kernel has no PKU.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use core::arch::asm;
use core::arch::x86_64::__cpuid_count;
use sex_gate::*;

const ARCH_SET_GS: libc::c_int = 0x1001;

fn host_pku() -> bool {
    let r = unsafe { __cpuid_count(7, 0) };
    r.ecx & (1 << 3) != 0 && r.ecx & (1 << 4) != 0
}

fn rdpkru() -> u32 {
    let v: u32;
    unsafe { asm!("rdpkru", in("ecx") 0, out("eax") v, out("edx") _, options(nomem, nostack)) };
    v
}

fn wrpkru(v: u32) {
    unsafe { asm!("wrpkru", in("eax") v, in("ecx") 0, in("edx") 0, options(nostack)) };
}

fn pkey_alloc() -> u8 {
    let k = unsafe { libc::syscall(libc::SYS_pkey_alloc, 0, 0) };
    assert!(k > 0, "pkey_alloc failed");
    k as u8
}

fn map(len: usize) -> *mut u8 {
    let p = unsafe {
        libc::mmap(core::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_POPULATE, -1, 0)
    };
    assert_ne!(p, libc::MAP_FAILED);
    p as *mut u8
}

fn tag(p: *mut u8, len: usize, key: u8) {
    let r = unsafe { libc::syscall(libc::SYS_pkey_mprotect, p, len, libc::PROT_READ | libc::PROT_WRITE, key as libc::c_int) };
    assert_eq!(r, 0, "pkey_mprotect");
}

/// Maps `len` bytes within ±1 GiB of `near` (for the stub's rel32 into the callee).
fn map_near(near: u64, len: usize) -> *mut u8 {
    for step in 1..64u64 {
        for hint in [near.wrapping_sub(step << 24) & !0xFFF, near.wrapping_add(step << 24) & !0xFFF] {
            let p = unsafe {
                libc::mmap(hint as *mut _, len, libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE, -1, 0)
            };
            if p != libc::MAP_FAILED {
                return p as *mut u8;
            }
        }
    }
    panic!("no free range near the callee");
}

extern "C" fn sum(a: u64, b: u64, c: u64, d: u64) -> u64 {
    a.wrapping_add(b).wrapping_add(c).wrapping_add(d)
}

extern "C" fn callee_pkru(_: u64, _: u64, _: u64, _: u64) -> u64 {
    rdpkru() as u64
}

/// A callee that tries to read the address it is given.
extern "C" fn peek(a: u64, _: u64, _: u64, _: u64) -> u64 {
    unsafe { core::ptr::read_volatile(a as *const u64) }
}

/// A callee that reports which vector state and general registers it was handed.
extern "C" fn leak_regs(_: u64, _: u64, _: u64, _: u64) -> u64 {
    let (rbx, r12, r15, x): (u64, u64, u64, u64);
    unsafe {
        asm!("mov {0}, rbx", "mov {1}, r12", "mov {2}, r15", "movq {3}, xmm7",
            out(reg) rbx, out(reg) r12, out(reg) r15, out(reg) x, options(nomem, nostack));
    }
    rbx | r12 | r15 | x
}

struct World {
    caller_pkru: u32,
    callee_pkru: u32,
    other_pkru: u32,
    frame_key: u8,
    area: *mut u8,
    matrix: *mut u8,
    stubs: *mut u8,
    secret: *mut u64,
}

const CALLER: u32 = 0;
const CALLEE: u32 = 1;
const STUB_LEN: usize = 1024;

fn world(entries: &[extern "C" fn(u64, u64, u64, u64) -> u64]) -> World {
    let (kc, ke, kf) = (pkey_alloc(), pkey_alloc(), pkey_alloc());
    let caller_pkru = pkru(1 << 0 | 1 << kc, 1 << kf);
    let callee_pkru = pkru(1 << ke, 1 << kf);
    let other_pkru = pkru(1 << 0, 1 << kf);

    // Frame page, caller slot page, callee slot page, then 16 pages of callee stack.
    let area = map(3 * 4096);
    let stack = map(16 * 4096);
    let secret = map(4096) as *mut u64;
    unsafe {
        let w = |off: usize, v: u64| (area.add(off) as *mut u64).write(v);
        w(frame::CUR, caller_pkru as u64);
        w(frame::DEPTH, 0);
        w(frame::ACTIVE, 1 << CALLER);
        w(frame::SELF, area as u64);
        w(slot_offset(CALLEE) as usize + slot::STACK, stack as u64 + 16 * 4096);
        secret.write(0x5ec7e7);
    }
    tag(area, 4096, kf);
    tag(unsafe { area.add(slot_offset(CALLER) as usize) }, 4096, kc);
    tag(unsafe { area.add(slot_offset(CALLEE) as usize) }, 4096, ke);
    tag(stack, 16 * 4096, ke);
    tag(secret as *mut u8, 4096, kc);
    let r = unsafe { libc::syscall(libc::SYS_arch_prctl, ARCH_SET_GS, area as u64) };
    assert_eq!(r, 0, "arch_prctl(ARCH_SET_GS)");

    let matrix = map(4096);
    let stubs = map_near(sum as *const () as usize as u64, 4096);
    for (i, &entry) in entries.iter().enumerate() {
        unsafe { matrix.add(i).write(1) };
        let s = Stub {
            at: stubs as u64 + (i * STUB_LEN) as u64,
            matrix: matrix as u64 + i as u64,
            id: 1 + i as u32,
            frame_key: kf,
            caller_pkru,
            callee_pkru,
            caller: CALLER,
            callee: CALLEE,
            entry: entry as *const () as usize as u64,
        };
        let code = build(&s).expect("build");
        unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), stubs.add(i * STUB_LEN), code.len()) };
    }
    unsafe { libc::mprotect(stubs as *mut _, 4096, libc::PROT_READ | libc::PROT_EXEC) };
    World { caller_pkru, callee_pkru, other_pkru, frame_key: kf, area, matrix, stubs, secret }
}

impl World {
    fn stub(&self, i: usize) -> extern "C" fn(u64, u64, u64, u64) -> u64 {
        unsafe { core::mem::transmute(self.stubs.add(i * STUB_LEN)) }
    }
    fn call(&self, i: usize, a: [u64; 4]) -> (u64, u32) {
        let f = self.stub(i);
        let saved = rdpkru();
        wrpkru(self.caller_pkru);
        let r = f(a[0], a[1], a[2], a[3]);
        let after = rdpkru();
        wrpkru(saved);
        (r, after)
    }
    fn frame(&self, off: usize) -> u64 {
        unsafe { (self.area.add(off) as *const u64).read_volatile() }
    }
    /// Runs `f` in a forked child; returns the signal that ended it (0 = clean exit).
    fn child(&self, f: impl FnOnce()) -> i32 {
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            f();
            unsafe { libc::_exit(0) };
        }
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        if libc::WIFSIGNALED(status) { libc::WTERMSIG(status) } else { 0 }
    }
    /// In a child: become domain X (frame CUR = X, PKRU = X), then jump to `offset` in stub 0
    /// with EAX = `eax`, ECX = EDX = 0.
    fn jump_as_other(&self, offset: usize, eax: u32) -> i32 {
        let target = self.stubs as u64 + offset as u64;
        let (area, other) = (self.area, self.other_pkru);
        self.child(move || unsafe {
            (area.add(frame::CUR) as *mut u64).write(other as u64);
            wrpkru(other);
            asm!("jmp {t}", t = in(reg) target, in("eax") eax, in("ecx") 0, in("edx") 0, options(noreturn));
        })
    }
}

#[test]
fn gate_runs_on_real_protection_keys() {
    if !host_pku() {
        eprintln!("[gate.linux] SKIP: no PKU on this host");
        return;
    }
    let w = world(&[sum, callee_pkru, peek, leak_regs]);
    let [g1, e, g2, c] = template::WRPKRU;

    // A call: arguments arrive, the result comes back, PKRU and the frame are restored.
    let (r, after) = w.call(0, [1, 20, 300, 4000]);
    assert_eq!(r, 4321);
    assert_eq!(after, w.caller_pkru);
    assert_eq!(w.frame(frame::DEPTH), 0);
    assert_eq!(w.frame(frame::CUR), w.caller_pkru as u64);
    assert_eq!(w.frame(frame::ACTIVE), 1 << CALLER);

    // The callee runs under its own keys only.
    assert_eq!(w.call(1, [0; 4]).0, w.callee_pkru as u64);

    // Callee-saved registers survive, and the callee sees none of the caller's registers.
    let (rbx, r12, r15): (u64, u64, u64);
    let f = w.stub(3);
    let saved = rdpkru();
    wrpkru(w.caller_pkru);
    let leaked: u64;
    unsafe {
        asm!(
            "push rbx", "push r12", "push r15",
            "mov rbx, 0x1111", "mov r12, 0x2222", "mov r15, 0x3333", "movq xmm7, rbx",
            "call r13",
            "mov rsi, rbx", "mov rdi, r12", "mov r8, r15",
            "pop r15", "pop r12", "pop rbx",
            in("r13") f, out("rsi") rbx, out("rdi") r12, out("r8") r15,
            out("rax") leaked, out("rcx") _, out("rdx") _,
            out("r9") _, out("r10") _, out("r11") _, out("xmm7") _,
        );
    }
    wrpkru(saved);
    assert_eq!((rbx, r12, r15), (0x1111, 0x2222, 0x3333), "caller's callee-saved registers restored");
    assert_eq!(leaked, 0, "callee saw caller registers");

    // The matrix bit denies without changing anything.
    unsafe { w.matrix.write(0) };
    assert_eq!(w.call(0, [1, 2, 3, 4]), (DENIED, w.caller_pkru));
    assert_eq!(w.frame(frame::DEPTH), 0);
    unsafe { w.matrix.write(1) };

    // The callee cannot read the caller's memory.
    let secret = w.secret as u64;
    let sig = w.child(|| { w.call(2, [secret, 0, 0, 0]); });
    assert_eq!(sig, libc::SIGSEGV, "callee read caller memory");

    // Another domain jumping to each WRPKRU with its matching immediate gains nothing:
    // every one ends in ud2 (SIGILL) before using any register it chose.
    assert_eq!(w.jump_as_other(g1, gate_pkru(w.frame_key)), libc::SIGILL, "jump to G write 1");
    assert_eq!(w.jump_as_other(e, w.callee_pkru), libc::SIGILL, "jump to E write");
    assert_eq!(w.jump_as_other(g2, gate_pkru(w.frame_key)), libc::SIGILL, "jump to G write 2");
    assert_eq!(w.jump_as_other(c, w.caller_pkru), libc::SIGILL, "jump to C write");
    // Calling the stub from the start as another domain faults on the caller's slot page.
    assert_eq!(w.jump_as_other(0, 0), libc::SIGSEGV, "call from another domain");

    // Cost of a full round trip on this CPU (TSC ticks), for the study's table.
    let f = w.stub(0);
    let saved = rdpkru();
    wrpkru(w.caller_pkru);
    let mut best = u64::MAX;
    for _ in 0..15 {
        let t0 = unsafe { core::arch::x86_64::_rdtsc() };
        for i in 0..20_000u64 {
            core::hint::black_box(f(i, 1, 2, 3));
        }
        let t1 = unsafe { core::arch::x86_64::_rdtsc() };
        best = best.min((t1 - t0) / 20_000);
    }
    wrpkru(saved);
    eprintln!("[gate.linux] full gate round trip: {best} TSC ticks (best of 15 x 20,000)");
}
