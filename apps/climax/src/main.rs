// SPDX-License-Identifier: GPL-3.0-only
//! Climax benchmark and thermal stress torture app on Weft.

#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;

    use climax::Climax;
    use climax::weft::sexos::App;
    use climax::weft::{Response, Strings, Theme};

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[climax.fatal] {}", info);
        sex_pdx::task_finish(1);
    }

    fn report(app: &App, climax: &Climax) {
        let corruptions = if climax.torture_analysis.corruption_detected { 1 } else { 0 };
        sex_pdx::serial_record!(
            "[climax.state] mode={:?} suite_ran={} step={}/{} throttle={:?} stability={:.1} corruptions={}",
            climax.mode,
            climax.suite_ran,
            climax.torture_runner.steps_completed(),
            20,
            climax.torture_analysis.throttle_state,
            climax.torture_analysis.stability_percent,
            corruptions,
        );

        let scale = app.ui.theme().scale;
        for (name, id) in [
            ("benchmarks", climax.choices[0]),
            ("torture", climax.choices[1]),
            ("compare_linux", climax.choices[2]),
            ("run_suite", climax.run_suite),
            ("start_step", climax.start_step),
            ("run_burst", climax.run_burst),
            ("reset", climax.reset),
            ("reload_doc", climax.reload_doc),
        ] {
            let c = app.ui.rect(id).center();
            sex_pdx::serial_record!(
                "[climax.control] name={} x={} y={}",
                name,
                (c.x * scale) as i32,
                (c.y * scale) as i32
            );
        }
        sex_pdx::serial_record!("[climax.frame] n={}", app.frames);
    }

    fn try_load_benchmark_doc(app: &mut App, climax: &mut Climax, arg_str: &str) {
        let mut buf = alloc::vec![0u8; 1024 * 1024];
        let mut loaded = false;

        // 1. Check if args contains a .md token
        let mut custom_path = None;
        for token in arg_str.split_whitespace() {
            if token.ends_with(".md") {
                let p = token.strip_prefix('@').unwrap_or(token);
                custom_path = Some(p);
                break;
            }
        }

        if let Some(cp) = custom_path {
            if let Ok(n) = sex_app::files::read(&mut app.window, cp.as_bytes(), &mut buf) {
                if let Ok(md_str) = core::str::from_utf8(&buf[..n]) {
                    let source = alloc::format!("disk:{}", cp);
                    if climax.load_linux_doc_markdown(md_str, &source).is_ok() {
                        loaded = true;
                    }
                }
            }
        }

        // 2. Try default disk locations (short lowercase sexfiles name first, then full doc paths)
        if !loaded {
            for path in [
                b"climax-vs-linux.md".as_slice(),
                b"CLIMAX_VS_LINUX_2026_11_14.md".as_slice(),
                b"docs/CLIMAX_VS_LINUX_2026_11_14.md".as_slice(),
            ] {
                if let Ok(n) = sex_app::files::read(&mut app.window, path, &mut buf) {
                    if let Ok(md_str) = core::str::from_utf8(&buf[..n]) {
                        if climax.load_linux_doc_markdown(md_str, "disk:climax-vs-linux.md").is_ok() {
                            loaded = true;
                            break;
                        }
                    }
                }
            }
        }

        // 3. If not found on disk, seed climax-vs-linux.md to app-private NVMe via files::write, then read it back
        if !loaded {
            let embedded = climax::EMBEDDED_LINUX_DOC;
            let _ = sex_app::files::write(&mut app.window, b"climax-vs-linux.md", embedded.as_bytes());
            if let Ok(n) = sex_app::files::read(&mut app.window, b"climax-vs-linux.md", &mut buf) {
                if let Ok(md_str) = core::str::from_utf8(&buf[..n]) {
                    if climax.load_linux_doc_markdown(md_str, "disk:climax-vs-linux.md").is_ok() {
                        loaded = true;
                    }
                }
            }
        }

        // 4. Fallback to embedded doc if disk read failed
        if !loaded {
            let _ = climax.load_linux_doc_markdown(climax::EMBEDDED_LINUX_DOC, "embedded:docs/CLIMAX_VS_LINUX_2026_11_14.md");
        }

        let geomean = climax.linux_doc.as_ref().map(|d| d.geometric_mean_ratio_pct()).unwrap_or(0.0);
        let rows = climax.linux_doc.as_ref().map(|d| d.comparison_rows.len()).unwrap_or(0);
        sex_pdx::serial_record!(
            "[climax.compare] source={} rows={} geomean_pct={:.1}",
            climax.linux_doc_source,
            rows,
            geomean
        );
    }

    extern "C" fn main() -> ! {
        let mut app = App::open("Climax", Theme::light(), Strings::new("en"));
        let mut climax = Climax::build(&mut app.ui);

        // Check command line arguments for flags
        let mut arg_buf = [0u8; sex_pdx::fs::ARGS_MAX];
        let args = sex_app::files::args(&mut app.window, &mut arg_buf);
        let arg_str = core::str::from_utf8(args).unwrap_or("");

        try_load_benchmark_doc(&mut app, &mut climax, arg_str);

        if arg_str.contains("--json") {
            let res = climax.run_bench_suite(sex_pdx::get_ticks);
            sex_pdx::serial_record!("[climax.bench_json] {}", res.json_payload);
            climax.refresh(&mut app.ui);
        }

        app.present();
        sex_pdx::serial_record!(
            "[climax.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        report(&app, &climax);

        let mut last_size = app.ui.size();
        loop {
            let responses = app.next();
            if app.ui.size() != last_size {
                last_size = app.ui.size();
                climax.refresh(&mut app.ui);
                app.present();
                report(&app, &climax);
            }
            for response in responses {
                if response == Response::Close {
                    sex_pdx::task_finish(0);
                }

                let run_suite_clicked = response == Response::Clicked(climax.run_suite)
                    || response == Response::Command(alloc::string::String::from("climax.run_suite"));
                let reload_doc_clicked = response == Response::Clicked(climax.reload_doc)
                    || response == Response::Command(alloc::string::String::from("climax.reload_doc"));

                if reload_doc_clicked {
                    try_load_benchmark_doc(&mut app, &mut climax, arg_str);
                }

                if climax.on(&mut app.ui, &response, sex_pdx::get_ticks) {
                    if run_suite_clicked {
                        if let Some(res) = &climax.suite_result {
                            sex_pdx::serial_record!("[climax.bench_json] {}", res.json_payload);
                        }
                    }
                    app.present();
                    report(&app, &climax);
                }
            }
        }
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

    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        sex_app::with_stack(2 << 20, main)
    }
}

#[cfg(not(feature = "sexos"))]
fn main() {
    println!("Climax: microbenchmark and thermal torture suite on SexOS/Weft.");
    let mut ui = climax::weft::Ui::new(800, 600, climax::weft::Theme::light(), climax::weft::Strings::new("en"));
    let mut climax = climax::Climax::build(&mut ui);
    println!("Running suite on host...");
    let res = climax.run_bench_suite(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
    });
    println!("JSON: {}", res.json_payload);
    if let Some(doc) = &climax.linux_doc {
        println!(
            "Linux comparison loaded: {} rows, geomean advantage: {:.1}%",
            doc.comparison_rows.len(),
            doc.geometric_mean_ratio_pct() - 100.0
        );
    }
}
