#![no_std]
#![no_main]

extern crate alloc;
extern crate spin;
extern crate sex_rt;

mod vfs;
mod messages;
mod pdx;
mod trampoline;
mod backends;
mod proof;
mod sexobject;
mod spindle_history;
mod spindle_probe;
mod document;
mod document_journal;
mod history_journal;
mod history_disk;

use crate::trampoline::trampoline_main;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    crate::pdx::serial_println!("[sexfiles.init.start] ok=1");
    // sexfiles: RamFS Contract Lock V1
    // Bounded RAM-backed filesystem with no POSIX semantics.
    sex_rt::heap_init();
    // trampoline_main publishes readiness after its handlers and backends
    // initialize. Heap initialization alone does not make requests serviceable.
    crate::pdx::serial_println!("[sexfiles.init.before_trampoline] ok=1");
    trampoline_main();
    crate::pdx::serial_println!("[sexfiles.trampoline.return.unexpected] ok=0");

    // Fallback: trampoline_main should never return
    loop {
        unsafe {
            core::arch::asm!("syscall", in("rax") 24, lateout("rcx") _, lateout("r11") _);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe {
            core::arch::asm!("syscall", in("rax") 24, lateout("rcx") _, lateout("r11") _);
        }
    }
}
