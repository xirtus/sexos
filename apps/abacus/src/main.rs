// SPDX-License-Identifier: GPL-3.0-only
//! Abacus on a SexOS window. `run abacus`.
//!
//! Serial record: `[abacus.ready] target= task= keys=`, then after `[abacus.layout] w= h=`
//! (again after each resize) one `[abacus.key] n= x= y=` per keypad key (in `abacus::KEYS` order) (window pixels, for gates), `[abacus.result] expr= value=`,
//! `[abacus.error] what=`, `[abacus.memory] value=`, `[abacus.recall] index= value=`,
//! `[abacus.mode] angle=deg|rad`, `[abacus.command] id=`, `[abacus.frame] n=` for the
//! first frame and `[abacus.closed] target= task=` when the window closes.
//!
//! `run abacus tray` also puts an item in the panel (slice S2): `[abacus.tray.ready]
//! target= task= item= rows=`, `[abacus.tray.menu] item= row= label=`, and for its
//! "Ask me a sum" row a notification with a reply field: `[abacus.notify.ask] tag=
//! posted=`, then `[abacus.notify.reply] tag= text= answer= error=`.
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;
    use abacus::{Abacus, Mode, Note};
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
        let mut laid = app.window.size();
        keys(&app, &calc, laid);
        // The first frame is painted before any input; say so now, not after the next event.
        let mut reported = app.frames > 0;
        if reported { sex_pdx::serial_record!("[abacus.frame] n={}", app.frames); }
        // `abacus tray`: an item in the panel whose menu works the calculator and
        // asks for a sum by a notification with an inline reply (slice S2).
        let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
        if sex_app::files::args(&mut app.window, &mut args).split(|b| *b == b' ').any(|w| w == b"tray") {
            let ok = sex_app::tray::add(0, b"=", ACCENT, b"Abacus: the last answer", TRAY_MENU);
            sex_pdx::serial_record!("[abacus.tray.ready] target={} task={} item={} rows={}", target, task, ok as u8, TRAY_MENU.len());
        }
        let mut asked = 0u32;
        loop {
            let out = app.next();
            for t in core::mem::take(&mut app.tray) { tray_event(&mut app, &mut calc, t, &mut asked); }
            // A resize lays the keypad out again: say where the keys went.
            if app.window.size() != laid { laid = app.window.size(); keys(&app, &calc, laid); }
            if !reported && app.frames > 0 { reported = true; sex_pdx::serial_record!("[abacus.frame] n={}", app.frames); }
            for r in out {
                if r == Response::Close { sex_pdx::serial_record!("[abacus.closed] target={} task={}", target, task); sex_pdx::task_finish(0); }
                if let Response::Command(c) = &r { sex_pdx::serial_record!("[abacus.command] id={}", c); }
                if let Response::Command(c) = &r { if c == "abacus.copy" { let t = alloc::string::String::from(calc.result_text(&app.ui)); app.ui.set_clipboard(&t); } }
                match calc.on(&mut app.ui, &r) {
                    Some(Note::Result { expr, value }) => sex_pdx::serial_record!("[abacus.result] expr={} value={}", ascii(&expr), value),
                    Some(Note::Error(what)) => sex_pdx::serial_record!("[abacus.error] what={}", what),
                    Some(Note::Memory(value)) => sex_pdx::serial_record!("[abacus.memory] value={}", value),
                    Some(Note::Recall { index, value }) => sex_pdx::serial_record!("[abacus.recall] index={} value={}", index, value),
                    Some(Note::Angle(deg)) => sex_pdx::serial_record!("[abacus.mode] angle={}", if deg { "deg" } else { "rad" }),
                    None => {}
                }
            }
        }
    }

    /// The tray item's menu.
    const TRAY_MENU: &[&[u8]] = &[b"Show answer", b"Clear", b"Scientific", b"Ask me a sum"];
    const ACCENT: u32 = 0x0035_74E8;

    /// A row chosen from the tray item's menu, or an answer to the sum notification.
    fn tray_event(app: &mut App, calc: &mut Abacus, event: sex_app::tray::Event, asked: &mut u32) {
        use sex_app::tray::Event as T;
        match event {
            T::MenuChosen { item, row } => {
                let label = TRAY_MENU.get(row as usize).map_or("?", |l| core::str::from_utf8(l).unwrap_or("?"));
                sex_pdx::serial_record!("[abacus.tray.menu] item={} row={} label={}", item, row, label);
                match row {
                    0 => {
                        let shown = calc.result_text(&app.ui);
                        let answer = if calc.past.is_empty() { "nothing yet" } else { shown };
                        sex_pdx::notify::post(b"Abacus", answer.as_bytes(), sex_pdx::notify::URGENCY_NORMAL);
                    }
                    1 => calc.clear(&mut app.ui),
                    2 => calc.set_mode(&mut app.ui, Mode::Scientific),
                    3 => {
                        *asked += 1;
                        let posted = sex_app::tray::notify_with_actions(*asked, b"Abacus", b"Reply with a sum to work out",
                            sex_pdx::notify::URGENCY_NORMAL, &[b"Clear"], Some(b"Type a sum"));
                        sex_pdx::serial_record!("[abacus.notify.ask] tag={} posted={}", *asked, posted as u8);
                    }
                    _ => {}
                }
            }
            T::Activated { item } => sex_pdx::serial_record!("[abacus.tray.activated] item={}", item),
            T::NotificationReply { tag, text } => {
                let typed = core::str::from_utf8(text.as_bytes()).unwrap_or("");
                let (answer, error) = match calc.work_out(&mut app.ui, typed) {
                    Note::Result { value, .. } => (value, 0),
                    Note::Error(what) => (what, 1),
                    _ => (alloc::string::String::new(), 1),
                };
                sex_pdx::serial_record!("[abacus.notify.reply] tag={} text={} answer={} error={}", tag, ascii(typed), ascii(&answer), error);
            }
            T::NotificationAction { tag, action } => {
                sex_pdx::serial_record!("[abacus.notify.action] tag={} action={}", tag, action);
                if action == 0 { calc.clear(&mut app.ui); }
            }
            T::NotificationGone { tag, why } => sex_pdx::serial_record!("[abacus.notify.gone] tag={} why={}", tag, why),
        }
    }

    /// `[abacus.layout] w= h=` and then each key's centre in window pixels.
    fn keys(app: &App, calc: &Abacus, (w, h): (usize, usize)) {
        sex_pdx::serial_record!("[abacus.layout] w={} h={}", w, h);
        let s = app.ui.theme().scale;
        for (n, (id, _)) in calc.keys.iter().enumerate() {
            let c = app.ui.rect(*id).center();
            sex_pdx::serial_record!("[abacus.key] n={} x={} y={}", n, (c.x * s) as i32, (c.y * s) as i32);
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
