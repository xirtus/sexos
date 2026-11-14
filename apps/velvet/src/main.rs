// SPDX-License-Identifier: GPL-3.0-only
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]
#[cfg(feature = "sexos")]
mod program {
    use velvet::{Game, Velvet};
    use weft::{sexos::App, Response, Strings, Theme};
    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[velvet.fatal] {}", info);
        sex_pdx::task_finish(1);
    }
    fn report(app: &App, v: &Velvet) {
        let state = match v.game {
            Game::Snags => v.snags.state,
            Game::Mingle => v.mingle.state,
            Game::TwinFlames => v.twins.state,
        };
        sex_pdx::serial_record!("[velvet.state] game={:?} state={:?} opened={} flags={} moves={} score={} pairs={} turns={}",v.game,state,v.snags.open.iter().filter(|x| **x).count(),v.snags.flags.iter().filter(|x| **x).count(),v.mingle.moves,v.mingle.score,v.twins.matched.iter().filter(|x| **x).count()/2,v.twins.turns);
        let scale = app.ui.theme().scale;
        for (n, id) in v.cells.iter().enumerate() {
            let c = app.ui.rect(*id).center();
            sex_pdx::serial_record!(
                "[velvet.cell] n={} x={} y={}",
                n,
                (c.x * scale) as i32,
                (c.y * scale) as i32
            );
        }
        for (name, id) in [
            ("snags", v.choices[0]),
            ("mingle", v.choices[1]),
            ("twins", v.choices[2]),
            ("new", v.new_game),
            ("flag", v.flag_mode),
        ] {
            let c = app.ui.rect(id).center();
            sex_pdx::serial_record!(
                "[velvet.control] name={} x={} y={}",
                name,
                (c.x * scale) as i32,
                (c.y * scale) as i32
            );
        }
        sex_pdx::serial_record!("[velvet.frame] n={}", app.frames);
    }
    extern "C" fn main() -> ! {
        let mut app = App::open("Velvet", Theme::light(), Strings::new("en"));
        let mut games = Velvet::build(&mut app.ui, sex_pdx::get_ticks());
        app.present();
        sex_pdx::serial_record!(
            "[velvet.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        report(&app, &games);
        loop {
            for response in app.next() {
                if response == Response::Close {
                    sex_pdx::task_finish(0);
                }
                if games.on(&mut app.ui, &response) {
                    app.present();
                    report(&app, &games);
                }
            }
        }
    }
    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        sex_app::with_stack(1 << 20, main)
    }
}
#[cfg(not(feature = "sexos"))]
fn main() {
    println!("Velvet: run velvet in SexOS for Snags, Mingle and Twin Flames.");
}
