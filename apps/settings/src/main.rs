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
fn load() -> (Config, u64) {
    let mut values = Config::new();
    let generation = settings::generation().unwrap_or(0);
    if let Ok(all) = settings::all() {
        for (k, v) in KEYS.iter().zip(all) { let _ = values.set(k.id, v); }
    }
    (values, generation)
}

fn show(window: &mut Window, m: &Model) {
    let mut text = [0u8; 4096];
    let n = model::render(m, &mut text);
    window.show(&text[..n]);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Settings");
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
        let change = match (sex_app::nav(key), key.code) {
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
        };
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
