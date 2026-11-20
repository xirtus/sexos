// SPDX-License-Identifier: GPL-3.0-only
//! Filigree on a SexOS window (`#[cfg(feature = "sexos")]`) or host CLI (`#[cfg(not(feature = "sexos"))]`).
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]

#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;
    use filigree::Filigree;
    use weft::sexos::App;
    use weft::{Response, Strings, Theme};

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[filigree.fatal] {}", info);
        sex_pdx::task_finish(1);
    }

    extern "C" fn main() -> ! {
        let mut app = App::open("Filigree", Theme::light(), Strings::new("en"));
        let mut filigree = Filigree::build(&mut app.ui);
        app.present();
        let (target, task) = (app.window.target, app.window.task);
        sex_pdx::serial_record!(
            "[filigree.ready] target={} task={} families={} faces={}",
            target,
            task,
            filigree.catalog.families().len(),
            filigree.catalog.entries.len()
        );

        loop {
            let out = app.next();
            for r in out {
                if r == Response::Close {
                    sex_pdx::serial_record!("[filigree.closed] target={} task={}", target, task);
                    sex_pdx::task_finish(0);
                }
                filigree.on(&mut app.ui, &r);
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
    use filigree::seeded_catalog;
    use filigree_core::cli::FiligreeCli;

    let (mut catalog, blobs) = seeded_catalog();
    let mut cli = FiligreeCli::new(&mut catalog);
    for (path, bytes) in blobs {
        cli = cli.with_blob(path, bytes);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = cli.run(&arg_refs, None);
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
