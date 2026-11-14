// SPDX-License-Identifier: GPL-3.0-only
//! Abacus on a SexOS window. `run abacus`.
//!
//! Serial record: `[abacus.ready] target= task= keys=`, one `[abacus.key] n= x= y=`
//! per keypad key (in `abacus::KEYS` order) (window pixels, for gates), `[abacus.result] expr= value=`,
//! `[abacus.error] what=`, `[abacus.command] id=` and `[abacus.frame] n=` for the first
//! frame.
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;
    use abacus::{Abacus, Note};
    use weft::sexos::App;
    use weft::{Response, Strings, Theme};

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[abacus.fatal] {}", info);
        sex_pdx::task_finish(1);
    }

    extern "C" fn main() -> ! {
        let mut app = App::open("Abacus", Theme::light(), Strings::new("en"));
        let calc = Abacus::build(&mut app.ui);
        let mut calc = calc;
        app.present();
        let (target, task) = (app.window.target, app.window.task);
        sex_pdx::serial_record!("[abacus.ready] target={} task={} keys={}", target, task, calc.keys.len());
        let s = app.ui.theme().scale;
        for (n, (id, _)) in calc.keys.iter().enumerate() {
            let c = app.ui.rect(*id).center();
            sex_pdx::serial_record!("[abacus.key] n={} x={} y={}", n, (c.x * s) as i32, (c.y * s) as i32);
        }
        let mut reported = false;
        loop {
            let out = app.next();
            if !reported && app.frames > 0 { reported = true; sex_pdx::serial_record!("[abacus.frame] n={}", app.frames); }
            for r in out {
                if r == Response::Close { sex_pdx::task_finish(0); }
                if let Response::Command(c) = &r { sex_pdx::serial_record!("[abacus.command] id={}", c); }
                if let Response::Command(c) = &r { if c == "abacus.copy" { let t = alloc::string::String::from(calc.result_text(&app.ui)); app.ui.set_clipboard(&t); } }
                match calc.on(&mut app.ui, &r) {
                    Some(Note::Result { expr, value }) => sex_pdx::serial_record!("[abacus.result] expr={} value={}", ascii(&expr), value),
                    Some(Note::Error(what)) => sex_pdx::serial_record!("[abacus.error] what={}", what),
                    None => {}
                }
            }
        }
    }

    /// The record stays ASCII: × ÷ − as * / -.
    fn ascii(s: &str) -> alloc::string::String { s.chars().map(|c| match c { '×' => '*', '÷' => '/', '−' => '-', 'π' => 'p', '√' => 'r', c if c.is_ascii() => c, _ => '?' }).collect() }

    #[no_mangle]
    pub extern "C" fn _start() -> ! { sex_app::with_stack(1 << 20, main) }
}

/// On the host: work out each argument and print it.
#[cfg(not(feature = "sexos"))]
fn main() {
    for a in std::env::args().skip(1) {
        match abacus::calc::eval(&a, 0.0, abacus::calc::Angle::Degrees) {
            Ok(v) => println!("{a} = {}", abacus::calc::format(v)),
            Err(e) => println!("{a}: {e:?}"),
        }
    }
}
