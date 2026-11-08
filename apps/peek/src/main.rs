// SPDX-License-Identifier: GPL-3.0-only
//! Peek: shows the text of one web page. `run +net peek URL`. Without `+net` the network
//! service refuses it, which is the point: a program reaches the network only when asked to.
#![no_std]
#![no_main]

use sex_app::{files, net, Event, Window};

static mut PAGE: [u8; 65536] = [0; 65536];
static mut SCREEN: [u8; 6000] = [0; 6000];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[peek.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// Text between tags, with runs of blank space folded. Enough to read a simple page.
fn text(html: &[u8], out: &mut [u8]) -> usize {
    let (mut n, mut in_tag, mut space) = (0, false, true);
    let mut i = 0;
    while i < html.len() {
        let b = html[i];
        // Styles and scripts are not text.
        for (open, close) in [(&b"<style"[..], &b"</style>"[..]), (b"<script", b"</script>")] {
            if html[i..].len() >= open.len() && html[i..i + open.len()].eq_ignore_ascii_case(open) {
                i += html[i..].windows(close.len()).position(|w| w.eq_ignore_ascii_case(close)).map_or(html.len() - i, |p| p + close.len());
            }
        }
        if i >= html.len() { break; }
        let b = if html[i] != b { html[i] } else { b };
        i += 1;
        match b {
            b'<' => in_tag = true,
            b'>' => { in_tag = false; if !space && n < out.len() { out[n] = b' '; n += 1; space = true; } }
            _ if in_tag => {}
            b'\n' | b'\r' | b'\t' | b' ' => if !space && n < out.len() { out[n] = b' '; n += 1; space = true; },
            32..=126 => if n < out.len() { out[n] = b; n += 1; space = false; },
            _ => {}
        }
    }
    n
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Peek");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let url = files::args(&mut window, &mut args);
    let (page, screen) = unsafe { (&mut *core::ptr::addr_of_mut!(PAGE), &mut *core::ptr::addr_of_mut!(SCREEN)) };
    let head: &[u8] = b"Peek. F11 closes.\n\n";
    screen[..head.len()].copy_from_slice(head);
    let mut n = head.len();
    if url.is_empty() {
        let say = b"Usage: run +net peek URL";
        screen[n..n + say.len()].copy_from_slice(say); n += say.len();
    } else {
        match net::get(&mut window, url, page) {
            Ok((status, len)) => {
                sex_pdx::serial_record!("[peek.got] status={} bytes={}", status, len);
                n += text(&page[..len], &mut screen[n..]);
            }
            Err(e) => {
                sex_pdx::serial_record!("[peek.refused] error={:?}", e);
                let say = net::message(e);
                screen[n..n + say.len()].copy_from_slice(say); n += say.len();
            }
        }
    }
    window.show(&screen[..n]);
    sex_pdx::serial_record!("[peek.ready] target={} task={}", window.target, window.task);
    loop {
        if let Event::Close = window.next() { sex_pdx::serial_record!("[peek.closed] target={} task={}", window.target, window.task); window.close() }
    }
}
