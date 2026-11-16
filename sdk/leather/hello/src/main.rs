// SPDX-License-Identifier: GPL-3.0-only
//! A complete Leather hello-app template. All widgets and events belong to Weft.
#![no_std]
#![no_main]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[leather-greeting.fatal] {}", info);
    sex_pdx::task_finish(1)
}
extern "C" fn main() -> ! {
    use weft::{sexos::App, Layout, Response, Strings, Theme};
    let mut app = App::open("Leather Greeting", Theme::light(), Strings::new("en"));
    let root = app.ui.root();
    app.ui
        .set_layout(root, Layout::column().padding(12.0).gap(8.0));
    let label = app.ui.label(root, "Hello, SexOS!");
    let greet = app.ui.button(root, "Greet");
    let mut again = false;
    app.present();
    sex_pdx::serial_record!(
        "[leather-greeting.ready] target={} task={}",
        app.window.target,
        app.window.task
    );
    loop {
        for response in app.next() {
            match response {
                Response::Close => sex_pdx::task_finish(0),
                Response::Clicked(id) if id == greet => {
                    again = !again;
                    app.ui.set_text(
                        label,
                        if again {
                            "Hello again, SexOS!"
                        } else {
                            "Hello, SexOS!"
                        },
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
