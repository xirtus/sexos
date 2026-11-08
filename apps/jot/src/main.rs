// SPDX-License-Identifier: GPL-3.0-only
//! Jot: a notepad that keeps its notes in its own folder. The first program built on
//! sex-app, installed as a file and run like any other.
#![no_std]
#![no_main]

use sex_app::{files, Event, Nav, Window};

const FILE: &[u8] = b"notes.txt";
const HEAD: &[u8] = b"Jot. Arrows move, Ctrl+S saves, F11 closes.\n\n";
/// Lines shown above the cursor's line, so the cursor stays on screen in a long note.
const ABOVE: usize = 16;
static mut NOTE: [u8; 4096] = [0; 4096];
static mut SCREEN: [u8; 4400] = [0; 4400];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[jot.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn line_start(note: &[u8], at: usize) -> usize { note[..at].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1) }
fn line_end(note: &[u8], at: usize) -> usize { note[at..].iter().position(|b| *b == b'\n').map_or(note.len(), |i| at + i) }
fn render(window: &mut Window, note: &[u8], cursor: usize, status: &[u8]) {
    let screen = unsafe { &mut *core::ptr::addr_of_mut!(SCREEN) };
    // From a few lines above the cursor, with the cursor drawn as a bar.
    let mut from = line_start(note, cursor);
    for _ in 0..ABOVE { if from == 0 { break; } from = line_start(note, from - 1); }
    let mut n = 0;
    for part in [HEAD, &note[from..cursor], b"|", &note[cursor..], b"\n\n", status] {
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
    // A file the person gave, as `@/path`, or the notes in Jot's own folder.
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let file: &[u8] = match given.split(|b| *b == b' ').find(|w| w.first() == Some(&b'@')) { Some(f) => f, None => FILE };
    let note = unsafe { &mut *core::ptr::addr_of_mut!(NOTE) };
    let mut len = match files::read(&mut window, file, note) {
        Ok(n) => { sex_pdx::serial_record!("[jot.loaded] bytes={}", n); n }
        Err(e) => { sex_pdx::serial_record!("[jot.loaded] bytes=0 status={}", e); 0 }
    };
    let mut cursor = len;
    render(&mut window, &note[..len], cursor, b"");
    sex_pdx::serial_record!("[jot.ready] target={} task={}", window.target, window.task);
    loop {
        let status: &[u8] = match window.next() {
            Event::Close => { sex_pdx::serial_record!("[jot.closed] target={} task={}", window.target, window.task); window.close() }
            // Ctrl+S saves.
            Event::Key(key) if key.control && key.code == 0x1F => match files::write(&mut window, file, &note[..len]) {
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
            Event::Key(key) if key.code == 0x0E => {
                if cursor > 0 { note.copy_within(cursor..len, cursor - 1); cursor -= 1; len -= 1; }
                b""
            }
            Event::Key(key) => {
                let typed = if key.code == 0x1C { Some(b'\n') } else { sex_app::typed(key) };
                match (sex_app::nav(key), typed) {
                    (Some(Nav::Delete), _) => { if cursor < len { note.copy_within(cursor + 1..len, cursor); len -= 1; } }
                    (Some(Nav::Left), _) => cursor = cursor.saturating_sub(1),
                    (Some(Nav::Right), _) => cursor = (cursor + 1).min(len),
                    (Some(Nav::Home), _) => cursor = line_start(&note[..len], cursor),
                    (Some(Nav::End), _) => cursor = line_end(&note[..len], cursor),
                    (Some(dir @ (Nav::Up | Nav::Down)), _) => {
                        // Same column on the line above or below, or that line's end.
                        let text = &note[..len];
                        let start = line_start(text, cursor);
                        let column = cursor - start;
                        let target = if dir == Nav::Up { if start == 0 { None } else { Some(line_start(text, start - 1)) } }
                            else { let end = line_end(text, cursor); (end < len).then_some(end + 1) };
                        if let Some(t) = target { cursor = (t + column).min(line_end(text, t)); }
                    }
                    (None, Some(c)) if len < note.len() => {
                        note.copy_within(cursor..len, cursor + 1);
                        note[cursor] = c; cursor += 1; len += 1;
                    }
                    _ => {}
                }
                b""
            }
        };
        render(&mut window, &note[..len], cursor, status);
    }
}
