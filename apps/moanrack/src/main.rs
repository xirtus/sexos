//! Developer qualification window for the offline helper boundary. Product
//! controls belong to Moan; this app makes failure containment observable.
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]
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
    extern "C" fn main() -> ! {
        let mut app = App::open(
            "Moan offline qualification",
            Theme::dark(),
            Strings::new("en"),
        );
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
        ];
        let status = app.ui.label(column, "Ready");
        let alive = app.ui.button(column, "Parent still works");
        let mut counter = 0u32;
        let mut job: Option<OfflineJob> = None;
        let mut closing = false;
        let mut selected = 0;
        let mut size = (0, 0);
        app.present();
        sex_pdx::serial_record!(
            "[moanrack.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        loop {
            if app.ui.size() != size {
                size = app.ui.size();
                app.present();
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
            if let Some(j) = job.as_mut() {
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
                        "[moanrack.result] mode={} complete={} processed={} retired=1",
                        selected,
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
                    let binary = if i == 4 { VST } else { [SWAP, CRASH, HANG][i] };
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
            app.present();
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
