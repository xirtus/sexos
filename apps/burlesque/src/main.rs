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
            "[burlesque.state] slides={} current={} presenter={} theme={} transition={}",
            b.deck.slide_count(),
            b.current_slide,
            if b.presenter_mode { 1 } else { 0 },
            b.theme_slug(),
            b.current_transition().kind_str(),
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
                    "[burlesque.exported] fmt=pdf file=/data/burlesque/deck.pdf bytes={} pages={}",
                    bytes.len(),
                    b.deck.slide_count()
                );
            }
        }
    }

    fn save_pptx_file(app: &mut App, b: &Burlesque) {
        if let Ok(bytes) = b.export_pptx() {
            if sex_app::files::write(&mut app.window, b"deck.pptx", &bytes).is_ok() {
                sex_pdx::serial_record!(
                    "[burlesque.exported] fmt=pptx file=/data/burlesque/deck.pptx bytes={} pages={}",
                    bytes.len(),
                    b.deck.slide_count()
                );
            }
        }
    }

    fn import_pptx_file(app: &mut App, b: &mut Burlesque) {
        let mut file_buf = alloc::vec![0u8; 4 * 1024 * 1024];
        let read_res = sex_app::files::read(&mut app.window, b"deck.pptx", &mut file_buf);
        let bytes = match read_res {
            Ok(n) => &file_buf[..n],
            Err(_) => {
                if let Ok(exp) = b.export_pptx() {
                    let _ = sex_app::files::write(&mut app.window, b"deck.pptx", &exp);
                    if let Ok(n) = sex_app::files::read(&mut app.window, b"deck.pptx", &mut file_buf) {
                        &file_buf[..n]
                    } else {
                        return;
                    }
                } else {
                    return;
                }
            }
        };
        if let Ok((slides, notes)) = b.import_pptx(bytes, &mut app.ui) {
            sex_pdx::serial_record!(
                "[burlesque.imported] fmt=pptx slides={} notes={}",
                slides,
                notes
            );
        }
    }

    fn save_odp_file(app: &mut App, b: &Burlesque) {
        if let Ok(bytes) = b.export_odp() {
            if sex_app::files::write(&mut app.window, b"deck.odp", &bytes).is_ok() {
                sex_pdx::serial_record!(
                    "[burlesque.exported] fmt=odp file=/data/burlesque/deck.odp bytes={} pages={}",
                    bytes.len(),
                    b.deck.slide_count()
                );
            }
        }
    }

    fn import_odp_file(app: &mut App, b: &mut Burlesque) {
        let mut file_buf = alloc::vec![0u8; 4 * 1024 * 1024];
        let read_res = sex_app::files::read(&mut app.window, b"deck.odp", &mut file_buf);
        let bytes = match read_res {
            Ok(n) => &file_buf[..n],
            Err(_) => {
                if let Ok(exp) = b.export_odp() {
                    let _ = sex_app::files::write(&mut app.window, b"deck.odp", &exp);
                    if let Ok(n) = sex_app::files::read(&mut app.window, b"deck.odp", &mut file_buf) {
                        &file_buf[..n]
                    } else {
                        return;
                    }
                } else {
                    return;
                }
            }
        };
        if let Ok((slides, notes)) = b.import_odp(bytes, &mut app.ui) {
            sex_pdx::serial_record!(
                "[burlesque.imported] fmt=odp slides={} notes={}",
                slides,
                notes
            );
        }
    }

    fn run_transition(app: &mut App, b: &mut Burlesque) {
        let mut transition = b.current_transition();
        if transition == burlesque_core::Transition::None {
            transition = burlesque_core::Transition::Dissolve;
        }
        let frames = 8usize;
        let w = b.preview_w;
        let h = b.preview_h;
        let len = (w * h) as usize;

        let mut from_buf = alloc::vec![0u32; len];
        let mut to_buf = alloc::vec![0u32; len];

        if let Some(slide) = b.deck.get_slide(b.current_slide) {
            burlesque_core::render_slide(slide, &b.deck.theme, b.deck.aspect_ratio, &mut from_buf, w, h);
        }
        let next_idx = if b.current_slide + 1 < b.deck.slide_count() {
            b.current_slide + 1
        } else if b.current_slide > 0 {
            b.current_slide - 1
        } else {
            b.current_slide
        };
        if next_idx != b.current_slide {
            if let Some(next_slide) = b.deck.get_slide(next_idx) {
                burlesque_core::render_slide(next_slide, &b.deck.theme, b.deck.aspect_ratio, &mut to_buf, w, h);
            }
        } else {
            let mut alt_theme = b.deck.theme.clone();
            alt_theme.palette.background = b.deck.theme.palette.primary_accent;
            if let Some(slide) = b.deck.get_slide(b.current_slide) {
                burlesque_core::render_slide(slide, &alt_theme, b.deck.aspect_ratio, &mut to_buf, w, h);
            }
        }

        for f in 1..=frames {
            let t = f as f32 / frames as f32;
            burlesque_core::render_transition_frame(&from_buf, &to_buf, &mut b.preview_buf, w, h, transition, t);
            app.ui.set_image(b.preview_id, w as usize, h as usize, b.preview_buf.clone());
            app.present();
        }

        sex_pdx::serial_record!(
            "[burlesque.transition] kind={} frames={}",
            transition.kind_str(),
            frames
        );
    }

    fn process_event(app: &mut App, burlesque: &mut Burlesque, e: sex_app::Event) -> bool {
        let is_resize = matches!(e, sex_app::Event::Resize { .. });
        let weft_e = weft::sexos::event(e);
        let responses = app.ui.handle(weft_e);
        let mut changed = is_resize;

        let prev_theme = burlesque.deck.theme.name.clone();
        let prev_presenter = burlesque.presenter_mode;

        for r in responses {
            if r == Response::Close {
                sex_pdx::task_finish(0);
            }
            if let Response::Clicked(id) = r {
                if id == burlesque.btn_export_pdf {
                    export_pdf_file(app, burlesque);
                    changed = true;
                    continue;
                } else if id == burlesque.btn_save_pptx {
                    save_pptx_file(app, burlesque);
                    changed = true;
                    continue;
                } else if id == burlesque.btn_import_pptx {
                    import_pptx_file(app, burlesque);
                    changed = true;
                    continue;
                } else if id == burlesque.btn_save_odp {
                    save_odp_file(app, burlesque);
                    changed = true;
                    continue;
                } else if id == burlesque.btn_import_odp {
                    import_odp_file(app, burlesque);
                    changed = true;
                    continue;
                } else if id == burlesque.btn_play_transition {
                    run_transition(app, burlesque);
                    changed = true;
                    continue;
                }
            }
            if let Response::Command(ref cmd) = r {
                if cmd == "burlesque.export-pdf" {
                    export_pdf_file(app, burlesque);
                    changed = true;
                    continue;
                } else if cmd == "burlesque.save-pptx" {
                    save_pptx_file(app, burlesque);
                    changed = true;
                    continue;
                } else if cmd == "burlesque.import-pptx" {
                    import_pptx_file(app, burlesque);
                    changed = true;
                    continue;
                } else if cmd == "burlesque.save-odp" {
                    save_odp_file(app, burlesque);
                    changed = true;
                    continue;
                } else if cmd == "burlesque.import-odp" {
                    import_odp_file(app, burlesque);
                    changed = true;
                    continue;
                } else if cmd == "burlesque.play-transition" {
                    run_transition(app, burlesque);
                    changed = true;
                    continue;
                }
            }
            if burlesque.on_response(&mut app.ui, &r) {
                changed = true;
            }
        }

        if burlesque.deck.theme.name != prev_theme {
            sex_pdx::serial_record!("[burlesque.theme] name={}", burlesque.deck.theme.name);
        }

        if burlesque.presenter_mode != prev_presenter {
            let active = if burlesque.presenter_mode { 1 } else { 0 };
            let notes_len = burlesque.deck.get_slide(burlesque.current_slide).map(|s| s.notes().len()).unwrap_or(0);
            sex_pdx::serial_record!(
                "[burlesque.presenter] active={} slide={} notes_len={}",
                active,
                burlesque.current_slide,
                notes_len
            );
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
                let path = clean.strip_prefix('@').unwrap_or(clean);
                let mut file_buf = alloc::vec![0u8; 4 * 1024 * 1024];
                if let Ok(n) = sex_app::files::read(&mut app.window, path.as_bytes(), &mut file_buf) {
                    let data = &file_buf[..n];
                    let (fmt, loaded) = if path.ends_with(".pptx") {
                        ("pptx", Deck::open_pptx(data).ok())
                    } else if path.ends_with(".odp") {
                        ("odp", Deck::open_odp(data).ok())
                    } else if path.ends_with(".md") {
                        ("md", core::str::from_utf8(data).ok().and_then(|s| Deck::from_markdown(s).ok()))
                    } else {
                        ("", None)
                    };
                    if let Some(deck) = loaded {
                        let notes_count = deck.slides.iter().filter(|s| !s.notes().is_empty()).count();
                        let slides_count = deck.slide_count();
                        burlesque.deck = deck;
                        burlesque.current_slide = 0;
                        burlesque.sync_fields_to_current_slide(&mut app.ui);
                        burlesque.refresh_preview(&mut app.ui);
                        if !fmt.is_empty() {
                            sex_pdx::serial_record!(
                                "[burlesque.imported] fmt={} slides={} notes={}",
                                fmt,
                                slides_count,
                                notes_count
                            );
                        }
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
        sex_pdx::serial_record!("[burlesque.theme] name={}", burlesque.deck.theme.name);
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
