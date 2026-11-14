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
            "[batik.state] layers={} tool={} undo={} pixel0={:08x}",
            b.layer_count(),
            b.tool_name(),
            b.undo_count(),
            b.pixel0()
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

    fn save_document(app: &mut App, b: &Batik, is_png: bool) {
        if is_png {
            match b.encode_png() {
                Ok(bytes) => {
                    match sex_app::files::write(&mut app.window, b"painting.png", &bytes) {
                        Ok(()) => {
                            sex_pdx::serial_record!(
                                "[batik.saved] file=/data/batik/painting.png bytes={}",
                                bytes.len()
                            );
                        }
                        Err(e) => {
                            sex_pdx::serial_record!("[batik.save_err] write error={}", e);
                        }
                    }
                }
                Err(e) => {
                    sex_pdx::serial_record!("[batik.save_err] png encode error={:?}", e);
                }
            }
        } else if let Ok(bytes) = b.encode_ora() {
            if sex_app::files::write(&mut app.window, b"painting.ora", &bytes).is_ok() {
                sex_pdx::serial_record!(
                    "[batik.saved] file=/data/batik/painting.ora bytes={}",
                    bytes.len()
                );
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
                if id == batik.btn_save_png {
                    save_document(app, batik, true);
                    changed = true;
                    continue;
                } else if id == batik.btn_save_ora {
                    save_document(app, batik, false);
                    changed = true;
                    continue;
                }
            }
            if let Response::Command(ref cmd) = r {
                if cmd == "batik.save-png" {
                    save_document(app, batik, true);
                    changed = true;
                    continue;
                } else if cmd == "batik.save-ora" {
                    save_document(app, batik, false);
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
                if clean.ends_with(".ora") {
                    let mut file_buf = alloc::vec![0u8; 1024 * 1024];
                    if let Ok(n) = sex_app::files::read(&mut app.window, clean.as_bytes(), &mut file_buf) {
                        let limits = sex_image::Limits::PHOTO;
                        if let Ok(doc) = batik_core::read_ora(&file_buf[..n], &limits) {
                            batik.doc = doc;
                            batik.canvas_w = batik.doc.width;
                            batik.canvas_h = batik.doc.height;
                            batik.refresh_canvas(&mut app.ui);
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
