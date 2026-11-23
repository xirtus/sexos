// SPDX-License-Identifier: GPL-3.0-only
//! sexnet runs on its own 512 KiB stack. The loader gives a service 64 KiB, and the
//! Wi-Fi path alone (open, bring-up, association, WPA2) needs more: on the x17 R1, the
//! first machine with an Intel Wi-Fi lease, sexnet page-faulted entering `Wifi::open`
//! (`_start`'s 51 KiB frame plus its 19 KiB one), taking Ethernet down with it.

const STACK_BYTES: usize = 512 * 1024;
#[repr(C, align(16))]
struct Stack([u8; STACK_BYTES]);
static mut STACK: Stack = Stack([0; STACK_BYTES]);

core::arch::global_asm!(r#"
.global sexnet_on_stack
sexnet_on_stack:
    mov rsp, rsi
    xor ebp, ebp
    call rdi
    ud2
"#);
extern "C" { fn sexnet_on_stack(f: extern "C" fn() -> !, stack_top: u64) -> !; }

extern "C" fn enter() -> ! { crate::_start() }

#[export_name = "_start"]
pub extern "C" fn start() -> ! {
    let top = unsafe { (core::ptr::addr_of_mut!(STACK.0) as *mut u8).add(STACK_BYTES) as u64 } & !15;
    unsafe { sexnet_on_stack(enter, top) }
}
