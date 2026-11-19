// SPDX-License-Identifier: GPL-3.0-only
//! Original developer fixture. Expected credentials are fictional test data,
//! never printed. Production clients use the same sex_app::autofill::Client API.
#![no_std]
#![no_main]
extern crate alloc;
use sex_app::autofill::Client;
use weft::{sexos::App, Layout, Response, Strings, Theme};
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[chastity.probe.fatal]");
    sex_pdx::task_finish(1)
}
extern "C" fn main() -> ! {
    let mut app = App::open("Chastity probe", Theme::light(), Strings::new("en"));
    let root = app.ui.root();
    app.ui
        .set_layout(root, Layout::column().padding(10.0).gap(8.0));
    app.ui.label(
        root,
        "Autofill transport fixture — fictional credentials only",
    );
    let origin = app.ui.field(root, "https://example.org", "Exact origin");
    let request = app.ui.button(root, "Request login");
    let forged = app.ui.button(root, "Try provider impersonation");
    let status = app.ui.paragraph(
        root,
        "Ctrl+R requests a login; Ctrl+I tests impersonation. Chastity must ask for explicit consent.",
    );
    let mut client: Option<Client> = None;
    app.present();
    sex_pdx::serial_record!(
        "[chastity.probe.ready] target={} task={}",
        app.window.target,
        app.window.task
    );
    for (name, id) in [("request", request), ("forge", forged)] {
        let r = app.ui.rect(id).center();
        sex_pdx::serial_record!(
            "[chastity.probe.control] name={} x={} y={}",
            name,
            r.x as i32,
            r.y as i32
        );
    }
    loop {
        let now = sex_pdx::get_ticks();
        if let Some(c) = &mut client {
            if let Some(result) = c.poll(now) {
                match result {
                    Ok(credentials) => {
                        let expected =
                            credentials.with(|u, p| u == "alice" && p == "autofilltestsecret");
                        app.ui.set_text(
                            status,
                            if expected {
                                "Received expected fictional credentials after consent."
                            } else {
                                "Unexpected fixture credentials."
                            },
                        );
                        sex_pdx::serial_record!(
                            "[chastity.probe.received] expected={}",
                            expected as u8
                        );
                    }
                    Err(error) => {
                        app.ui
                            .set_text(status, "Request refused or expired. No credentials used.");
                        sex_pdx::serial_record!("[chastity.probe.refused] reason={:?}", error);
                    }
                }
                client = None;
            }
        }
        while let Some(mut m) = sex_app::mailbox::take(|m| {
            m.caller_pd == 3
                && m.caller_task == 3
                && m.type_id == sex_pdx::autofill::STATUS
                && m.arg0 == 0
        }) {
            let denied = m.arg2 == sex_pdx::autofill::Status::Denied as u64;
            sex_pdx::serial_record!("[chastity.probe.impersonation] denied={}", denied as u8);
            unsafe {
                for v in [&mut m.arg0, &mut m.arg1, &mut m.arg2] {
                    core::ptr::write_volatile(v, 0);
                }
            }
        }
        app.present();
        let Some(input) = app
            .window
            .next_within(if client.is_some() { 1 } else { 100 })
        else {
            continue;
        };
        let responses = match &input {
            sex_app::Event::Key(k) if k.control && k.code == 0x13 => {
                alloc::vec![Response::Clicked(request)]
            }
            sex_app::Event::Key(k) if k.control && k.code == 0x17 => {
                alloc::vec![Response::Clicked(forged)]
            }
            _ => weft::sexos::event(input).map_or_else(alloc::vec::Vec::new, |e| app.ui.handle(e)),
        };
        for r in responses {
            match r {
                Response::Close => {
                    if let Some(c) = &mut client {
                        c.cancel();
                    }
                    sex_pdx::task_finish(0)
                }
                Response::Clicked(id) if id == request => {
                    if let Some(c) = &mut client {
                        c.cancel();
                        continue;
                    }
                    match Client::request(&app.window, app.ui.text_of(origin), now) {
                        Ok(c) => {
                            client = Some(c);
                            app.ui
                                .set_text(status, "Waiting for Chastity's visible consent.");
                            sex_pdx::serial_record!("[chastity.probe.requested]");
                        }
                        Err(_) => app.ui.set_text(status, "Invalid origin."),
                    }
                }
                Response::Clicked(id) if id == forged => {
                    let _ = sex_pdx::pdx_call(
                        sex_pdx::SLOT_SHELL,
                        sex_pdx::autofill::REGISTER,
                        777,
                        0,
                        0,
                    );
                }
                _ => {}
            }
        }
    }
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_app::with_stack(1 << 20, main)
}
