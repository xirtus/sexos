// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The lock password: Files keeps only an Argon2id hash in `/system/password` and
//! answers whether a password matches (`sex_config::secret`). Only the program run
//! from `/data/settings` may set it.
use sex_config::secret::{self as s, Staging, Store, STORED_MAX};

pub const PATH: &[u8] = b"/system/password";
const CALLERS: usize = 8;
static mut STAGES: [(u32, Staging); CALLERS] = [const { (0, Staging::new()) }; CALLERS];

struct Disk;
impl Store for Disk {
    fn load(&mut self, out: &mut [u8; STORED_MAX]) -> Option<usize> {
        crate::fs_service::read_own(PATH, out).filter(|n| *n > 0)
    }
    fn save(&mut self, text: &[u8]) -> bool { crate::fs_service::replace_own(PATH, text) }
    /// A salt need only be unique: the CPU's RDRAND, mixed with time and a counter.
    fn salt(&mut self) -> [u8; 16] {
        static mut COUNT: u64 = 0;
        let mut out = [0u8; 16];
        for half in out.chunks_mut(8) {
            let mut r: u64 = 0;
            let mut ok: u8 = 0;
            for _ in 0..10 {
                unsafe { core::arch::asm!("rdrand {r}", "setc {ok}", r = out(reg) r, ok = out(reg_byte) ok, options(nomem, nostack)); }
                if ok != 0 { break; }
            }
            let count = unsafe { COUNT += 1; COUNT };
            let mixed = r ^ sex_pdx::get_ticks().rotate_left(17) ^ count.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            half.copy_from_slice(&mixed.to_le_bytes());
        }
        out
    }
}

fn stage(caller: u32) -> &'static mut Staging {
    let stages = unsafe { &mut *core::ptr::addr_of_mut!(STAGES) };
    let at = stages.iter().position(|(c, _)| *c == caller)
        .or_else(|| stages.iter().position(|(c, _)| *c == 0))
        .unwrap_or((caller as usize) % CALLERS);
    stages[at].0 = caller;
    &mut stages[at].1
}

/// Argon2 and the disk write after it need more stack than Files' own, which is small
/// and already deep when a request arrives. They run on this one instead.
const DEEP: usize = 256 * 1024;
#[repr(C, align(16))]
struct Stack([u8; DEEP]);
static mut STACK: Stack = Stack([0; DEEP]);

fn on_deep_stack(f: &mut dyn FnMut()) {
    extern "C" fn enter(f: *mut &mut dyn FnMut()) { unsafe { (*f)() } }
    let top = unsafe { core::ptr::addr_of_mut!(STACK) as usize } + DEEP;
    let mut f = f;
    // SAFETY: Files serves one request at a time, so the stack is ours alone; r12 is
    // callee-saved and brings the old stack pointer back across the call.
    unsafe {
        core::arch::asm!("mov r12, rsp", "mov rsp, {top}", "call {enter}", "mov rsp, r12",
            top = in(reg) top, enter = sym enter, in("rdi") &mut f as *mut &mut dyn FnMut(),
            out("r12") _, clobber_abi("C"));
    }
}

/// One request; the word to reply with.
pub fn handle(caller: u32, arg0: u64, arg1: u64) -> u64 {
    let may_set = crate::fs_service::program_is(caller, b"settings");
    let mut status = s::INVALID;
    on_deep_stack(&mut || status = s::serve(stage(caller), may_set, arg0, arg1, &mut Disk));
    if let Some((verb, _)) = sex_config::wire::decode_request(arg0).filter(|(v, _)| *v != s::PUT) {
        sex_pdx::serial_record!("[files.password] caller={} verb={} status={}", caller, verb, status);
    }
    s::reply(status)
}
