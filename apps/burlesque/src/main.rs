// SPDX-License-Identifier: GPL-3.0-only
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;

    use burlesque::Burlesque;
    use burlesque::weft;
    use burlesque::weft::sexos::App;
    use burlesque::weft::{Response, Strings, Theme};
    use burlesque_core::Deck;

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[burlesque.fatal] {}", info);
        sex_pdx::task_finish(1);
    }

    fn report(app: &App, b: &Burlesque) {
        let s = app.ui.theme().scale;

        sex_pdx::serial_record!(
            "[burlesque.state] slides={} current={} presenter={}",
            b.deck.slide_count(),
            b.current_slide,
            if b.presenter_mode { 1 } else { 0 }
        );

        for (name, id) in &b.controls {
            let c = app.ui.rect(*id).center();
            sex_pdx::serial_record!(
                "[burlesque.control] name={} x={} y={}",
                name,
                (c.x * s) as i32,
                (c.y * s) as i32
            );
        }

        sex_pdx::serial_record!("[burlesque.frame] n={}", app.frames);
    }

    fn export_pdf_file(app: &mut App, b: &Burlesque) {
        if let Ok(bytes) = b.export_pdf() {
            if sex_app::files::write(&mut app.window, b"deck.pdf", &bytes).is_ok() {
                sex_pdx::serial_record!(
                    "[burlesque.exported] file=/data/burlesque/deck.pdf bytes={} pages={}",
                    bytes.len(),
                    b.deck.slide_count()
                );
            }
        }
    }

    fn process_event(app: &mut App, burlesque: &mut Burlesque, e: sex_app::Event) -> bool {
        let is_resize = matches!(e, sex_app::Event::Resize { .. });
        let weft_e = weft::sexos::event(e);
        let responses = app.ui.handle(weft_e);
        let mut changed = is_resize;

        for r in responses {
            if r == Response::Close {
                sex_pdx::task_finish(0);
            }
            if let Response::Clicked(id) = r {
                if id == burlesque.btn_export_pdf {
                    export_pdf_file(app, burlesque);
                    changed = true;
                    continue;
                }
            }
            if let Response::Command(ref cmd) = r {
                if cmd == "burlesque.export-pdf" {
                    export_pdf_file(app, burlesque);
                    changed = true;
                    continue;
                }
            }
            if burlesque.on_response(&mut app.ui, &r) {
                changed = true;
            }
        }
        changed
    }

    extern "C" fn main() -> ! {
        let mut app = App::open("Burlesque", Theme::light(), Strings::new("en"));
        let mut burlesque = Burlesque::build(&mut app.ui);

        // Check launch args if any (.pptx, .odp, .md)
        let mut args_buf = [0u8; sex_pdx::fs::ARGS_MAX];
        let args = sex_app::files::args(&mut app.window, &mut args_buf);
        if !args.is_empty() {
            if let Ok(arg_str) = core::str::from_utf8(args) {
                let clean = arg_str.trim();
                let mut file_buf = alloc::vec![0u8; 2 * 1024 * 1024];
                if let Ok(n) = sex_app::files::read(&mut app.window, clean.as_bytes(), &mut file_buf) {
                    let data = &file_buf[..n];
                    let loaded = if clean.ends_with(".pptx") {
                        Deck::open_pptx(data).ok()
                    } else if clean.ends_with(".odp") {
                        Deck::open_odp(data).ok()
                    } else if clean.ends_with(".md") {
                        core::str::from_utf8(data).ok().and_then(|s| Deck::from_markdown(s).ok())
                    } else {
                        None
                    };
                    if let Some(deck) = loaded {
                        burlesque.deck = deck;
                        burlesque.current_slide = 0;
                        burlesque.sync_fields_to_current_slide(&mut app.ui);
                        burlesque.refresh_preview(&mut app.ui);
                    }
                }
            }
        }

        app.present();
        sex_pdx::serial_record!(
            "[burlesque.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        report(&app, &burlesque);

        let mut last_tick = weft::sexos::now();
        loop {
            let now = weft::sexos::now();
            let delta = now.saturating_sub(last_tick);
            last_tick = now;

            if burlesque.presenter_mode {
                if let Some(ref mut session) = burlesque.presenter_session {
                    session.tick(delta);
                }
            }

            let wait = app.ui.tick(now);
            let e = match wait {
                Some(ms) => app.window.next_within(ms.max(1).min(500)),
                None => Some(app.window.next()),
            };
            app.ui.tick(weft::sexos::now());

            let mut changed = false;
            if let Some(e) = e {
                if process_event(&mut app, &mut burlesque, e) {
                    changed = true;
                }
                for _ in 0..64 {
                    let Some(e) = app.window.poll() else { break };
                    if process_event(&mut app, &mut burlesque, e) {
                        changed = true;
                    }
                }
            }

            if changed || burlesque.presenter_mode {
                burlesque.refresh_preview(&mut app.ui);
                app.present();
                report(&app, &burlesque);
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        sex_app::with_stack(2 << 20, main)
    }

    #[allow(suspicious_runtime_symbol_definitions)]
    #[no_mangle]
    pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        sex_pdx::mem::copy_forward(dest, src, n);
        dest
    }

    #[allow(suspicious_runtime_symbol_definitions)]
    #[no_mangle]
    pub unsafe extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
        sex_pdx::mem::fill(dest, c as u8, n);
        dest
    }

    #[allow(suspicious_runtime_symbol_definitions)]
    #[no_mangle]
    pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        sex_pdx::mem::move_bytes(dest, src, n);
        dest
    }

    #[allow(suspicious_runtime_symbol_definitions)]
    #[no_mangle]
    pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
        for i in 0..n {
            let diff = *a.add(i) as i32 - *b.add(i) as i32;
            if diff != 0 {
                return diff;
            }
        }
        0
    }
}

#[cfg(not(feature = "sexos"))]
fn main() {
    println!("Burlesque: Weft presentations app for SexOS.");
}
