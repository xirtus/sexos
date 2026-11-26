// SPDX-License-Identifier: GPL-3.0-only
//! Developer-only public SDK processing and recovery. No hostile isolation claim.
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]
#[cfg(not(feature = "sexos"))]
fn main() {}
#[cfg(feature = "sexos")]
mod guest {
    extern crate alloc;
    use alloc::{format, vec::Vec};
    use moan_plugin::{
        job::{Format, Parameter, Status},
        supervisor::OfflineJob,
    };
    use sex_app::files::direct as files;
    use weft::{
        sexos::{event, App},
        Response, Strings, Theme,
    };
    const ROOT: &[u8] = b"@/data/moanrecover";
    const SWAP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/swap.clap"));
    const CRASH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/crash.clap"));
    const HANG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hang.clap"));
    const VST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/gain.vst3"));
    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[moanrecover.failed] detail={}", info);
        sex_app::launch::exit(1)
    }
    fn write(path: &[u8], data: &[u8]) {
        files::create(path).expect("exclusive fixture");
        let mut at = 0;
        while at < data.len() {
            let n = files::write_at(path, at as u64, &data[at..]).expect("fixture write");
            assert!(n > 0 && n <= data.len() - at);
            at += n;
        }
        files::sync().expect("fixture sync");
    }
    fn start(mode: u8, cookie: u32) -> Result<OfflineJob, moan_plugin::supervisor::Error> {
        let pcm: Vec<_> = (0..moan_recovery::FRAMES)
            .map(moan_recovery::input)
            .collect();
        let plugin = format!(
            "{}/{}",
            core::str::from_utf8(ROOT).unwrap(),
            match mode {
                1 => "crash.clap",
                2 => "hang.clap",
                3 => "gain.vst3",
                _ => "swap.clap",
            }
        );
        let input = format!("{}/input-{cookie}.bin", core::str::from_utf8(ROOT).unwrap());
        let output = format!(
            "{}/output-{cookie}.bin",
            core::str::from_utf8(ROOT).unwrap()
        );
        let parameters = if mode == 3 {
            &[Parameter { id: 0, value: 0.25 }][..]
        } else {
            &[]
        };
        OfflineJob::start_restricted_with(
            plugin.as_bytes(),
            input.as_bytes(),
            output.as_bytes(),
            moan_plugin::nonce::fresh().expect("hardware nonce"),
            cookie,
            &pcm,
            parameters,
            if mode == 3 {
                Format::Vst3
            } else {
                Format::Clap
            },
            sex_pdx::get_ticks(),
        )
    }
    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        let mut app = App::open(
            "Moan recovery qualification",
            Theme::dark(),
            Strings::new("en"),
        );
        for (name, bytes) in [
            ("swap.clap", SWAP),
            ("crash.clap", CRASH),
            ("hang.clap", HANG),
            ("gain.vst3", VST),
        ] {
            let path = format!("{}/{}", core::str::from_utf8(ROOT).unwrap(), name);
            write(path.as_bytes(), bytes);
        }
        let parent = app.ui.column(app.ui.root());
        let buttons = [
            "CLAP swap",
            "Own crash",
            "Wait for restart",
            "VST3 gain",
            "Two jobs",
            "Third job",
            "Cancel jobs",
            "Parent alive",
            "Close",
        ]
        .map(|text| app.ui.button(parent, text));
        let label = app.ui.label(parent, "Public SDK recovery qualification");
        let mut jobs: [Option<(u8, OfflineJob, Option<bool>)>; 2] = [None, None];
        let mut last_status: [Option<Status>; 2] = [None, None];
        let mut cookie = 0x7000u32;
        let mut alive = 0;
        let mut size = (0, 0);
        let mut closing = false;
        app.present();
        sex_pdx::serial_record!(
            "[moanrecover.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        loop {
            if app.ui.size() != size {
                size = app.ui.size();
                app.present();
                for (i, id) in buttons.iter().enumerate() {
                    let r = app.ui.rect(*id).center();
                    sex_pdx::serial_record!(
                        "[moanrecover.control] id={} x={} y={} w={} h={}",
                        i,
                        (r.x * app.ui.theme().scale) as i32,
                        (r.y * app.ui.theme().scale) as i32,
                        size.0,
                        size.1
                    );
                }
            }
            for (i, slot) in jobs.iter_mut().enumerate() {
                if let Some((mode, j, verified)) = slot {
                    j.poll(sex_pdx::get_ticks());
                    if last_status[i] != Some(j.status()) {
                        last_status[i] = Some(j.status());
                        sex_pdx::serial_record!(
                            "[moanrecover.status] mode={} cookie={} status={:?}",
                            mode,
                            j.cookie(),
                            j.status()
                        );
                    }
                    if j.retired() && verified.is_none() {
                        if let Some(identity) = j.child_identity() {
                            sex_pdx::serial_record!(
                                "[moanrecover.identity] cookie={} pd={} task={}",
                                j.cookie(),
                                identity.row,
                                identity.task
                            );
                        }
                        if j.status() == Status::Complete {
                            let pcm = j
                                .take_result()
                                .expect("bounded result")
                                .expect("complete PCM");
                            assert!(
                                moan_recovery::verifies(&pcm, *mode == 3),
                                "every output sample"
                            );
                            *verified = Some(true);
                        } else {
                            *verified = Some(false);
                        }
                    }
                    // Acceptance schedules asynchronous release. Retain the job
                    // and continue bounded polling until cleanup is acknowledged.
                    if let Some(processed) = *verified {
                        if j.retired() {
                            assert!(!processed || j.result_consumed());
                            sex_pdx::serial_record!("[moanrecover.result] mode={} cookie={} complete={} processed={} retired=1 consumed={}",mode,j.cookie(),(j.status()==Status::Complete)as u8,processed as u8,j.result_consumed()as u8);
                            app.ui.set_text(
                                label,
                                if processed {
                                    "Effect verified; parent remains alive"
                                } else {
                                    "Child ended; parent remains alive"
                                },
                            );
                            *slot = None;
                        }
                    }
                }
            }
            if closing && jobs.iter().all(Option::is_none) {
                sex_pdx::serial_record!("[moanrecover.closed]");
                sex_app::launch::exit(0)
            }
            if let Some(e) = app.window.next_within(20) {
                for response in app.ui.handle(event(e)) {
                    let which = buttons
                        .iter()
                        .position(|id| response == Response::Clicked(*id));
                    if response == Response::Close || which == Some(8) {
                        closing = true;
                        for (_, j, _) in jobs.iter_mut().flatten() {
                            j.cancel();
                        }
                    } else if which == Some(6) {
                        for (_, j, _) in jobs.iter_mut().flatten() {
                            j.cancel();
                        }
                        sex_pdx::serial_record!("[moanrecover.cancelled] requested=1");
                    } else if which == Some(7) {
                        alive += 1;
                        sex_pdx::serial_record!("[moanrecover.alive] n={}", alive);
                    } else if let Some(which) = which {
                        let repeats = if which == 4 { 2 } else { 1 };
                        let mode = if which >= 4 { 2 } else { which as u8 };
                        for _ in 0..repeats {
                            cookie = cookie.checked_add(1).expect("cookies");
                            let result = start(mode, cookie);
                            match result {
                                Ok(j) => {
                                    let index = jobs
                                        .iter()
                                        .position(Option::is_none)
                                        .expect("only2admitted");
                                    last_status[index] = None;
                                    jobs[index] = Some((mode, j, None));
                                    sex_pdx::serial_record!(
                                        "[moanrecover.started] mode={} cookie={}",
                                        mode,
                                        cookie
                                    );
                                }
                                Err(error) => {
                                    sex_pdx::serial_record!(
                                        "[moanrecover.refused] mode={} cookie={} reason={:?}",
                                        mode,
                                        cookie,
                                        error
                                    );
                                }
                            }
                        }
                    }
                }
            }
            app.ui.tick(sex_pdx::get_ticks());
            app.present();
        }
    }
}
