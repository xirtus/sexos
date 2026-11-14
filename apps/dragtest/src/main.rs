// SPDX-License-Identifier: GPL-3.0-only
//! Drag test (D1): the smallest VMM. `run dragtest`. It makes a VM, lends it one page of
//! its own heap holding a 16-bit real-mode guest that writes "Drag" to port 0xE9 and
//! halts, and runs the vCPU, handling each I/O exit itself, until HLT.
//!
//! Serial: `[drag.exit] reason=io port=0xe9 value=...` per byte, `[drag.guest] said=Drag`,
//! then `[drag.done]`. Without usable VMX the kernel refuses cleanly and this logs
//! `[drag.refused] status=3 reason=...`; a refusal of any other kind is a failure.
#![no_std]
#![no_main]

extern crate alloc;
use sex_app::{Event, Window};
use sex_pdx::vm::{self, exit, Exit};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[drag.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// mov al,'D'; out 0xE9,al; mov al,'r'; out 0xE9,al; mov al,'a'; out 0xE9,al;
/// mov al,'g'; out 0xE9,al; hlt
const GUEST: [u8; 17] = [0xB0, b'D', 0xE6, 0xE9, 0xB0, b'r', 0xE6, 0xE9, 0xB0, b'a', 0xE6, 0xE9, 0xB0, b'g', 0xE6, 0xE9, 0xF4];
/// Where the guest's code sits: guest-physical 0x1000, run as 0000:1000.
const GPA: u64 = 0x1000;

fn guest() -> Result<([u8; 16], usize, u32), (&'static str, u64)> {
    match vm::info() {
        Ok((true, _)) => {}
        Ok((false, reason)) => return Err(("unavailable", reason)),
        Err(s) => return Err(("info", s)),
    }
    let layout = core::alloc::Layout::from_size_align(4096, 4096).unwrap();
    let page = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if page.is_null() { return Err(("no_page", 0)); }
    unsafe { core::ptr::copy_nonoverlapping(GUEST.as_ptr(), page, GUEST.len()) };
    let machine = vm::create().map_err(|s| ("create", s))?;
    vm::lend(machine, page as u64, GPA, 1, vm::READ | vm::WRITE | vm::EXEC).map_err(|s| ("lend", s))?;
    let cpu = vm::vcpu_create(machine).map_err(|s| ("vcpu", s))?;
    let mut state = vm::State::real_mode(0, GPA as u16);
    vm::set_state(machine, cpu, &state).map_err(|s| ("set_regs", s))?;
    let (mut said, mut len, mut exits) = ([0u8; 16], 0usize, 0u32);
    let mut record = Exit::default();
    let mut load = false;
    loop {
        vm::run(machine, cpu, &mut record, load).map_err(|s| ("run", s))?;
        exits += 1;
        match record.reason {
            exit::IO if record.io_flags & vm::IO_IN == 0 => {
                let value = record.regs.rax as u8;
                sex_pdx::serial_record!("[drag.exit] reason=io port={:#x} value={:#x} size={} rip={:#x}", record.port, value, record.io_size, record.regs.rip);
                if record.port == 0xE9 && len < said.len() { said[len] = value; len += 1; }
                record.regs.rip += record.instruction_len as u64;
                load = true;
            }
            exit::HLT => { sex_pdx::serial_record!("[drag.exit] reason=hlt rip={:#x}", record.regs.rip); break; }
            other => {
                sex_pdx::serial_record!("[drag.exit] reason={:#x} qualification={:#x} rip={:#x}", other, record.qualification, record.regs.rip);
                return Err(("unexpected_exit", other as u64));
            }
        }
        if exits > 64 { return Err(("too_many_exits", exits as u64)); }
    }
    vm::get_state(machine, cpu, &mut state).map_err(|s| ("get_regs", s))?;
    vm::destroy(machine).map_err(|s| ("destroy", s))?;
    Ok((said, len, exits))
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Drag test");
    sex_pdx::serial_record!("[drag.start] target={} task={}", window.target, window.task);
    let text: &[u8] = match guest() {
        Ok((said, len, exits)) => {
            let said = core::str::from_utf8(&said[..len]).unwrap_or("?");
            sex_pdx::serial_record!("[drag.guest] said={} exits={}", said, exits);
            sex_pdx::serial_record!("[drag.done] target={} task={}", window.target, window.task);
            b"Drag test: the guest said its word.\nF11 closes."
        }
        Err(("unavailable", reason)) => {
            sex_pdx::serial_record!("[drag.refused] status={} reason={}", vm::UNAVAILABLE, reason);
            // Every op is refused cleanly too, not just INFO.
            let create = vm::create();
            sex_pdx::serial_record!("[drag.refused.create] status={}", create.err().unwrap_or(0));
            b"Drag test: this machine has no usable VMX.\nF11 closes."
        }
        Err((step, status)) => {
            sex_pdx::serial_record!("[drag.fail] step={} status={}", step, status);
            b"Drag test FAILED.\nF11 closes."
        }
    };
    window.show(text);
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[drag.closed] target={} task={}", window.target, window.task);
            window.close()
        }
    }
}
