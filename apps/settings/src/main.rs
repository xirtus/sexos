// SPDX-License-Identifier: GPL-3.0-only
//! Settings: one place for every setting SexOS keeps (`sex-config`).
//!
//! Pages are the schema's sections; typing searches every setting by name. Left and
//! Right change the chosen value and Files saves it at once; the Shell and other
//! parts of the system pick the change up from Files. Files lets this program, run
//! from `/data/settings`, change settings; any other program may only read them.
#![no_std]
#![no_main]

mod model;
mod setup;

use model::{Change, Model};
use sex_app::settings;
use sex_app::{Event, Nav, Window};
use sex_config::{Config, KEYS};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[settings.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const TAB: u8 = 0x0F;
const ESC: u8 = 0x01;
const ENTER: u8 = 0x1C;
const BACKSPACE: u8 = 0x0E;

/// Every value as Files holds it now (defaults for any it cannot answer).
/// Every value Files holds now. A key Files does not answer keeps its default, and
/// the others still load.
fn load() -> (Config, u64) {
    let mut values = Config::new();
    let generation = settings::generation().unwrap_or(0);
    let mut failed = 0;
    for k in KEYS.iter() {
        match settings::get(k.id) { Ok(v) => { let _ = values.set(k.id, v); } Err(_) => failed += 1 }
    }
    sex_pdx::serial_record!("[settings.load] generation={} failed={} night={}", generation, failed, values.value(sex_config::id::NIGHT_LIGHT));
    (values, generation)
}

fn show(window: &mut Window, m: &Model) {
    let mut text = [0u8; 4096];
    let n = model::render(m, &mut text);
    window.show(&text[..n]);
}

/// First-start setup: runs until its last step or until the window closes.
fn first_start(window: &mut Window) {
    let mut s = setup::Setup::new();
    let mut status: &[u8] = b"";
    sex_pdx::serial_record!("[settings.setup] step=start");
    loop {
        let mut text = [0u8; 4096];
        let n = setup::render(&s, &mut text, status);
        window.show(&text[..n]);
        if s.step == setup::Step::Done { return; }
        let key = match window.next() { Event::Close => window_close(), Event::Key(k) => k, _ => continue };
        status = b"";
        match (sex_app::nav(key), key.code) {
            (Some(Nav::Up), _) => s.up_down(false),
            (Some(Nav::Down), _) => s.up_down(true),
            (_, ESC) => s.back(),
            (_, BACKSPACE) => s.backspace(),
            (_, ENTER) => {
                let step = s.step;
                let ok = match s.next() {
                    setup::Out::Nothing => true,
                    setup::Out::Values(values) => values.iter().filter(|(k, _)| *k != 0).all(|(k, v)| settings::set(*k, *v).is_ok()),
                    setup::Out::Password(mut p, n) => {
                        let r = settings::password(sex_config::secret::SET, &p[..n]);
                        for b in p.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) }; }
                        r == Ok(sex_config::secret::OK)
                    }
                };
                sex_pdx::serial_record!("[settings.setup] step={:?} ok={}", step, ok as u8);
                if !ok { status = b"That could not be saved; it can be changed later in Settings."; }
            }
            _ => if let Some(c) = sex_app::typed(key) { s.typed(c) },
        }
    }
}
fn window_close() -> ! { sex_pdx::task_finish(0) }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Settings");
    if settings::get(sex_config::id::SETUP_DONE).is_ok_and(|done| done == 0) { first_start(&mut window); }
    let (values, mut generation) = load();
    let mut m = Model::new(values);
    sex_pdx::serial_record!("[settings.open] generation={}", generation);
    show(&mut window, &m);
    loop {
        let Some(event) = window.next_within(1000) else {
            // Someone else changed a setting (a Shell key, say): show it.
            if settings::generation().is_ok_and(|g| g != generation) {
                let (values, g) = load();
                m.values = values;
                generation = g;
                show(&mut window, &m);
            }
            continue;
        };
        let key = match event {
            Event::Close => window.close(),
            Event::Key(key) => key,
            _ => continue,
        };
        const F2: u8 = 0x3C;
        let change = if m.entering() {
            let c = match key.code { ENTER => 13, ESC => 27, BACKSPACE => 8, _ => sex_app::typed(key).unwrap_or(0) };
            m.password_key(c)
        } else if key.code == F2 {
            m.start_password();
            Change::None
        } else { match (sex_app::nav(key), key.code) {
            (Some(Nav::Up), _) => { m.move_row(false); Change::None }
            (Some(Nav::Down), _) => { m.move_row(true); Change::None }
            (Some(Nav::Left), _) => m.step(false, key.shift),
            (Some(Nav::Right), _) | (_, ENTER) => m.step(true, key.shift),
            (_, TAB) => { m.next_page(!key.shift); Change::None }
            (_, ESC) => { m.clear_search(); Change::None }
            (_, BACKSPACE) => { m.backspace(); Change::None }
            _ => {
                if let Some(c) = sex_app::typed(key) { m.type_char(c); }
                Change::None
            }
        } };
        if let Change::Password(mut typed, n) = change {
            let result = settings::password(sex_config::secret::SET, &typed[..n]);
            for b in typed.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) }; }
            sex_pdx::serial_record!("[settings.password] status={}", match result { Ok(s) => s, Err(e) => e });
            m.say(match result { Ok(sex_config::secret::OK) => "Lock password set.", Ok(_) | Err(_) => "The password could not be saved." });
        }
        if let Change::Set(id, value) = change {
            let result = settings::set(id, value);
            sex_pdx::serial_record!("[settings.set] key={} value={} status={}", id, value, result.err().unwrap_or(0));
            match result {
                Ok(()) => { let _ = m.values.set(id, value); m.say("Saved."); }
                Err(e) if e == sex_pdx::config::NOT_SAVED => { let _ = m.values.set(id, value); m.say("Changed, but the disk could not be written."); }
                Err(e) => m.say(settings::message(e)),
            }
            generation = settings::generation().unwrap_or(generation);
        }
        show(&mut window, &m);
    }
}
