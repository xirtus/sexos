// SPDX-License-Identifier: GPL-3.0-only
//! Cabaret on a SexOS window (`#[cfg(feature = "sexos")]`) or headless CLI
//! (`#[cfg(not(feature = "sexos"))]`).
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;
    use cabaret::{sexos_runtime::SexosBroadcastRuntime, Cabaret};
    use sex_pdx::app_key::DESKTOP_KEYS;
    use sex_pdx::desktop_recording::OP_STUDIO_WINDOW_FLAGS;
    use sex_pdx::{pdx_call_checked, SLOT_DISPLAY};
    use alloc::vec::Vec;
    use weft::sexos::App;
    use weft::{Response, Strings, Theme};

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[cabaret.fatal] {}", info);
        sex_pdx::task_finish(1);
    }

    extern "C" fn main() -> ! {
        let mut app = App::open("Cabaret", Theme::dark(), Strings::new("en"));
        let _ = app.window.want_keys(DESKTOP_KEYS);

        let mut cabaret = Cabaret::build(&mut app.ui);
        let (target, task) = (app.window.target, app.window.task);
        let mut runtime = SexosBroadcastRuntime::initialize(target, &cabaret.studio);

        if let Some(flags_req) = runtime.window_capture_flags {
            let args = flags_req.args();
            let _ = pdx_call_checked(SLOT_DISPLAY, OP_STUDIO_WINDOW_FLAGS, args[0], args[1], args[2]);
        }

        app.present();
        sex_pdx::serial_record!(
            "[cabaret.ready] target={} task={} studios={} vcam=\"{}\" capture_policy=HIDE_FROM_CAPTURE",
            target,
            task,
            cabaret.studio.collection.studios.len(),
            runtime.virtual_cam_ring.device_name()
        );

        // While live, frames advance on a ~30 fps tick even when nobody touches the
        // window; otherwise the window waits for input like any other.
        const FRAME_MS: u64 = 33;
        let mut last_frame = weft::sexos::now();
        loop {
            let out = if cabaret.studio.wants_frames() {
                app.present();
                app.ui.tick(weft::sexos::now());
                let mut out = Vec::new();
                if let Some(e) = app.window.next_within(FRAME_MS) {
                    if let Some(ev) = weft::sexos::event(e) { out.extend(app.ui.handle(ev)); }
                }
                for _ in 0..64 {
                    let Some(e) = app.window.poll() else { break };
                    if let Some(ev) = weft::sexos::event(e) { out.extend(app.ui.handle(ev)); }
                }
                let now = weft::sexos::now();
                let dt = now.saturating_sub(last_frame);
                if dt >= FRAME_MS {
                    cabaret.studio.step_frame(dt.min(1000) as u32);
                    cabaret.refresh_all(&mut app.ui);
                    last_frame = now;
                }
                out
            } else {
                last_frame = weft::sexos::now();
                app.next()
            };
            for r in out {
                if r == Response::Close {
                    sex_pdx::serial_record!("[cabaret.closed] target={} task={}", target, task);
                    sex_pdx::task_finish(0);
                }
                cabaret.on(&mut app.ui, &r);
            }
            runtime.sync_and_publish(&mut cabaret.studio);
        }
    }

    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        sex_app::with_stack(1 << 20, main)
    }
}

#[cfg(not(feature = "sexos"))]
fn main() {
    use cabaret::control::run_cli_args;
    use cabaret::studio::CabaretStudio;

    let mut studio = CabaretStudio::new_seeded();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run_cli_args(&mut studio, &arg_refs);
    if !out.stdout.is_empty() {
        print!("{}", out.stdout);
        if !out.stdout.ends_with('\n') {
            println!();
        }
    }
    if !out.stderr.is_empty() {
        eprintln!("{}", out.stderr);
    }
    if out.exit_code != 0 {
        std::process::exit(out.exit_code);
    }
}
