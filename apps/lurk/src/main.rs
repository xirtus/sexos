// SPDX-License-Identifier: GPL-3.0-only
//! Lurk: a remote desktop viewer. `run +net lurk HOST:PORT [PASSWORD]` connects to a
//! VNC (RFB) server, shows its screen fitted to the window and sends the keys typed and
//! the mouse used over it. `HOST:N` with N under 100 is display N (port 5900 + N); no
//! port is 5900. The protocol is `sex-rfb`; the network is this run's own `+net` grant.
//!
//! VNC Authentication is DES-based and weak; use it only on networks you trust (or
//! through an ssh tunnel) until Lurk speaks a secured transport.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use sex_app::net::{self, TcpStream};
use sex_app::{files, Event, Window};
use sex_pdx::app_key::{us_char, Key};
use sex_pdx::app_pointer::{Kind, Pointer};
use sex_rfb::client::{Client, Event as Rfb};
use sex_rfb::{keysym, Framebuffer};

const BACK: u32 = 0x0010_1418;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[lurk.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn text(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }

/// `HOST:PORT`, `HOST:DISPLAY` or `HOST`.
fn target(arg: &[u8]) -> Option<(&[u8], u16)> {
    // An IPv6 address has colons of its own: `[addr]:port` or a bare address.
    if let Some(rest) = arg.strip_prefix(b"[") {
        let end = rest.iter().position(|b| *b == b']')?;
        let port = match rest.get(end + 1..)? { b"" => 5900, p => port(p.strip_prefix(b":")?)? };
        return Some((&rest[..end], port));
    }
    match arg.iter().rposition(|b| *b == b':') {
        Some(i) if arg.iter().filter(|b| **b == b':').count() == 1 => Some((&arg[..i], port(&arg[i + 1..])?)),
        _ => Some((arg, 5900)),
    }
}
fn port(p: &[u8]) -> Option<u16> {
    let n: u16 = text(p).parse().ok()?;
    Some(if n < 100 { 5900 + n } else { n })
}

/// Where the remote screen sits in the window: offset and scaled size.
#[derive(Clone, Copy, Default)]
struct Fit { x: usize, y: usize, w: usize, h: usize }

/// Draws the remote screen fitted (aspect kept) into the window's pixels.
fn draw(window: &mut Window, fb: &Framebuffer, xmap: &mut Vec<usize>) -> Fit {
    let out = window.pixels();
    let (mut aw, mut ah) = window.size();
    aw = aw.max(1);
    ah = ah.max(1).min(out.len() / aw);
    if fb.width == 0 || fb.height == 0 {
        out[..aw * ah].fill(BACK);
        window.show_pixels(aw, ah);
        return Fit::default();
    }
    let (w, h) = if fb.width * ah > fb.height * aw { (aw, (fb.height * aw / fb.width).max(1)) } else { ((fb.width * ah / fb.height).max(1), ah) };
    let fit = Fit { x: (aw - w) / 2, y: (ah - h) / 2, w, h };
    xmap.clear();
    xmap.extend((0..w).map(|x| x * fb.width / w));
    for row in 0..ah {
        let line = &mut out[row * aw..row * aw + aw];
        if row < fit.y || row >= fit.y + h { line.fill(BACK); continue; }
        let src = &fb.pixels[(row - fit.y) * fb.height / h * fb.width..][..fb.width];
        line[..fit.x].fill(BACK);
        for (d, &sx) in line[fit.x..fit.x + w].iter_mut().zip(xmap.iter()) { *d = src[sx]; }
        line[fit.x + w..].fill(BACK);
    }
    window.show_pixels(aw, ah);
    fit
}

/// The remote position under a window position.
fn remote(fit: Fit, fb: &Framebuffer, p: &Pointer) -> (u16, u16) {
    if fit.w == 0 || fit.h == 0 { return (0, 0); }
    let x = (p.x as isize - fit.x as isize).clamp(0, fit.w as isize - 1) as usize * fb.width / fit.w;
    let y = (p.y as isize - fit.y as isize).clamp(0, fit.h as isize - 1) as usize * fb.height / fit.h;
    (x as u16, y as u16)
}

/// SexOS buttons (bit 0 left, 1 right, 2 middle) as RFB's (0 left, 1 middle, 2 right).
fn buttons(b: u8) -> u8 { (b & 1) | ((b & 4) >> 1) | ((b & 2) << 1) }

fn key(client: &mut Client, k: Key) {
    // Control letters type nothing; the US letter under the key is what the remote wants.
    let typed = k.char().or_else(|| if k.control { us_char(Key { control: false, ..k }).map(|b| b as char) } else { None });
    let Some(sym) = keysym::from_key(k.code, typed) else { return };
    client.tap(sym, k.control);
    sex_pdx::serial_record!("[lurk.key] keysym=0x{:x} control={}", sym, k.control as u8);
}

fn pointer(client: &mut Client, fit: Fit, p: Pointer) {
    let (x, y) = remote(fit, &client.fb, &p);
    let held = buttons(p.buttons);
    match p.kind {
        Kind::Wheel { steps } => {
            let bit = if steps > 0 { 8 } else { 16 };
            for _ in 0..steps.unsigned_abs() { client.pointer(held | bit, x, y); client.pointer(held, x, y); }
        }
        Kind::Leave => {}
        _ => client.pointer(held, x, y),
    }
}

/// Shows why Lurk stopped and waits for the window to close.
fn stop(window: &mut Window, why: &str) -> ! {
    sex_pdx::serial_record!("[lurk.failed] {}", why);
    let mut s = alloc::string::String::from("Lurk. F11 closes.\n\n");
    s.push_str(why);
    window.show(s.as_bytes());
    loop { if let Event::Close = window.next() { window_close() } }
}

fn run(window: &mut Window, host: &[u8], port: u16, password: Option<&[u8]>) -> Result<(), &'static str> {
    // An address skips the name lookup, which is what waits for the network; a run
    // started right after boot would ask for a connection before the DHCP lease and
    // be refused. So wait for the network first (thirty seconds at most).
    let failed = |stage: &str, e: net::Error| { sex_pdx::serial_record!("[lurk.net] stage={} error={:?}", stage, e); text(net::message(e)) };
    net::up(window).map_err(|e| failed("up", e))?;
    let mut stream = TcpStream::connect(window, host, port).map_err(|e| failed("connect", e))?;
    sex_pdx::serial_record!("[lurk.tcp] host={} port={}", text(host), port);
    let mut client = Client::new(password);
    let mut buf = alloc::vec![0u8; 32 * 1024];
    let mut xmap = Vec::new();
    let (mut fit, mut dirty, mut shown) = (Fit::default(), false, 0u64);
    loop {
        let mut busy = false;
        for _ in 0..64 {
            let n = stream.read(window, &mut buf).map_err(|e| text(net::message(e)))?;
            if n == 0 { break; }
            client.feed(&buf[..n]);
            busy = true;
        }
        loop {
            let event = match client.next_event() {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    sex_pdx::serial_record!("[lurk.protocol] {:?}", e);
                    return Err(match e {
                        sex_rfb::Error::NeedPassword => "The server wants a password: lurk HOST:PORT PASSWORD",
                        sex_rfb::Error::AuthFailed(_) => "The server refused the password.",
                        sex_rfb::Error::Refused(_) => "The server refused the connection.",
                        sex_rfb::Error::NoSecurity => "The server wants a kind of security Lurk doesn't know yet.",
                        _ => "The server broke the RFB protocol.",
                    });
                }
            };
            match event {
                Rfb::Connected => {
                    sex_pdx::serial_record!("[lurk.connected] name={} size={}x{} version=3.{} security={}", text(&client.name), client.fb.width, client.fb.height, client.minor, client.security);
                    dirty = true;
                }
                Rfb::Updated { area, rects } => {
                    let n = client.updates;
                    if n <= 5 || n % 100 == 0 {
                        let a = area.unwrap_or(sex_rfb::Rect { x: 0, y: 0, w: 0, h: 0 });
                        sex_pdx::serial_record!("[lurk.frame] updates={} rects={} area={},{},{}x{}", n, rects, a.x, a.y, a.w, a.h);
                    }
                    dirty |= area.is_some();
                    client.request_update(true);
                }
                Rfb::Resized { width, height } => { sex_pdx::serial_record!("[lurk.resized] size={}x{}", width, height); dirty = true; }
                Rfb::Bell => sex_pdx::serial_record!("[lurk.bell]"),
                Rfb::CutText(t) => sex_pdx::serial_record!("[lurk.cut] bytes={}", t.len()),
                Rfb::Cursor => {}
            }
        }
        while let Some(event) = window.poll() {
            busy = true;
            match event {
                Event::Close => { sex_pdx::serial_record!("[lurk.closed] updates={}", client.updates); stream.close(window); window_close() }
                Event::Resize { .. } => dirty = true,
                Event::Key(k) if client.connected() => key(&mut client, k),
                Event::Pointer(p) if client.connected() => pointer(&mut client, fit, p),
                _ => {}
            }
        }
        let out = client.take_output();
        if !out.is_empty() { stream.write_all(window, &out).map_err(|e| text(net::message(e)))?; }
        if dirty && client.connected() {
            fit = draw(window, &client.fb, &mut xmap);
            dirty = false;
            shown += 1;
            if shown <= 3 { sex_pdx::serial_record!("[lurk.shown] n={} fit={}x{} at={},{} window={}x{}", shown, fit.w, fit.h, fit.x, fit.y, window.size().0, window.size().1); }
        }
        if !busy {
            if stream.is_closed(window).unwrap_or(true) { return Err("The server closed the connection."); }
            let _ = sex_pdx::sys_wait_self_ticks(2);
        }
    }
}

fn window_close() -> ! { sex_pdx::task_finish(0) }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Lurk");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let args = files::args(&mut window, &mut args).to_vec();
    let mut words = args.split(|b| *b == b' ').filter(|w| !w.is_empty());
    let Some((host, port)) = words.next().and_then(target) else { stop(&mut window, "Usage: run +net lurk HOST:PORT [PASSWORD]") };
    let password = words.next();
    let mut s = alloc::string::String::from("Lurk. Connecting to ");
    s.push_str(text(host));
    window.show(s.as_bytes());
    sex_pdx::serial_record!("[lurk.start] host={} port={} password={}", text(host), port, password.is_some() as u8);
    match run(&mut window, host, port, password) {
        Ok(()) => window.close(),
        Err(why) => stop(&mut window, why),
    }
}
