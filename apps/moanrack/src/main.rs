//! Developer qualification window for the offline helper boundary. Product
//! controls belong to Moan; this app makes failure containment observable.
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]
mod seed;
mod status;
#[cfg(feature = "sexos")]
mod guest {
    extern crate alloc;
    use alloc::{format, vec::Vec};
    use moan_plugin::{job::Status, supervisor::OfflineJob};
    use sex_app::files::direct as files;
    use weft::{
        sexos::{event, App},
        widget::TextStyle,
        Response, Strings, Theme,
    };
    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[moanrack.fatal] payload=redacted");
        sex_pdx::task_finish(1)
    }
    const SWAP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/swap.clap"));
    const CRASH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/crash.clap"));
    const RESTART: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/restart.clap"));
    const VST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/gain.vst3"));
    const HANG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hang.clap"));
    fn nonce() -> Option<[u8; 16]> {
        let seed = unsafe { core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 18) != 0 };
        let rand = unsafe { core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0 };
        if !seed && !rand {
            return None;
        }
        let mut out = [0; 16];
        for chunk in out.chunks_mut(8) {
            let mut value = 0u64;
            let mut ok = 0u8;
            for _ in 0..128 {
                unsafe {
                    if seed {
                        core::arch::asm!("rdseed {v}","setc {ok}",v=out(reg)value,ok=out(reg_byte)ok,options(nomem,nostack));
                    } else {
                        core::arch::asm!("rdrand {v}","setc {ok}",v=out(reg)value,ok=out(reg_byte)ok,options(nomem,nostack));
                    }
                }
                if ok != 0 {
                    break;
                }
            }
            if ok == 0 {
                return None;
            }
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        Some(out)
    }
    fn write(path: &[u8], bytes: &[u8]) -> Result<(), ()> {
        files::create(path).map_err(|_| ())?;
        files::truncate(path, 0).map_err(|_| ())?;
        let mut at = 0;
        while at < bytes.len() {
            let n = files::write_at(path, at as u64, &bytes[at..]).map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            at += n;
        }
        files::sync().map_err(|_| ())
    }
    fn present_status(app: &mut App, id: weft::widget::Id, painted: &mut u64) {
        app.present();
        if app.frames == *painted {
            return;
        }
        *painted = app.frames;
        if let Some(view) = super::status::snapshot(&app.ui, id, app.window.pixels()) {
            let encoded: alloc::string::String = view
                .text
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let (w, h) = app.ui.size();
            sex_pdx::serial_record!("[moanrack.paint] target={} task={} surface={} frame={} width={} height={} x={} y={} w={} h={} hash={:#018x} text_hex={}",app.window.target,app.window.task,sex_pdx::app_catalog::by_target(app.window.target).expect("status subject").surface,app.frames,w,h,view.x,view.y,view.w,view.h,view.hash,encoded);
        }
    }
    const STALE: &[u8] = b"@/data/moanrack/stale.ticket";
    /// Crash/restart qualification: admit a hanging helper, wait for its
    /// authenticated Started, keep the admission where only the next parent run
    /// can read it, then fault this parent with the helper still live.
    fn crash_parent(app: &mut App) -> ! {
        let fail = |why: &str| -> ! {
            sex_pdx::serial_record!("[moanrack.parent.crash] ok=0 why={}", why);
            sex_app::launch::exit(2)
        };
        let Some(nonce) = nonce() else { fail("entropy") };
        let hex: alloc::string::String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let plugin = format!("@/data/moanrack/{hex}.clap");
        let input = format!("@/data/moanrack/{hex}.in");
        let output = format!("@/data/moanrack/{hex}.out");
        if write(plugin.as_bytes(), HANG).is_err() {
            fail("write")
        }
        let pcm: Vec<[f32; 2]> = (0..1025).map(|n| [0.125, if n % 2 == 0 { 0.5 } else { -0.25 }]).collect();
        let cookie = 0x5afe;
        let Ok(mut job) = OfflineJob::start_with(
            plugin.as_bytes(),
            input.as_bytes(),
            output.as_bytes(),
            nonce,
            cookie,
            &pcm,
            &[],
            moan_plugin::job::Format::Clap,
            sex_pdx::get_ticks(),
        ) else {
            fail("admission")
        };
        let child = loop {
            job.poll(sex_pdx::get_ticks());
            if let Some(child) = job.child_identity() {
                break child;
            }
            if job.retired() {
                fail("retired_before_start")
            }
            let _ = app.window.next_within(20);
        };
        let (ticket, nonce, files_epoch, shell_epoch) = job.admission();
        let mut record = Vec::with_capacity(40);
        record.extend_from_slice(&ticket.to_le_bytes());
        record.extend_from_slice(&cookie.to_le_bytes());
        record.extend_from_slice(&nonce);
        record.extend_from_slice(&files_epoch.to_le_bytes());
        record.extend_from_slice(&shell_epoch.to_le_bytes());
        if write(STALE, &record).is_err() {
            fail("record")
        }
        sex_pdx::serial_record!(
            "[moanrack.parent.crash] ok=1 parent={} task={} ticket={} cookie={} child={} child_task={}",
            app.window.target,
            app.window.task,
            ticket,
            cookie,
            child.row,
            child.task
        );
        core::mem::forget(job);
        unsafe { core::arch::asm!("ud2", options(noreturn)) }
    }
    /// A later parent holding the crashed parent's exact ticket, nonce and
    /// epochs must get nothing: no result, no entry to release, no START.
    fn stale_probe() -> ! {
        let mut record = [0u8; 40];
        if files::read_at(STALE, 0, &mut record).ok() != Some(40) {
            sex_pdx::serial_record!("[moanrack.stale] ok=0 why=record");
            sex_app::launch::exit(2)
        }
        let _ = files::remove(STALE);
        let _ = files::sync();
        let ticket = u32::from_le_bytes(record[0..4].try_into().unwrap());
        let cookie = u32::from_le_bytes(record[4..8].try_into().unwrap());
        let nonce: [u8; 16] = record[8..24].try_into().unwrap();
        let files_epoch = u64::from_le_bytes(record[24..32].try_into().unwrap());
        let shell_epoch = u64::from_le_bytes(record[32..40].try_into().unwrap());
        let current = files::restricted::epoch().unwrap_or(0);
        let result = files::restricted::result(ticket, nonce, files_epoch).is_ok();
        let cancel = match files::restricted::cancel(ticket, nonce, files_epoch) {
            Ok(()) => 0,
            Err(e) => e as u32 + 1,
        };
        let start = match sex_pdx::restricted_job::start_words(ticket, cookie, files_epoch, shell_epoch) {
            Some(w) => sex_pdx::pdx_call(sex_pdx::SLOT_SHELL, sex_pdx::restricted_job::OP_START, w[0], w[1], w[2]).0,
            None => u64::MAX,
        };
        let (mut started, mut refused) = (0u32, 0u32);
        let until = sex_pdx::get_ticks().saturating_add(3000);
        while sex_pdx::get_ticks() < until {
            while let Some(event) = sex_app::launch::poll(cookie) {
                match event {
                    sex_app::launch::Event::Started { .. } => started += 1,
                    sex_app::launch::Event::Refused { .. } => refused += 1,
                    _ => {}
                }
            }
            sex_pdx::sys_yield();
        }
        sex_pdx::serial_record!(
            "[moanrack.stale] ok=1 ticket={} same_files_epoch={} result={} cancel={} start={:#x} started={} refused={}",
            ticket,
            (current == files_epoch) as u8,
            result as u8,
            cancel,
            start,
            started,
            refused
        );
        sex_app::launch::exit(0)
    }
    extern "C" fn main() -> ! {
        let mut app = App::open(
            "Moan offline qualification",
            Theme::dark(),
            Strings::new("en"),
        );
        let mut args = [0; sex_pdx::fs::ARGS_MAX];
        let count = files::args(&mut args);
        if args[..count]
            .split(|b| b.is_ascii_whitespace())
            .any(|word| word == b"--seed-live-mixer")
        {
            let ok = sex_app::files::size(&mut app.window, b"@/live.wav").is_err()
                && write(b"@/live.wav", &super::seed::live_wav()).is_ok();
            sex_pdx::serial_record!("[moanrack.live.seed] ok={} frames=576000 left_hz=400 right_hz=600 developer_only=1",ok as u8);
            sex_app::launch::exit(if ok { 0 } else { 2 })
        }
        if args[..count]
            .split(|b| b.is_ascii_whitespace())
            .any(|word| word == b"--seed-moan-ui")
        {
            // Developer qualification only, with an explicit user @/ folder grant.
            // Refuse collisions: never overwrite the user's existing files.
            let fileset = [
                (b"@/swap.clap".as_slice(), SWAP),
                (b"@/crash.clap".as_slice(), CRASH),
                (b"@/hang.clap".as_slice(), HANG),
                (b"@/gain.vst3".as_slice(), VST),
            ];
            let ok = fileset
                .iter()
                .all(|(path, _)| sex_app::files::size(&mut app.window, path).is_err())
                && sex_app::files::size(&mut app.window, b"@/source.wav").is_err()
                && files::mkdir(b"@/renders").is_ok()
                && fileset
                    .iter()
                    .all(|(path, bytes)| write(path, bytes).is_ok())
                && write(b"@/source.wav", &super::seed::wav()).is_ok();
            sex_pdx::serial_record!(
                "[moanrack.ui.seed] ok={} frames={} fixtures=4 developer_only=1",
                ok as u8,
                super::seed::FRAMES
            );
            sex_app::launch::exit(if ok { 0 } else { 2 })
        }
        let flag = |name: &[u8]| {
            args[..count]
                .split(|b| b.is_ascii_whitespace())
                .any(|word| word == name)
        };
        if flag(b"--crash-parent") {
            crash_parent(&mut app)
        }
        if flag(b"--stale-probe") {
            stale_probe()
        }
        let column = app.ui.column(app.ui.root());
        app.ui
            .text(column, "Offline plugin helper", TextStyle::Title);
        app.ui.paragraph(column,"Source-built upstream CLAP template and VST3 Gain. Separate domain; crash containment only. Ordinary OS plugin binaries and hostile sandboxing are not qualified.");
        let row = app.ui.row(column);
        let buttons = [
            app.ui.button(row, "Render"),
            app.ui.button(row, "Crash"),
            app.ui.button(row, "Timeout"),
            app.ui.button(row, "Cancel"),
            app.ui.button(row, "VST3 gain"),
            app.ui.button(row, "Restart request"),
            app.ui.button(row, "Receipt wait"),
        ];
        let status = app.ui.label(column, "Ready");
        let alive = app.ui.button(column, "Parent still works");
        let mut counter = 0u32;
        let mut job: Option<OfflineJob> = None;
        let mut closing = false;
        let mut selected = 0;
        let mut resume_at: Option<u64> = None;
        let mut size = (0, 0);
        let mut painted = 0;
        present_status(&mut app, status, &mut painted);
        sex_pdx::serial_record!(
            "[moanrack.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        loop {
            if app.ui.size() != size {
                size = app.ui.size();
                present_status(&mut app, status, &mut painted);
                for (i, id) in buttons.iter().chain(core::iter::once(&alive)).enumerate() {
                    let r = app.ui.rect(*id).center();
                    sex_pdx::serial_record!(
                        "[moanrack.control] id={} x={} y={} w={} h={}",
                        i,
                        (r.x * app.ui.theme().scale) as i32,
                        (r.y * app.ui.theme().scale) as i32,
                        size.0,
                        size.1
                    );
                }
            }
            let now = sex_pdx::get_ticks();
            let can_poll = resume_at.is_none_or(|until| now >= until);
            if can_poll && resume_at.take().is_some() {
                sex_pdx::serial_record!("[moanrack.window.waited] receipts_pending=1");
            }
            if let Some(j) = job.as_mut().filter(|_| can_poll) {
                j.poll(now);
                if j.retired() {
                    let st = j.status();
                    let result = j.take_result();
                    let pass = match result {
                        Ok(Some(pcm)) => pcm.iter().enumerate().all(|(i, v)| {
                            *v == if selected == 4 {
                                [0.03125, if i % 2 == 0 { 0.125 } else { -0.0625 }]
                            } else {
                                [if i % 2 == 0 { 0.5 } else { -0.25 }, 0.125]
                            }
                        }),
                        _ => false,
                    };
                    sex_pdx::serial_record!(
                        "[moanrack.result] mode={} cookie={} complete={} processed={} retired=1",
                        selected,
                        j.cookie(),
                        (st == Status::Complete) as u8,
                        pass as u8
                    );
                    app.ui.set_text(
                        status,
                        if pass {
                            if selected == 4 {
                                "VST3 gain 0.25 verified"
                            } else {
                                "CLAP stereo swap verified"
                            }
                        } else {
                            "Child failed or cancelled; parent remains alive"
                        },
                    );
                    job = None;
                }
            }
            if closing && job.is_none() {
                sex_pdx::serial_record!("[moanrack.closed]");
                sex_app::launch::exit(0)
            }
            let mut responses = Vec::new();
            if let Some(e) = app.window.next_within(20) {
                responses.extend(app.ui.handle(event(e)));
            }
            app.ui.tick(now);
            for response in responses {
                if response == Response::Close {
                    closing = true;
                    if let Some(j) = job.as_mut() {
                        j.cancel();
                    }
                }
                if response == Response::Clicked(alive) {
                    counter += 1;
                    app.ui
                        .set_text(status, &format!("Parent action {}", counter));
                    sex_pdx::serial_record!("[moanrack.alive] n={}", counter);
                }
                for (i, id) in buttons.iter().enumerate() {
                    if response != Response::Clicked(*id) {
                        continue;
                    }
                    if i == 3 {
                        if let Some(j) = job.as_mut() {
                            j.cancel();
                        }
                        continue;
                    }
                    if job.is_some() || closing {
                        continue;
                    }
                    let Some(nonce) = nonce() else {
                        app.ui.set_text(status, "Entropy unavailable");
                        continue;
                    };
                    let hex: alloc::string::String =
                        nonce.iter().map(|b| format!("{b:02x}")).collect();
                    let plugin = format!("@/data/moanrack/{hex}.clap");
                    let input = format!("@/data/moanrack/{hex}.in");
                    let output = format!("@/data/moanrack/{hex}.out");
                    let binary = match i {
                        4 => VST,
                        5 => RESTART,
                        6 => SWAP,
                        _ => [SWAP, CRASH, HANG][i],
                    };
                    let pcm: Vec<[f32; 2]> = (0..1025)
                        .map(|n| [0.125, if n % 2 == 0 { 0.5 } else { -0.25 }])
                        .collect();
                    counter += 1;
                    selected = i;
                    match write(plugin.as_bytes(), binary).and_then(|_| {
                        OfflineJob::start_with(
                            plugin.as_bytes(),
                            input.as_bytes(),
                            output.as_bytes(),
                            nonce,
                            counter,
                            &pcm,
                            if i == 4 {
                                &[moan_plugin::job::Parameter { id: 0, value: 0.25 }]
                            } else {
                                &[]
                            },
                            if i == 4 {
                                moan_plugin::job::Format::Vst3
                            } else {
                                moan_plugin::job::Format::Clap
                            },
                            now,
                        )
                        .map_err(|_| ())
                    }) {
                        Ok(mut j) => {
                            if i == 2 {
                                j.shorten_deadline(now.saturating_add(2000));
                            }
                            if i == 6 {
                                resume_at = Some(now.saturating_add(500));
                            }
                            job = Some(j);
                            app.ui.set_text(status, "Offline render running");
                            sex_pdx::serial_record!(
                                "[moanrack.started] mode={} cookie={}",
                                i,
                                counter
                            );
                        }
                        Err(_) => app
                            .ui
                            .set_text(status, "Render folder grant required or launch refused"),
                    }
                }
            }
            present_status(&mut app, status, &mut painted);
        }
    }
    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        sex_app::with_stack(1 << 20, main)
    }
}
#[cfg(not(feature = "sexos"))]
fn main() {
    println!("SexOS offline plugin qualification window");
}
