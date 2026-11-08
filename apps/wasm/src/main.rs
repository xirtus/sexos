// SPDX-License-Identifier: GPL-3.0-only
//! Wasm: runs a WebAssembly program in this window. `run wasm @game.wasm ARGS`, or
//! `run wasm` alone for a small built-in guest.
//!
//! The guest runs inside this program's confinement and narrows it further: its files
//! are `guests/NAME/` in this program's folder, and it reaches the network only when
//! this program was started with `run +net`. See `crates/sex-wasm`.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, Event, Window};
use sex_wasm::{sexos::AppHost, Grants, Guest, Limits, Outcome};

/// `crates/sex-wasm/guests/hello/hello.wat`.
const HELLO: &[u8] = include_bytes!("../../../crates/sex-wasm/guests/hello/hello.wasm");
/// The largest module this program loads.
const MODULE_MAX: usize = 2 << 20;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[wasm.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn say(window: &mut Window, text: &[u8]) -> ! {
    window.show(text);
    loop { if let Event::Close = window.next() { sex_pdx::task_finish(0) } }
}

/// The file's name without folders, `@` or `.wasm`: the guest's folder name.
fn stem(file: &[u8]) -> &[u8] {
    let base = file.rsplit(|b| *b == b'/').next().unwrap_or(file);
    let base = base.strip_prefix(b"@").unwrap_or(base);
    base.strip_suffix(b".wasm").unwrap_or(base)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Wasm");
    let mut raw = [0u8; sex_pdx::fs::ARGS_MAX];
    let given: Vec<u8> = files::args(&mut window, &mut raw).to_vec();
    let mut words = given.split(|b| *b == b' ').filter(|w| !w.is_empty());
    let (name, module): (&[u8], Vec<u8>) = match words.next() {
        None => (b"hello", HELLO.to_vec()),
        Some(file) => {
            let mut bytes = vec![0u8; MODULE_MAX + 1];
            match files::read(&mut window, file, &mut bytes) {
                Err(e) => say(&mut window, files::message(e)),
                Ok(n) if n > MODULE_MAX => say(&mut window, b"That module is over the 2 MiB limit."),
                Ok(n) => { bytes.truncate(n); (stem(file), bytes) }
            }
        }
    };
    let args: Vec<&[u8]> = words.collect();
    let mut folder = b"guests/".to_vec();
    folder.extend_from_slice(name);
    let guest = Guest {
        name,
        args: &args,
        // The network grant only passes this program's own on: without `+net` it is refused.
        grants: Grants { folder: Some(folder), net: true },
        limits: Limits { memory_bytes: 4 << 20, console_bytes: sex_pdx::text_region::BYTES as usize - 256, ..Limits::default() },
    };
    sex_pdx::serial_record!("[wasm.start] name={} bytes={} args={}", core::str::from_utf8(name).unwrap_or("?"), module.len(), args.len());
    let (outcome, mut host) = sex_wasm::run(&module, &guest, AppHost::new(window));
    drop(module);
    // The guest already took the window's Close: it ended because of it.
    if host.closed && !matches!(outcome, Outcome::Rejected(_)) {
        sex_pdx::serial_record!("[wasm.closed] shown={}", host.shown.len());
        sex_pdx::task_finish(0);
    }
    let ended: &[u8] = match &outcome {
        Outcome::Exited(code) => {
            sex_pdx::serial_record!("[wasm.exited] code={} shown={}", code, host.shown.len());
            b"\n[The program ended. F11 closes.]"
        }
        Outcome::Closed => { sex_pdx::serial_record!("[wasm.closed]"); host.window.close() }
        Outcome::OutOfFuel => b"\n[The program ran too long.]",
        Outcome::Trapped(why) => { sex_pdx::serial_record!("[wasm.trapped] {}", why); b"\n[The program crashed.]" }
        Outcome::Rejected(why) => {
            sex_pdx::serial_record!("[wasm.rejected] {}", why);
            let mut text = b"This file can't run: ".to_vec();
            text.extend_from_slice(why.as_bytes());
            say(&mut host.window, &text)
        }
    };
    let mut text = core::mem::take(&mut host.shown);
    text.extend_from_slice(ended);
    say(&mut host.window, &text)
}
