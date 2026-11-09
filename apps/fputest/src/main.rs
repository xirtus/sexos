// SPDX-License-Identifier: GPL-3.0-only
//! FPU test: a program's x87/SSE registers are its own. `run fputest`. It fills all
//! sixteen XMM registers and MXCSR with values only this run uses, then yields and
//! spins so the core switches to other tasks (other copies of this test among them),
//! and checks every register came back unchanged. It also runs float arithmetic whose
//! exact result is known at compile time. Its gate runs several copies at once.
#![no_std]
#![no_main]

use core::fmt::Write as _;
use sex_app::{Event, Window};

const ROUNDS: u32 = 40;
/// Keep going this long (ms ticks) so copies started one after another overlap.
const MIN_TICKS: u64 = 15_000;
const YIELDS: u64 = 8;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[fputest.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// What `hold` loads and reads back, at fixed offsets for its assembly.
#[repr(C)]
struct Frame { regs: [[u64; 2]; 16], out: [[u64; 2]; 16], mx_in: u32, mx_out: u32, mx_default: u32, _pad: u32, spin: u64 }

/// Loads `regs` and `mxcsr`, yields and spins `YIELDS` times, then reads them back.
fn hold(regs: &[[u64; 2]; 16], mxcsr: u32, spin: u64) -> ([[u64; 2]; 16], u32) {
    let mut f = Frame { regs: *regs, out: [[0; 2]; 16], mx_in: mxcsr, mx_out: 0, mx_default: 0x1F80, _pad: 0, spin };
    unsafe {
        core::arch::asm!(
            "ldmxcsr [r14 + 512]",
            "movdqu xmm0, [r14]", "movdqu xmm1, [r14 + 16]", "movdqu xmm2, [r14 + 32]", "movdqu xmm3, [r14 + 48]",
            "movdqu xmm4, [r14 + 64]", "movdqu xmm5, [r14 + 80]", "movdqu xmm6, [r14 + 96]", "movdqu xmm7, [r14 + 112]",
            "movdqu xmm8, [r14 + 128]", "movdqu xmm9, [r14 + 144]", "movdqu xmm10, [r14 + 160]", "movdqu xmm11, [r14 + 176]",
            "movdqu xmm12, [r14 + 192]", "movdqu xmm13, [r14 + 208]", "movdqu xmm14, [r14 + 224]", "movdqu xmm15, [r14 + 240]",
            "mov r12, {yields}",
            "3:",
            "mov eax, 32", // yield
            "syscall",
            "mov r13, [r14 + 528]",
            "4:",
            "dec r13",
            "jnz 4b",
            "dec r12",
            "jnz 3b",
            "movdqu [r14 + 256], xmm0", "movdqu [r14 + 272], xmm1", "movdqu [r14 + 288], xmm2", "movdqu [r14 + 304], xmm3",
            "movdqu [r14 + 320], xmm4", "movdqu [r14 + 336], xmm5", "movdqu [r14 + 352], xmm6", "movdqu [r14 + 368], xmm7",
            "movdqu [r14 + 384], xmm8", "movdqu [r14 + 400], xmm9", "movdqu [r14 + 416], xmm10", "movdqu [r14 + 432], xmm11",
            "movdqu [r14 + 448], xmm12", "movdqu [r14 + 464], xmm13", "movdqu [r14 + 480], xmm14", "movdqu [r14 + 496], xmm15",
            "stmxcsr [r14 + 516]",
            "ldmxcsr [r14 + 520]",
            yields = const YIELDS,
            in("r14") &mut f as *mut Frame,
            out("rax") _, out("rcx") _, out("rdx") _, out("rsi") _, out("rdi") _,
            out("r8") _, out("r9") _, out("r10") _, out("r11") _, out("r12") _, out("r13") _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _, out("xmm4") _, out("xmm5") _, out("xmm6") _, out("xmm7") _,
            out("xmm8") _, out("xmm9") _, out("xmm10") _, out("xmm11") _, out("xmm12") _, out("xmm13") _, out("xmm14") _, out("xmm15") _,
        );
    }
    (f.out, f.mx_out)
}
const _: () = assert!(core::mem::offset_of!(Frame, out) == 256 && core::mem::offset_of!(Frame, mx_in) == 512
    && core::mem::offset_of!(Frame, mx_out) == 516 && core::mem::offset_of!(Frame, mx_default) == 520 && core::mem::offset_of!(Frame, spin) == 528);

/// Float arithmetic with an exact, known answer: the same loop run by the compiler.
const fn series(n: u32) -> f64 {
    let (mut x, mut sum, mut i) = (1.0f64, 0.0f64, 0);
    while i < n {
        x = x * 1.000_000_7 + 0.25 / (i as f64 + 1.0);
        sum += x / 3.0;
        i += 1;
    }
    sum
}
const SERIES: f64 = series(20_000);

fn run(task: u32) -> Result<u32, (u32, &'static str)> {
    let mut state = (task as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; state };
    // A run starts with the reset MXCSR, whatever the row's last run left.
    let mut initial = 0u32;
    unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut initial); }
    if initial & 0xFFC0 != 0x1F80 { return Err((0, "a run started with another run's MXCSR")); }
    let start = sex_pdx::get_ticks();
    let mut round = 0;
    while round < ROUNDS || sex_pdx::get_ticks() - start < MIN_TICKS {
        let regs: [[u64; 2]; 16] = core::array::from_fn(|_| [next(), next()]);
        // A rounding mode and masks only some runs use, so leaking MXCSR shows.
        let mxcsr = 0x1F80 | (((task + round) & 3) << 13);
        let (got, got_mx) = hold(&regs, mxcsr, 20_000 + (next() & 0x3FFF));
        if got != regs { return Err((round, "XMM registers changed")); }
        if got_mx & 0xFFC0 != mxcsr & 0xFFC0 { return Err((round, "MXCSR changed")); }
        if core::hint::black_box(series)(core::hint::black_box(20_000)) != SERIES { return Err((round, "float arithmetic differs")); }
        round += 1;
    }
    Ok(round)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"FPU test");
    let mut text = heapless_text();
    match run(window.task) {
        Ok(rounds) => {
            let _ = write!(text, "FPU test passed.\n{} rounds, each across {} yields.\nXMM0-15 and MXCSR kept.\nF11 closes.", rounds, YIELDS);
            sex_pdx::serial_record!("[fputest.ok] target={} task={} rounds={}", window.target, window.task, rounds);
        }
        Err((round, why)) => {
            let _ = write!(text, "FPU test FAILED in round {}: {}.", round, why);
            sex_pdx::serial_record!("[fputest.fail] target={} task={} round={} why={}", window.target, window.task, round, why);
        }
    }
    window.show(text.as_bytes());
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[fputest.closed] target={} task={}", window.target, window.task);
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
fn heapless_text() -> Text { Text { buf: [0; 256], len: 0 } }
