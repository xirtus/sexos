// SPDX-License-Identifier: GPL-3.0-only
//! AVX test: a program's YMM registers are its own. `run avxtest`. It fills YMM0-14
//! with 256-bit values only this run uses, then yields and spins so the core switches
//! to other tasks (other copies of this test among them). Each yield also adds YMM0 into
//! YMM15 with an AVX2 integer add, so YMM15 must end at yields x YMM0 per lane. Every
//! register is checked after each round; a run makes at least 1,000 yields. Its gate
//! runs two copies on one core.
#![no_std]
#![no_main]

use core::fmt::Write as _;
use sex_app::{Event, Window};

const ROUNDS: u32 = 40;
const YIELDS: u64 = 25;
/// Keep going this long (ms ticks) so copies started one after another overlap.
const MIN_TICKS: u64 = 15_000;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[avxtest.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// What `hold` loads and reads back, at fixed offsets for its assembly.
#[repr(C)]
struct Frame { regs: [[u64; 4]; 16], out: [[u64; 4]; 16], spin: u64 }
const _: () = assert!(core::mem::offset_of!(Frame, out) == 512 && core::mem::offset_of!(Frame, spin) == 1024);

/// True when the CPU has AVX2 and the kernel enabled YMM state (OSXSAVE, XCR0 bits 1-2).
fn avx2_usable() -> bool {
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    let l1 = unsafe { __cpuid(1) }.ecx;
    if l1 & (1 << 27) == 0 || l1 & (1 << 28) == 0 { return false; }
    let (lo, _hi): (u32, u32);
    unsafe { core::arch::asm!("xgetbv", in("ecx") 0, out("eax") lo, out("edx") _hi, options(nomem, nostack)); }
    lo & 0b110 == 0b110 && unsafe { __cpuid_count(7, 0) }.ebx & (1 << 5) != 0
}

/// Loads YMM0-14, zeroes YMM15, then `YIELDS` times: yield, YMM15 += YMM0 (AVX2), spin.
/// Reads all sixteen back.
#[target_feature(enable = "avx2")]
unsafe fn hold(f: &mut Frame) {
    core::arch::asm!(
        "vmovdqu ymm0, [r14]", "vmovdqu ymm1, [r14 + 32]", "vmovdqu ymm2, [r14 + 64]", "vmovdqu ymm3, [r14 + 96]",
        "vmovdqu ymm4, [r14 + 128]", "vmovdqu ymm5, [r14 + 160]", "vmovdqu ymm6, [r14 + 192]", "vmovdqu ymm7, [r14 + 224]",
        "vmovdqu ymm8, [r14 + 256]", "vmovdqu ymm9, [r14 + 288]", "vmovdqu ymm10, [r14 + 320]", "vmovdqu ymm11, [r14 + 352]",
        "vmovdqu ymm12, [r14 + 384]", "vmovdqu ymm13, [r14 + 416]", "vmovdqu ymm14, [r14 + 448]",
        "vpxor ymm15, ymm15, ymm15",
        "mov r12, {yields}",
        "3:",
        "mov eax, 32", // yield
        "syscall",
        "vpaddq ymm15, ymm15, ymm0",
        "mov r13, [r14 + 1024]",
        "4:",
        "dec r13",
        "jnz 4b",
        "dec r12",
        "jnz 3b",
        "vmovdqu [r14 + 512], ymm0", "vmovdqu [r14 + 544], ymm1", "vmovdqu [r14 + 576], ymm2", "vmovdqu [r14 + 608], ymm3",
        "vmovdqu [r14 + 640], ymm4", "vmovdqu [r14 + 672], ymm5", "vmovdqu [r14 + 704], ymm6", "vmovdqu [r14 + 736], ymm7",
        "vmovdqu [r14 + 768], ymm8", "vmovdqu [r14 + 800], ymm9", "vmovdqu [r14 + 832], ymm10", "vmovdqu [r14 + 864], ymm11",
        "vmovdqu [r14 + 896], ymm12", "vmovdqu [r14 + 928], ymm13", "vmovdqu [r14 + 960], ymm14", "vmovdqu [r14 + 992], ymm15",
        "vzeroupper",
        yields = const YIELDS,
        in("r14") f as *mut Frame,
        out("rax") _, out("rcx") _, out("rdx") _, out("rsi") _, out("rdi") _,
        out("r8") _, out("r9") _, out("r10") _, out("r11") _, out("r12") _, out("r13") _,
        out("ymm0") _, out("ymm1") _, out("ymm2") _, out("ymm3") _, out("ymm4") _, out("ymm5") _, out("ymm6") _, out("ymm7") _,
        out("ymm8") _, out("ymm9") _, out("ymm10") _, out("ymm11") _, out("ymm12") _, out("ymm13") _, out("ymm14") _, out("ymm15") _,
    );
}

fn run(task: u32) -> Result<(u32, u64), (u32, &'static str)> {
    let mut state = (task as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; state };
    let start = sex_pdx::get_ticks();
    let (mut round, mut yields) = (0u32, 0u64);
    while round < ROUNDS || sex_pdx::get_ticks() - start < MIN_TICKS {
        let mut f = Frame { regs: core::array::from_fn(|_| [next(), next(), next(), next()]), out: [[0; 4]; 16], spin: 2_000 + (next() & 0xFFF) };
        f.regs[15] = [0; 4];
        let want_sum = f.regs[0].map(|lane| lane.wrapping_mul(YIELDS));
        unsafe { hold(&mut f) };
        for r in 0..15 {
            if f.out[r][..2] != f.regs[r][..2] { return Err((round, "XMM (low half) registers changed")); }
            if f.out[r] != f.regs[r] { return Err((round, "YMM upper halves changed")); }
        }
        if f.out[15] != want_sum { return Err((round, "AVX2 sum in YMM15 is wrong")); }
        round += 1;
        yields += YIELDS;
    }
    Ok((round, yields))
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"AVX test");
    let mut text = Text { buf: [0; 256], len: 0 };
    if !avx2_usable() {
        let _ = write!(text, "AVX test skipped: this CPU or kernel offers no AVX2.\nF11 closes.");
        sex_pdx::serial_record!("[avxtest.skip] target={} task={} why=no-avx2", window.target, window.task);
    } else {
        match run(window.task) {
            Ok((rounds, yields)) => {
                let _ = write!(text, "AVX test passed.\n{} rounds, {} yields.\nYMM0-15 kept.\nF11 closes.", rounds, yields);
                sex_pdx::serial_record!("[avxtest.ok] target={} task={} rounds={} yields={}", window.target, window.task, rounds, yields);
            }
            Err((round, why)) => {
                let _ = write!(text, "AVX test FAILED in round {}: {}.", round, why);
                sex_pdx::serial_record!("[avxtest.fail] target={} task={} round={} why={}", window.target, window.task, round, why);
            }
        }
    }
    window.show(text.as_bytes());
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[avxtest.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}

/// A small fixed text buffer.
struct Text { buf: [u8; 256], len: usize }
impl Text { fn as_bytes(&self) -> &[u8] { &self.buf[..self.len] } }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}
