// SPDX-License-Identifier: GPL-3.0-only
//! Settings: one place for every setting SexOS keeps (`sex-config`).
//!
//! Pages are the schema's sections; typing searches every setting by name. Left and
//! Right change the chosen value and Files saves it at once; the Shell and other
//! parts of the system pick the change up from Files. Files lets this program, run
//! from `/data/settings`, change settings; any other program may only read them.
#![no_std]
#![no_main]

mod about;
mod facts;
mod grants;
mod model;
mod setup;

use model::{Change, Model};
use sex_app::settings;
use sex_app::{Event, Nav, Window};
use sex_config::{Config, KEYS};
use sex_pdx::package as pkg;

const _: () = assert!(grants::GRANTS[0].0 == pkg::NETWORK && grants::GRANTS[1].0 == pkg::MICROPHONE && grants::MACHINE == pkg::MACHINE);

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

/// The Privacy page's program rows, from Files. Files lets this program, and no other,
/// list grants and change the person's answers (`sex_pdx::package`).
fn load_grants(window: &mut Window, m: &mut Model, rows: &mut [grants::Row; grants::ROWS_MAX]) -> usize {
    let (status, report) = sex_app::files::package(window, pkg::GRANTS, b"", 0);
    let n = if status == sex_pdx::fs::OK { grants::parse(&report, rows) } else { 0 };
    let defaults = [m.values.value(sex_config::id::FIRST_RUN_NETWORK), m.values.value(sex_config::id::FIRST_RUN_MICROPHONE)];
    let mut text = [0u8; model::EXTRA_MAX];
    let k = grants::render(&rows[..n], defaults, &mut text);
    m.set_extra(&text[..k], n);
    sex_pdx::serial_record!("[settings.grants] status={} rows={}", status, n);
    n
}
/// Left (take away) or Right (give) on a program row: the person's answer, to Files.
fn change_grant(window: &mut Window, m: &mut Model, row: &grants::Row, allow: bool) {
    let (status, _) = sex_app::files::package(window, pkg::CONSENT, row.program(), grants::consent(row, allow));
    let program = core::str::from_utf8(row.program()).unwrap_or("?");
    sex_pdx::serial_record!("[settings.grant] program={} grant={} allow={} status={}", program, row.word(), allow as u8, status);
    m.say(if status != sex_pdx::fs::OK { "Files did not take the change." } else if allow { "Allowed from its next run." } else { "Taken away from its next run." });
}

/// The Updates, Storage or About page's lines: Files' facts (`sex_pdx::facts`, answered
/// to this program alone), and on About the kernel's numbers first.
fn load_info(window: &mut Window, m: &mut Model, page: &str) {
    use sex_pdx::facts as f;
    let (topic, render): (u64, fn(&[u8], &mut [u8]) -> usize) = match page {
        "updates" => (f::UPDATES, facts::updates),
        "storage" => (f::STORAGE, facts::storage),
        _ => (f::ABOUT, facts::about),
    };
    let mut data = [0u8; 2048];
    let got = sex_app::files::facts(window, topic, &mut data);
    let data = match got { Ok(n) => &data[..n], Err(_) => &[][..] };
    let mut text = [0u8; model::INFO_MAX];
    let mut n = if topic == f::ABOUT { about_lines(&mut text) } else { 0 };
    n += render(data, &mut text[n..]);
    m.set_info(&text[..n]);
    let word = |k| facts::field(data, k).and_then(|v| core::str::from_utf8(v).ok()).unwrap_or("-");
    sex_pdx::serial_record!("[settings.facts] page={} status={} slot={} version={} filesystem={} health={} image={}", page, got.err().unwrap_or(0), word("slot"), word("version"), word("filesystem"), word("health"), word("image_sha256"));
}

/// The About page's lines from the kernel's numbers (syscall 87); returns their length.
fn about_lines(text: &mut [u8]) -> usize {
    use sex_pdx::{mesh_stats as ms, system_stats as ss};
    let overview = ss::overview();
    let memory = ms::memory();
    let kernel = ss::kernel();
    let mut domains = 0;
    let mut at = 0;
    while let Some(pd) = ss::next(at) { if pd <= at || domains >= 4096 { break; } domains += 1; at = pd; }
    let k = about::Facts {
        cores: overview.map(|o| o.cores), program_limit: memory.map(|x| x.limit_pages), program_lent: memory.map(|x| x.lent_pages),
        free_frames: kernel.map(|k| k.free_frames), kernel_heap: kernel.map(|k| k.heap_used), domains, uptime_s: sex_pdx::get_ticks() / 1000,
    };
    sex_pdx::serial_record!("[settings.about] cores={} domains={} limit_pages={}", k.cores.unwrap_or(0), domains, k.program_limit.unwrap_or(0));
    about::render(&k, env!("CARGO_PKG_VERSION"), text)
}

fn show(window: &mut Window, m: &Model) {
    let mut text = [0u8; 8192];
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
    window.title(b"Tatami");
    if settings::get(sex_config::id::SETUP_DONE).is_ok_and(|done| done == 0) { first_start(&mut window); }
    let (values, mut generation) = load();
    let mut m = Model::new(values);
    let mut rows = [grants::Row::EMPTY; grants::ROWS_MAX];
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
        sex_pdx::serial_record!("[settings.key] code={} page={} row={}", key.code, m.page, m.row);
        // Ctrl+P: the Shell keeps F-keys and Tab for itself, so Settings uses keys a
        // program receives.
        const KEY_P: u8 = 0x19;
        const PAGE_UP: u8 = 0x49;
        const PAGE_DOWN: u8 = 0x51;
        let page = m.page;
        let program_row = m.extra_row().filter(|_| !m.entering());
        let change = if let (Some(i), Some(nav)) = (program_row, sex_app::nav(key).filter(|n| matches!(n, Nav::Left | Nav::Right)).or((key.code == ENTER).then_some(Nav::Right))) {
            let row = rows[i];
            change_grant(&mut window, &mut m, &row, matches!(nav, Nav::Right));
            load_grants(&mut window, &mut m, &mut rows);
            Change::None
        } else if m.entering() {
            let c = match key.code { ENTER => 13, ESC => 27, BACKSPACE => 8, _ => sex_app::typed(key).unwrap_or(0) };
            m.password_key(c)
        } else if key.control && key.code == KEY_P {
            m.start_password();
            Change::None
        } else { match (sex_app::nav(key), key.code) {
            (Some(Nav::Up), _) => { m.move_row(false); Change::None }
            (Some(Nav::Down), _) => { m.move_row(true); Change::None }
            (Some(Nav::Left), _) => m.step(false, key.shift),
            (Some(Nav::Right), _) | (_, ENTER) => m.step(true, key.shift),
            (_, TAB) | (_, PAGE_DOWN) => { m.next_page(!(key.shift && key.code == TAB)); Change::None }
            (_, PAGE_UP) => { m.next_page(false); Change::None }
            (_, ESC) => { m.clear_search(); Change::None }
            (_, BACKSPACE) => { m.backspace(); Change::None }
            _ => {
                if let Some(c) = sex_app::typed(key) { m.type_char(c); }
                Change::None
            }
        } };
        if m.page != page && m.on_privacy() { load_grants(&mut window, &mut m, &mut rows); }
        if let Some(info) = m.info_page().filter(|_| m.page != page) { load_info(&mut window, &mut m, info); }
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
                Ok(()) => {
                    let _ = m.values.set(id, value); m.say("Saved.");
                    // A first-run default shows on the program rows too.
                    if m.on_privacy() { load_grants(&mut window, &mut m, &mut rows); }
                }
                Err(e) if e == sex_pdx::config::NOT_SAVED => { let _ = m.values.set(id, value); m.say("Changed, but the disk could not be written."); }
                Err(e) => m.say(settings::message(e)),
            }
            generation = settings::generation().unwrap_or(generation);
        }
        show(&mut window, &m);
    }
}
