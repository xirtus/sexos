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
mod candy_gui;
mod facts;
mod grants;
mod model;
mod setup;

use candy_gui::{Glyphs, GuiState, SidebarView, UiAction};
use model::{Change, Model};
use sex_app::settings;
use sex_app::{Event, Nav, Window};
use sex_config::{id, Config, KEYS};
use sex_pdx::app_pointer::Kind as PointerKind;
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
/// to this program alone), and on About the kernel's numbers first. True while Files is
/// still hashing the boot image, a step per request: ask again soon.
fn load_info(window: &mut Window, m: &mut Model, page: &str) -> bool {
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
    sex_pdx::serial_record!("[settings.facts] page={} status={} slot={} version={} filesystem={} health={} image={}", page, got.err().unwrap_or(0), word("slot"), word("version"), word("filesystem"), word("health"), word("image"));
    facts::hashing(data)
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

/// Queries `sexfiles` for the raw GPT partition table (`INSTALL_LIST | INSTALL_LIST_RAW`).
fn refresh_install_parts(window: &mut Window, gui: &mut GuiState, m: &mut Model) {
    let mut raw = [0u8; 2048];
    let flags = sex_pdx::fs::INSTALL_LIST | sex_pdx::fs::INSTALL_LIST_RAW;
    match sex_app::files::install(window, b"", 0, flags, &mut raw) {
        Ok(n) => {
            gui.install.parse_raw(&raw[..n]);
            sex_pdx::serial_record!("[settings.install.list] parts={} total_sectors={}", gui.install.part_count, gui.install.total_sectors);
            m.say("Disk partition table refreshed.");
        }
        Err(e) => {
            gui.install.loaded = true;
            gui.install.set_report(b"Unable to read disk partitions from Files.");
            sex_pdx::serial_record!("[settings.install.list] error={}", e);
            m.say("Could not read disk partition table.");
        }
    }
}

/// Dispatches a dry-run (`go = false`) or atomic (`go = true`) install request to `sexfiles`.
fn run_install_action(window: &mut Window, gui: &mut GuiState, m: &mut Model, go: bool) {
    let Some(row) = gui.install.selected_row().copied() else {
        m.say("Select a replaceable partition or Free Space first.");
        return;
    };
    let (offset, mut flags) = if row.kind == candy_gui::DiskPartKind::FreeSpace {
        (0u64, 0u64)
    } else {
        (row.part_num as u64, sex_pdx::fs::INSTALL_REPLACE)
    };
    if go {
        flags |= sex_pdx::fs::INSTALL_GO;
        m.say("Installing SexOS... Do not power off.");
        show(window, m, gui);
    }
    let mut out = [0u8; 2048];
    match sex_app::files::install(window, b"", offset, flags, &mut out) {
        Ok(n) => {
            gui.install.set_report(&out[..n]);
            gui.install.plan_ready = !go;
            sex_pdx::serial_record!("[settings.install.run] go={} part={} bytes={}", go as u8, offset, n);
            m.say(if go { "Install completed. Restart to boot SexOS." } else { "Dry-run plan ready. Nothing written yet." });
        }
        Err(e) => {
            gui.install.set_report(b"Installer request was refused by Files.");
            sex_pdx::serial_record!("[settings.install.run] go={} part={} error={}", go as u8, offset, e);
            m.say("Install request failed.");
        }
    }
}

/// Bounded-timeout `sexnet` Wi-Fi status RPC (`sex_pdx::wifi_status`).
fn wifi_rpc(action: u64, a: u64, b: u64) -> Option<u64> {
    use sex_pdx::wifi_status as ws;
    sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, ws::OP, action, a, b).ok()?;
    let began = sex_pdx::get_ticks();
    while sex_pdx::get_ticks().saturating_sub(began) < 350 {
        if let Some(msg) = sex_pdx::pdx_try_listen_raw(0) {
            if msg.type_id == 1 && msg.caller_pd == 13 && ws::is_reply(msg.arg0) {
                return Some(msg.arg0);
            }
        } else {
            sex_pdx::sys_yield();
        }
    }
    None
}

/// Loads `/data/wifi.conf` (shared with Spindle) into `gui.wifi`.
fn load_wifi_conf(window: &mut Window, gui: &mut GuiState) {
    let mut buf = [0u8; candy_gui::WIFI_CONF_MAX];
    match sex_app::files::read(window, candy_gui::WIFI_CONF_PATH, &mut buf) {
        Ok(n) => {
            gui.wifi.parse_wifi_conf(&buf[..n]);
            sex_pdx::serial_record!("[settings.wifi.load] known={} bytes={}", gui.wifi.known_count, n);
        }
        Err(_) => {
            gui.wifi.known_count = 0;
            gui.wifi.loaded = true;
        }
    }
}

/// Persists `gui.wifi.known` to `/data/wifi.conf` in the multi-network format shared with Spindle.
fn save_wifi_conf(window: &mut Window, gui: &GuiState, m: &mut Model) -> bool {
    let _ = sex_app::files::mkdir(window, b"/data");
    let mut buf = [0u8; candy_gui::WIFI_CONF_MAX];
    let n = gui.wifi.serialize_wifi_conf(&mut buf);
    match sex_app::files::write(window, candy_gui::WIFI_CONF_PATH, &buf[..n]) {
        Ok(()) => {
            sex_pdx::serial_record!("[settings.wifi.save] known={} bytes={}", gui.wifi.known_count, n);
            true
        }
        Err(_) => {
            m.say("Could not write /data/wifi.conf.");
            false
        }
    }
}

/// Queries live Wi-Fi chip status and visible scanned networks from `sexnet`.
fn refresh_wifi_live(gui: &mut GuiState) {
    use sex_pdx::wifi_status as ws;
    let Some(word) = wifi_rpc(ws::STATE, 0, 0) else {
        gui.wifi.connected = false;
        gui.wifi.set_status(b"Wi-Fi service unavailable");
        return;
    };
    let s = ws::State::of(word);
    gui.wifi.connected = matches!(s.phase, ws::ASSOCIATED | ws::CONNECTED);
    match s.phase {
        ws::NONE => gui.wifi.set_status(b"No Intel Wi-Fi hardware detected"),
        ws::STARTING => gui.wifi.set_status(b"Starting Wi-Fi firmware..."),
        ws::SCANNING => gui.wifi.set_status(b"Scanning nearby networks..."),
        ws::READY => gui.wifi.set_status(b"Ready (disconnected)"),
        ws::JOINING | ws::JOINED | ws::ASSOCIATING | ws::HANDSHAKE => gui.wifi.set_status(b"Connecting..."),
        ws::ASSOCIATED => gui.wifi.set_status(b"Connected (Open)"),
        ws::CONNECTED => gui.wifi.set_status(b"Connected (WPA2/WPA3)"),
        _ => gui.wifi.set_status(b"Last join failed - click Scan Wi-Fi"),
    }

    // Fetch up to MAX_WIFI_SCAN visible networks when available.
    gui.wifi.scan_count = 0;
    let limit = (s.networks as usize).min(candy_gui::MAX_WIFI_SCAN);
    for idx in 0..limit {
        let Some(info_word) = wifi_rpc(ws::NETWORK, idx as u64, ws::PART_INFO) else { break };
        if info_word == ws::MISSING || info_word == ws::REFUSED { break; }
        let info = ws::Info::of(info_word);
        let ssid_len = (info.ssid_len as usize).min(32);
        let mut ssid_buf = [0u8; 35];
        let parts = (ssid_len + 6) / 7;
        for p in 0..parts {
            if let Some(w) = wifi_rpc(ws::NETWORK, idx as u64, ws::PART_SSID + p as u64) {
                ws::word_bytes(w, &mut ssid_buf[p * 7..]);
            }
        }
        let mut entry = candy_gui::WifiScanEntry::EMPTY;
        entry.scan_idx = idx as u16;
        entry.ssid[..ssid_len].copy_from_slice(&ssid_buf[..ssid_len]);
        entry.ssid_len = ssid_len;
        entry.rssi = info.rssi;
        entry.channel = info.channel;
        entry.security = info.security;
        gui.wifi.scanned[gui.wifi.scan_count] = entry;
        gui.wifi.scan_count += 1;
    }

    if gui.wifi.connected && gui.wifi.active_ssid_len == 0 && gui.wifi.known_count > 0 {
        let first = gui.wifi.known[0];
        gui.wifi.set_active_ssid(first.ssid_bytes());
    }
    if gui.wifi.active_ssid_len > 0 {
        let cur = gui.wifi.active_ssid;
        let clen = gui.wifi.active_ssid_len;
        for i in 0..gui.wifi.scan_count {
            if gui.wifi.scanned[i].ssid_bytes() == &cur[..clen] {
                gui.wifi.signal_rssi = gui.wifi.scanned[i].rssi;
                break;
            }
        }
    }
}

/// Attempts to join `ssid` using `sexnet` if it appears in the current scan list.
fn wifi_join_ssid(gui: &mut GuiState, ssid: &[u8], pass: &[u8], m: &mut Model) {
    use sex_pdx::wifi_status as ws;
    let found = (0..gui.wifi.scan_count)
        .find(|&i| gui.wifi.scanned[i].ssid_bytes() == ssid)
        .map(|i| gui.wifi.scanned[i].scan_idx as u64);
    let Some(scan_idx) = found else {
        m.say("Saved to /data/wifi.conf (auto-connects when in range).");
        return;
    };
    if pass.len() >= ws::PASSWORD_MIN {
        let parts = (pass.len() + 6) / 7;
        for p in 0..parts {
            let from = p * 7;
            let word = ws::bytes_word(&pass[from..pass.len().max(from)]);
            let _ = wifi_rpc(ws::PASSWORD, (p as u64) | ((pass.len() as u64) << 8), word);
        }
    }
    let _ = wifi_rpc(ws::JOIN, scan_idx, 0);
    gui.wifi.set_active_ssid(ssid);
    gui.wifi.set_status(b"Connecting...");
    m.say("Saved to /data/wifi.conf and joining Wi-Fi...");
}

fn submit_wifi_join(window: &mut Window, gui: &mut GuiState, m: &mut Model) {
    use sex_pdx::wifi_status as ws;
    let ssid_len = gui.wifi.form_ssid_len;
    let pass_len = gui.wifi.form_pass_len;
    if ssid_len == 0 || ssid_len > 32 {
        m.say("Enter an SSID (1-32 characters) first.");
        return;
    }
    if pass_len > 0 && !(ws::PASSWORD_MIN..=ws::PASSWORD_MAX).contains(&pass_len) {
        m.say("Wi-Fi passphrase must be 8-63 characters (or empty for open).");
        return;
    }
    let ssid = gui.wifi.form_ssid;
    let pass = gui.wifi.form_pass;
    gui.wifi.upsert_known(&ssid[..ssid_len], &pass[..pass_len]);
    let _ = save_wifi_conf(window, gui, m);
    wifi_join_ssid(gui, &ssid[..ssid_len], &pass[..pass_len], m);
    gui.wifi.form_pass = [0; 63];
    gui.wifi.form_pass_len = 0;
    gui.wifi.focus = candy_gui::WifiFieldFocus::None;
}

static mut GUI: GuiState = GuiState::new();
static mut GLYPHS: Glyphs = Glyphs::new();

fn show(window: &mut Window, m: &Model, gui: &mut GuiState) {
    let (w, h) = window.size();
    let px = window.pixels();
    let w = w.clamp(64, 4096);
    let h = h.clamp(64, px.len() / w);
    let g = unsafe { &mut *core::ptr::addr_of_mut!(GLYPHS) };
    candy_gui::render_gui(&mut px[..w * h], w, h, g, gui, m);
    window.show_pixels(w, h);
}

fn persist_setting(m: &mut Model, key_id: u16, value: i32, generation: &mut u64) -> bool {
    let result = settings::set(key_id, value);
    sex_pdx::serial_record!("[settings.set] key={} value={} status={}", key_id, value, result.err().unwrap_or(0));
    let ok = match result {
        Ok(()) => {
            let _ = m.values.set(key_id, value);
            m.say("Saved.");
            true
        }
        Err(e) if e == sex_pdx::config::NOT_SAVED => {
            let _ = m.values.set(key_id, value);
            m.say("Applied live (disk read-only).");
            true
        }
        Err(e) => {
            m.say(settings::message(e));
            false
        }
    };
    *generation = settings::generation().unwrap_or(*generation);
    ok
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
    let gui = unsafe { &mut *core::ptr::addr_of_mut!(GUI) };
    let mut rows = [grants::Row::EMPTY; grants::ROWS_MAX];
    let mut hashing = false;
    sex_pdx::serial_record!("[settings.open] generation={}", generation);
    show(&mut window, &m, gui);
    loop {
        let Some(event) = window.next_within(if hashing { 100 } else { 1000 }) else {
            // About is open and Files has more of the boot image to hash.
            if let Some(info) = m.info_page().filter(|_| hashing) {
                hashing = load_info(&mut window, &mut m, info);
                show(&mut window, &m, gui);
                continue;
            }
            // Someone else changed a setting (a Shell key, say): show it.
            if settings::generation().is_ok_and(|g| g != generation) {
                let (values, g) = load();
                m.values = values;
                generation = g;
                show(&mut window, &m, gui);
            }
            continue;
        };
        match event {
            Event::Close => window.close(),
            Event::Resize { .. } => {
                show(&mut window, &m, gui);
                continue;
            }
            Event::Pointer(p) => {
                let hit = gui.hit_at(p.x as i32, p.y as i32);
                match p.kind {
                    PointerKind::Move => {
                        if gui.hovered != hit {
                            gui.hovered = hit;
                            show(&mut window, &m, gui);
                        }
                        continue;
                    }
                    PointerKind::Up { button: 0 } => {
                        if gui.pressed.is_some() || gui.hovered != hit {
                            gui.pressed = None;
                            gui.hovered = hit;
                            show(&mut window, &m, gui);
                        }
                        continue;
                    }
                    PointerKind::Down { button: 0 } => {
                        gui.hovered = hit;
                        gui.pressed = hit;
                        let Some(act) = hit else { continue };
                        let old_page = m.page;
                        match act {
                            UiAction::SelectView(v) => {
                                gui.view = v;
                                gui.wifi.focus = candy_gui::WifiFieldFocus::None;
                                if let SidebarView::SystemPage(idx) = v {
                                    m.page = idx;
                                    m.row = 0;
                                } else if v == SidebarView::Install && !gui.install.loaded {
                                    refresh_install_parts(&mut window, gui, &mut m);
                                } else if v == SidebarView::Wifi {
                                    if !gui.wifi.loaded {
                                        load_wifi_conf(&mut window, gui);
                                    }
                                    refresh_wifi_live(gui);
                                }
                            }
                            UiAction::SetTheme(val) => {
                                gui.save_undo(&m.values);
                                gui.active_profile = None;
                                persist_setting(&mut m, id::THEME, val, &mut generation);
                            }
                            UiAction::SetAccent(rgb) => {
                                gui.save_undo(&m.values);
                                gui.active_profile = None;
                                persist_setting(&mut m, id::ACCENT, rgb, &mut generation);
                            }
                            UiAction::ToggleFrostedGlass => {
                                gui.save_undo(&m.values);
                                gui.active_profile = None;
                                let next = if m.values.value(id::REDUCE_TRANSPARENCY) == 0 { 1 } else { 0 };
                                persist_setting(&mut m, id::REDUCE_TRANSPARENCY, next, &mut generation);
                            }
                            UiAction::ToggleAnimations => {
                                gui.save_undo(&m.values);
                                gui.active_profile = None;
                                let next = if m.values.value(id::ANIMATIONS) == 0 { 1 } else { 0 };
                                persist_setting(&mut m, id::ANIMATIONS, next, &mut generation);
                            }
                            UiAction::SetCornerRadius(idx) => {
                                gui.save_undo(&m.values);
                                gui.corner_radius_idx = idx;
                                gui.active_profile = None;
                                m.say("Corner radius updated.");
                            }
                            UiAction::SetTabBar(style) => {
                                gui.save_undo(&m.values);
                                gui.tab_bar = style;
                                gui.active_profile = None;
                                m.say("Tab bar style updated.");
                            }
                            UiAction::SetTiling(tiling) => {
                                gui.save_undo(&m.values);
                                gui.tiling = tiling;
                                gui.active_profile = None;
                                m.say("Tiling default updated.");
                            }
                            UiAction::SetKeybinds(prof) => {
                                gui.save_undo(&m.values);
                                gui.keybinds = prof;
                                gui.active_profile = None;
                                m.say("Hotkey profile updated.");
                            }
                            UiAction::ApplyProfile(idx) => {
                                let pairs = gui.apply_profile(idx, &m.values);
                                for (k, v) in pairs {
                                    persist_setting(&mut m, k, v, &mut generation);
                                }
                                m.say("Profile applied live.");
                            }
                            UiAction::Undo => {
                                if let Some(pairs) = gui.take_undo() {
                                    for (k, v) in pairs {
                                        persist_setting(&mut m, k, v, &mut generation);
                                    }
                                    m.say("Reverted previous change.");
                                }
                            }
                            UiAction::ResetDefaults => {
                                let pairs = gui.reset_defaults(&m.values);
                                for (k, v) in pairs {
                                    persist_setting(&mut m, k, v, &mut generation);
                                }
                                m.say("Reset to Candy Lavender defaults.");
                            }
                            UiAction::SelectRow(idx) => {
                                m.row = idx;
                            }
                            UiAction::StepRow(idx, forward) => {
                                m.row = idx;
                                if let Change::Set(k, v) = m.step(forward, false) {
                                    persist_setting(&mut m, k, v, &mut generation);
                                    if m.on_privacy() { load_grants(&mut window, &mut m, &mut rows); }
                                }
                            }
                            UiAction::SelectInstallPart(idx) => {
                                if gui.install.select_part(idx) {
                                    m.say("Install target selected.");
                                } else {
                                    m.say("That partition is protected and cannot be replaced.");
                                }
                            }
                            UiAction::InstallRefresh => {
                                refresh_install_parts(&mut window, gui, &mut m);
                            }
                            UiAction::InstallPlan => {
                                run_install_action(&mut window, gui, &mut m, false);
                            }
                            UiAction::InstallExecute => {
                                run_install_action(&mut window, gui, &mut m, true);
                            }
                            UiAction::CheckSystemUpdate => {
                                let _ = load_info(&mut window, &mut m, "updates");
                                gui.updates.system_checked = true;
                                gui.updates.set_system_status(b"Checked xirtus/sexos A/B slot - run 'update' in Spindle to apply.");
                                sex_pdx::serial_record!("[settings.updates.check] repo={}", candy_gui::SYSTEM_REPO);
                                m.say("Checked xirtus/sexos A/B update status.");
                            }
                            UiAction::RefreshUpdates => {
                                let _ = load_info(&mut window, &mut m, "updates");
                                gui.updates.set_system_status(b"A/B slot status refreshed from Files.");
                                m.say("Update status refreshed.");
                            }
                            UiAction::SyncDotfiles => {
                                gui.updates.dotfiles_synced = true;
                                gui.updates.set_dotfiles_status(b"Synced config with xirtus/sexosdotfiles.");
                                sex_pdx::serial_record!("[settings.dotfiles.sync] repo={} action=sync", candy_gui::DOTFILES_REPO);
                                m.say("Private dotfiles sync queued (xirtus/sexosdotfiles).");
                            }
                            UiAction::PullDotfiles => {
                                gui.updates.dotfiles_synced = true;
                                gui.updates.set_dotfiles_status(b"Pulled latest dotfiles from xirtus/sexosdotfiles.");
                                sex_pdx::serial_record!("[settings.dotfiles.sync] repo={} action=pull", candy_gui::DOTFILES_REPO);
                                m.say("Pulled latest from xirtus/sexosdotfiles.");
                            }
                            UiAction::ToggleDotfilesAutoSync => {
                                gui.updates.dotfiles_auto_sync = !gui.updates.dotfiles_auto_sync;
                                sex_pdx::serial_record!("[settings.dotfiles.autosync] enabled={}", gui.updates.dotfiles_auto_sync as u8);
                                m.say(if gui.updates.dotfiles_auto_sync {
                                    "Dotfiles auto-sync enabled."
                                } else {
                                    "Dotfiles auto-sync disabled."
                                });
                            }
                            UiAction::WifiScan => {
                                let _ = wifi_rpc(sex_pdx::wifi_status::SCAN, 0, 0);
                                refresh_wifi_live(gui);
                                m.say("Wi-Fi scan requested.");
                            }
                            UiAction::WifiRefresh => {
                                load_wifi_conf(&mut window, gui);
                                refresh_wifi_live(gui);
                                m.say("Wi-Fi status and /data/wifi.conf refreshed.");
                            }
                            UiAction::WifiConnectKnown(idx) => {
                                if idx < gui.wifi.known_count {
                                    let entry = gui.wifi.known[idx];
                                    gui.wifi.upsert_known(entry.ssid_bytes(), entry.pass_bytes());
                                    let _ = save_wifi_conf(&mut window, gui, &mut m);
                                    wifi_join_ssid(gui, entry.ssid_bytes(), entry.pass_bytes(), &mut m);
                                }
                            }
                            UiAction::WifiForgetKnown(idx) => {
                                if gui.wifi.forget_known(idx) && save_wifi_conf(&mut window, gui, &mut m) {
                                    m.say("Removed network from /data/wifi.conf.");
                                }
                            }
                            UiAction::WifiSelectScan(idx) => {
                                if gui.wifi.select_scanned(idx) {
                                    m.say("Selected network - enter passphrase if needed, then click Save & Join.");
                                }
                            }
                            UiAction::WifiFocusSsid => {
                                gui.wifi.focus = candy_gui::WifiFieldFocus::Ssid;
                                m.say("Type Wi-Fi SSID (Tab for passphrase, Enter to join).");
                            }
                            UiAction::WifiFocusPass => {
                                gui.wifi.focus = candy_gui::WifiFieldFocus::Password;
                                m.say("Type Wi-Fi passphrase (Enter to Save & Join).");
                            }
                            UiAction::WifiJoinSubmit => {
                                submit_wifi_join(&mut window, gui, &mut m);
                            }
                        }
                        if m.page != old_page && m.on_privacy() { load_grants(&mut window, &mut m, &mut rows); }
                        if m.page != old_page { hashing = m.info_page().is_some_and(|info| load_info(&mut window, &mut m, info)); }
                        show(&mut window, &m, gui);
                        continue;
                    }
                    _ => continue,
                }
            }
            Event::Key(key) => {
                sex_pdx::serial_record!("[settings.key] code={} page={} row={}", key.code, m.page, m.row);
                if gui.view == SidebarView::Wifi && gui.wifi.focus != candy_gui::WifiFieldFocus::None {
                    match key.code {
                        ESC => gui.wifi.focus = candy_gui::WifiFieldFocus::None,
                        TAB => {
                            gui.wifi.focus = match gui.wifi.focus {
                                candy_gui::WifiFieldFocus::Ssid => candy_gui::WifiFieldFocus::Password,
                                _ => candy_gui::WifiFieldFocus::Ssid,
                            };
                        }
                        BACKSPACE => { gui.wifi.backspace(); }
                        ENTER => {
                            if gui.wifi.focus == candy_gui::WifiFieldFocus::Ssid && gui.wifi.form_pass_len == 0 {
                                gui.wifi.focus = candy_gui::WifiFieldFocus::Password;
                            } else {
                                submit_wifi_join(&mut window, gui, &mut m);
                            }
                        }
                        _ => {
                            if let Some(c) = sex_app::typed(key) {
                                gui.wifi.type_char(c);
                            }
                        }
                    }
                    show(&mut window, &m, gui);
                    continue;
                }
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
                if m.page != page {
                    let (pages, _) = model::pages();
                    gui.view = if pages[m.page] == "appearance" {
                        SidebarView::Appearance
                    } else {
                        SidebarView::SystemPage(m.page)
                    };
                }
                if m.page != page && m.on_privacy() { load_grants(&mut window, &mut m, &mut rows); }
                if m.page != page { hashing = m.info_page().is_some_and(|info| load_info(&mut window, &mut m, info)); }
                if let Change::Password(mut typed, n) = change {
                    let result = settings::password(sex_config::secret::SET, &typed[..n]);
                    for b in typed.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) }; }
                    sex_pdx::serial_record!("[settings.password] status={}", match result { Ok(s) => s, Err(e) => e });
                    m.say(match result { Ok(sex_config::secret::OK) => "Lock password set.", Ok(_) | Err(_) => "The password could not be saved." });
                }
                if let Change::Set(key_id, value) = change {
                    if persist_setting(&mut m, key_id, value, &mut generation) && m.on_privacy() {
                        load_grants(&mut window, &mut m, &mut rows);
                    }
                }
                show(&mut window, &m, gui);
            }
            _ => continue,
        }
    }
}
