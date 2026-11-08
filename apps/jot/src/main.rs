// SPDX-License-Identifier: GPL-3.0-only
//! Jot: a notepad that keeps its notes in its own folder. The first program built on
//! sex-app, installed as a file and run like any other.
#![no_std]
#![no_main]

use sex_app::{files, Event, Window};

const FILE: &[u8] = b"notes.txt";
const HEAD: &[u8] = b"Jot. Type a note. Ctrl+S saves, F11 closes.\n\n";
static mut NOTE: [u8; 4096] = [0; 4096];
static mut SCREEN: [u8; 4400] = [0; 4400];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[jot.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn render(window: &mut Window, note: &[u8], status: &[u8]) {
    let screen = unsafe { &mut *core::ptr::addr_of_mut!(SCREEN) };
    let mut n = 0;
    for part in [HEAD, note, b"_\n\n", status] {
        let take = part.len().min(screen.len() - n);
        screen[n..n + take].copy_from_slice(&part[..take]);
        n += take;
    }
    window.show(&screen[..n]);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Jot");
    let note = unsafe { &mut *core::ptr::addr_of_mut!(NOTE) };
    let mut len = match files::read(&mut window, FILE, note) {
        Ok(n) => { sex_pdx::serial_record!("[jot.loaded] bytes={}", n); n }
        Err(e) => { sex_pdx::serial_record!("[jot.loaded] bytes=0 status={}", e); 0 }
    };
    render(&mut window, &note[..len], b"");
    sex_pdx::serial_record!("[jot.ready] target={} task={}", window.target, window.task);
    loop {
        let status: &[u8] = match window.next() {
            Event::Close => { sex_pdx::serial_record!("[jot.closed] target={} task={}", window.target, window.task); window.close() }
            // Ctrl+S saves.
            Event::Key(key) if key.control && key.code == 0x1F => match files::write(&mut window, FILE, &note[..len]) {
                Ok(()) => { sex_pdx::serial_record!("[jot.saved] bytes={}", len); b"Saved." }
                Err(e) => { sex_pdx::serial_record!("[jot.save.failed] status={}", e); files::message(e) }
            },
            // Ctrl+E tries to leave its folder, which Files must refuse.
            Event::Key(key) if key.control && key.code == 0x12 => {
                let mut probe = [0u8; 16];
                let result = files::read(&mut window, b"../../bin/jot", &mut probe);
                sex_pdx::serial_record!("[jot.escape] refused={} status={}", result.is_err() as u8, result.err().unwrap_or(0));
                b"Only this program's folder is reachable."
            }
            Event::Key(key) if key.code == 0x0E => { len = len.saturating_sub(1); b"" }
            Event::Key(key) if key.code == 0x1C && len < note.len() => { note[len] = b'\n'; len += 1; b"" }
            Event::Key(key) => match sex_app::typed(key) {
                Some(c) if len < note.len() => { note[len] = c; len += 1; b"" }
                _ => b"",
            },
        };
        render(&mut window, &note[..len], status);
    }
}
