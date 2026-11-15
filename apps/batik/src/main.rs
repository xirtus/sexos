// SPDX-License-Identifier: GPL-3.0-only
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;

    use batik::Batik;
    use batik::weft;
    use batik::weft::sexos::App;
    use batik::weft::{Response, Strings, Theme};

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[batik.fatal] {}", info);
        sex_pdx::task_finish(1);
    }

    fn report(app: &App, b: &Batik) {
        let s = app.ui.theme().scale;

        sex_pdx::serial_record!(
            "[batik.state] layers={} active={} blend={} mask={} edit_mask={} tool={} sel={} undo={} pixel0={:08x} adj_layers={} text_layers={} fx_count={} fx_enabled={}",
            b.layer_count(),
            b.active_layer_id,
            b.active_blend_name(),
            if b.active_has_mask() { "yes" } else { "no" },
            if b.edit_mask { "yes" } else { "no" },
            b.tool_name(),
            if b.selection.is_active() { "yes" } else { "no" },
            b.undo_count(),
            b.pixel0(),
            b.adj_layer_count(),
            b.text_layer_count(),
            b.active_fx_count(),
            if b.active_fx_enabled() { "yes" } else { "no" }
        );

        let cr = app.ui.rect(b.canvas_id);
        sex_pdx::serial_record!(
            "[batik.canvas] x={} y={} w={} h={}",
            (cr.x * s) as i32,
            (cr.y * s) as i32,
            (cr.w * s) as i32,
            (cr.h * s) as i32
        );

        for (name, id) in &b.controls {
            let c = app.ui.rect(*id).center();
            sex_pdx::serial_record!(
                "[batik.control] name={} x={} y={}",
                name,
                (c.x * s) as i32,
                (c.y * s) as i32
            );
        }

        sex_pdx::serial_record!("[batik.frame] n={}", app.frames);
    }

    fn export_file(app: &mut App, b: &Batik, fmt: &str) {
        let (filename, res): (&[u8], Result<alloc::vec::Vec<u8>, _>) = match fmt {
            "png" => (b"painting.png", b.encode_png()),
            "jpg" => (b"painting.jpg", b.encode_jpeg(90)),
            "webp" => (b"painting.webp", b.encode_webp()),
            "psd" => (b"painting.psd", b.encode_psd()),
            "ora" => (b"painting.ora", b.encode_ora()),
            _ => return,
        };

        match res {
            Ok(bytes) => {
                match sex_app::files::write(&mut app.window, filename, &bytes) {
                    Ok(()) => {
                        sex_pdx::serial_record!(
                            "[batik.exported] fmt={} bytes={}",
                            fmt,
                            bytes.len()
                        );
                        sex_pdx::serial_record!(
                            "[batik.saved] file=/data/batik/{} bytes={}",
                            core::str::from_utf8(filename).unwrap_or("file"),
                            bytes.len()
                        );
                    }
                    Err(e) => {
                        sex_pdx::serial_record!("[batik.export_err] fmt={} write error={}", fmt, e);
                    }
                }
            }
            Err(e) => {
                sex_pdx::serial_record!("[batik.export_err] fmt={} encode error={:?}", fmt, e);
            }
        }
    }

    fn import_file(app: &mut App, b: &mut Batik, fmt: &str) {
        let filename: &[u8] = match fmt {
            "png" => b"painting.png",
            "jpg" => b"painting.jpg",
            "webp" => b"painting.webp",
            "psd" => b"painting.psd",
            "ora" => b"painting.ora",
            _ => return,
        };

        let mut buf = alloc::vec![0u8; 16 * 1024 * 1024];
        match sex_app::files::read(&mut app.window, filename, &mut buf) {
            Ok(n) => {
                let bytes = &buf[..n];
                match b.import_bytes(bytes, &mut app.ui) {
                    Ok(()) => {
                        sex_pdx::serial_record!(
                            "[batik.imported] fmt={} layers={} w={} h={}",
                            fmt,
                            b.doc.layers.len(),
                            b.doc.width,
                            b.doc.height
                        );
                    }
                    Err(e) => {
                        sex_pdx::serial_record!("[batik.import_err] fmt={} decode error={:?}", fmt, e);
                    }
                }
            }
            Err(e) => {
                sex_pdx::serial_record!("[batik.import_err] fmt={} read error={}", fmt, e);
            }
        }
    }

    fn process_event(app: &mut App, batik: &mut Batik, e: sex_app::Event) -> bool {
        let is_resize = matches!(e, sex_app::Event::Resize { .. });
        let weft_e = weft::sexos::event(e);
        let mut changed = batik.handle_event(&mut app.ui, &weft_e);
        let responses = app.ui.handle(weft_e);
        if is_resize {
            changed = true;
        }

        for r in responses {
            if r == Response::Close {
                sex_pdx::task_finish(0);
            }
            if let Response::Clicked(id) = r {
                if id == batik.btn_export_png {
                    export_file(app, batik, "png");
                    changed = true;
                    continue;
                } else if id == batik.btn_export_jpg {
                    export_file(app, batik, "jpg");
                    changed = true;
                    continue;
                } else if id == batik.btn_export_webp {
                    export_file(app, batik, "webp");
                    changed = true;
                    continue;
                } else if id == batik.btn_export_psd {
                    export_file(app, batik, "psd");
                    changed = true;
                    continue;
                } else if id == batik.btn_save_ora {
                    export_file(app, batik, "ora");
                    changed = true;
                    continue;
                } else if id == batik.btn_import_png {
                    import_file(app, batik, "png");
                    changed = true;
                    continue;
                } else if id == batik.btn_import_jpg {
                    import_file(app, batik, "jpg");
                    changed = true;
                    continue;
                } else if id == batik.btn_import_webp {
                    import_file(app, batik, "webp");
                    changed = true;
                    continue;
                } else if id == batik.btn_import_psd {
                    import_file(app, batik, "psd");
                    changed = true;
                    continue;
                }
            }
            if let Response::Command(ref cmd) = r {
                if cmd == "batik.export-png" {
                    export_file(app, batik, "png");
                    changed = true;
                    continue;
                } else if cmd == "batik.export-jpg" {
                    export_file(app, batik, "jpg");
                    changed = true;
                    continue;
                } else if cmd == "batik.export-webp" {
                    export_file(app, batik, "webp");
                    changed = true;
                    continue;
                } else if cmd == "batik.export-psd" {
                    export_file(app, batik, "psd");
                    changed = true;
                    continue;
                } else if cmd == "batik.save-ora" {
                    export_file(app, batik, "ora");
                    changed = true;
                    continue;
                } else if cmd == "batik.import-png" {
                    import_file(app, batik, "png");
                    changed = true;
                    continue;
                } else if cmd == "batik.import-jpg" {
                    import_file(app, batik, "jpg");
                    changed = true;
                    continue;
                } else if cmd == "batik.import-webp" {
                    import_file(app, batik, "webp");
                    changed = true;
                    continue;
                } else if cmd == "batik.import-psd" {
                    import_file(app, batik, "psd");
                    changed = true;
                    continue;
                }
            }
            if batik.on_response(&mut app.ui, &r) {
                changed = true;
            }
        }
        changed
    }

    extern "C" fn main() -> ! {
        let mut app = App::open("Batik", Theme::light(), Strings::new("en"));
        let mut batik = Batik::build(&mut app.ui);

        // Check launch args if any
        let mut args_buf = [0u8; sex_pdx::fs::ARGS_MAX];
        let args = sex_app::files::args(&mut app.window, &mut args_buf);
        if !args.is_empty() {
            if let Ok(arg_str) = core::str::from_utf8(args) {
                let clean = arg_str.trim();
                let path = if clean.starts_with('@') {
                    &clean[1..]
                } else {
                    clean
                };
                if !path.is_empty() {
                    let mut file_buf = alloc::vec![0u8; 16 * 1024 * 1024];
                    if let Ok(n) = sex_app::files::read(&mut app.window, path.as_bytes(), &mut file_buf) {
                        let bytes = &file_buf[..n];
                        let fmt = if bytes.starts_with(b"8BPS") {
                            "psd"
                        } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                            "png"
                        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                            "jpg"
                        } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
                            "webp"
                        } else if bytes.starts_with(b"PK\x03\x04") {
                            "ora"
                        } else {
                            "img"
                        };
                        if batik.import_bytes(bytes, &mut app.ui).is_ok() {
                            sex_pdx::serial_record!(
                                "[batik.imported] fmt={} layers={} w={} h={}",
                                fmt,
                                batik.doc.layers.len(),
                                batik.doc.width,
                                batik.doc.height
                            );
                        }
                    }
                }
            }
        }

        app.present();
        sex_pdx::serial_record!(
            "[batik.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        report(&app, &batik);

        loop {
            let wait = app.ui.tick(weft::sexos::now());
            let e = match wait {
                Some(ms) => app.window.next_within(ms.max(1)),
                None => Some(app.window.next()),
            };
            app.ui.tick(weft::sexos::now());

            let mut changed = false;
            if let Some(e) = e {
                if process_event(&mut app, &mut batik, e) {
                    changed = true;
                }
                for _ in 0..64 {
                    let Some(e) = app.window.poll() else { break };
                    if process_event(&mut app, &mut batik, e) {
                        changed = true;
                    }
                }
            }

            if changed {
                app.present();
                report(&app, &batik);
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
    println!("Batik: Weft image editor & digital painting app for SexOS.");
}
