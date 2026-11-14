// SPDX-License-Identifier: GPL-3.0-only
//! Shuttle: carries a run across, starting another program the way any program can
//! (`sex_app::launch`). `run shuttle @photo.png` opens the file in the program for its
//! type, handing it that file alone; `run shuttle NAME ARGS` runs a program. `-k` ends what it started at
//! once; `-a` also hands on the whole filesystem, which Files refuses unless this run
//! holds it; `-h` runs NAME as a hidden helper, with no window on screen.
//! It shows whether the run started, was refused, and how it ended. What it starts
//! gets only what this run was given.
#![no_std]
#![no_main]

use sex_app::{files, launch, Event, Window};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[shuttle.fatal] {}", info);
    sex_pdx::task_finish(1);
}

struct Screen { text: [u8; 1024], n: usize }
impl Screen {
    fn say(&mut self, window: &mut Window, parts: &[&[u8]]) {
        for part in parts { for b in *part { if self.n < self.text.len() { self.text[self.n] = *b; self.n += 1; } } }
        if self.n < self.text.len() { self.text[self.n] = b'\n'; self.n += 1; }
        window.show(&self.text[..self.n]);
    }
}
fn number(mut v: u32, out: &mut [u8; 10]) -> &[u8] {
    let mut i = out.len();
    loop { i -= 1; out[i] = b'0' + (v % 10) as u8; v /= 10; if v == 0 { break; } }
    &out[i..]
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Shuttle");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let line = files::args(&mut window, &mut args);
    let mut screen = Screen { text: [0; 1024], n: 0 };
    let (kill, line) = match line.strip_prefix(b"-k ") { Some(rest) => (true, rest), None => (false, line) };
    let (all, line) = match line.strip_prefix(b"-a ") { Some(rest) => (true, rest), None => (false, line) };
    let (hidden, line) = match line.strip_prefix(b"-h ") { Some(rest) => (true, rest), None => (false, line) };
    let (first, rest) = match line.iter().position(|b| *b == b' ') { Some(i) => (&line[..i], &line[i + 1..]), None => (line, &b""[..]) };
    // `-a` hands on the whole filesystem too, which works only if this run holds it.
    let mut joined = [0u8; sex_pdx::fs::ARGS_MAX];
    let rest = if all && rest.len() + 3 <= joined.len() {
        joined[..rest.len()].copy_from_slice(rest);
        let at = if rest.is_empty() { 0 } else { joined[rest.len()] = b' '; rest.len() + 1 };
        joined[at..at + 2].copy_from_slice(b"@/");
        &joined[..at + 2]
    } else { rest };
    const COOKIE: u32 = 1;
    let asked = match first {
        [] => { screen.say(&mut window, &[b"Usage: run shuttle @FILE, or run shuttle NAME ARGS"]); Ok(()) }
        [b'@', path @ ..] if rest.is_empty() && path.first() == Some(&b'/') => launch::open(path, COOKIE),
        name if hidden => launch::run_hidden(name, rest, COOKIE),
        name => launch::run(name, rest, false, COOKIE),
    };
    match asked {
        Err(e) => {
            sex_pdx::serial_record!("[shuttle.refused] by=files error={:?}", e);
            let why: &[u8] = match e { launch::Error::NoProgram => b"No program opens files of that type.", launch::Error::Shell => b"The desktop could not be asked.", launch::Error::Files(c) => files::message(c) };
            screen.say(&mut window, &[b"Not started: ", why]);
        }
        Ok(()) if !line.is_empty() => loop {
            let mut digits = [0u8; 10];
            match launch::wait(COOKIE) {
                launch::Event::Started { row, task, .. } => {
                    sex_pdx::serial_record!("[shuttle.started] row={} task={} kill={} hidden={}", row, task, kill as u8, hidden as u8);
                    screen.say(&mut window, &[b"Started on row ", number(row, &mut digits)]);
                    if kill { launch::kill(COOKIE); }
                }
                launch::Event::Refused { code, .. } => {
                    sex_pdx::serial_record!("[shuttle.refused] by=shell code={}", code);
                    screen.say(&mut window, &[b"Not started: ", sex_pdx::program_store::refusal(code as u64)]);
                    break;
                }
                launch::Event::Done { status, .. } => {
                    sex_pdx::serial_record!("[shuttle.done] status={:?}", status);
                    match status {
                        Some(s) => screen.say(&mut window, &[b"It ended with status ", number(s, &mut digits)]),
                        None => screen.say(&mut window, &[b"It was ended."]),
                    }
                    break;
                }
            }
        },
        Ok(()) => {}
    }
    loop {
        match window.next() {
            Event::Close => window.close(),
            // Tiling resized the window: the text is drawn again to fit.
            Event::Resize { .. } => window.show(&screen.text[..screen.n]),
            _ => {}
        }
    }
}
