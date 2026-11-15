// SPDX-License-Identifier: GPL-3.0-only
//! Ribbon Seal: developer-only real launch ownership/argument/grant qualification.
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]
#[cfg(not(feature = "sexos"))]
fn main() {}
#[cfg(feature = "sexos")]
mod guest {
    extern crate alloc;
    use ribbon_seal::{Record, CHANNEL, MARKER, PRIVATE, SECRET};
    use sex_app::{files::direct as files, launch, Event, Window};
    use sex_pdx::{launch as l, program_store as store};
    const BIN: &[u8] = b"/bin/ribbonseal";
    const SDK_ARGS: &[u8] = b"--child sdk ribbon-owner-20261115 @/ribbon-private";
    const SDK_MAIN: u32 = 0x5001;
    const FOREIGN_ARGS: &[u8] = b"--child foreign ribbon-owner-20261115 @/ribbon-private";
    const STALE_ARGS: &[u8] = b"--child stale ribbon-owner-20261115 @/ribbon-private";
    const PLAIN_ARGS: &[u8] = b"--child reuse ribbon-owner-20261115";
    const FOREIGN_RAW: u32 = 0x5201;
    const FOREIGN_MAIN: u32 = 0x5202;
    const REPLAY: u32 = 0x5203;
    const REUSE_RAW: u32 = 0x5301;
    const REUSE_MAIN: u32 = 0x5302;
    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[ribbon.failed] invariant=1 detail={}", info);
        launch::exit(1)
    }
    fn pause() {
        let _ = sex_pdx::sys_wait_self_ticks(2);
    }
    fn identity() -> sex_pdx::app_lifecycle::CatalogInfo {
        sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT)
            .expect("identity")
    }
    fn read_private() -> Result<bool, u8> {
        let mut b = [0; 128];
        let n = files::read_at(PRIVATE, 0, &mut b)?;
        Ok(&b[..n] == SECRET)
    }
    fn denied_private() {
        // Files rejects an ungranted @ path during confine with NAME, before I/O.
        assert_eq!(read_private(), Err(sex_pdx::fs::NAME));
    }
    fn record() -> Record {
        let mut b = [0; 64];
        assert_eq!(
            files::read_at(CHANNEL, 0, &mut b).expect("channel read"),
            64
        );
        Record::decode(&b).expect("bounded record")
    }
    fn put(r: Record) {
        assert_eq!(
            files::write_at(CHANNEL, 0, &r.encode()).expect("channel write"),
            64
        );
        files::sync().expect("channel sync");
    }
    fn raw(ticket: u32, cookie: u32) {
        sex_pdx::pdx_call_checked(
            sex_pdx::SLOT_SHELL,
            l::OP_LAUNCH,
            ticket as u64,
            store::RUN_HIDDEN,
            cookie as u64,
        )
        .expect("enqueue launch");
    }
    fn next(cookie: u32) -> l::Event {
        let deadline = sex_pdx::get_ticks() + 30000;
        loop {
            if let Some(e) = launch::poll(cookie) {
                return e;
            }
            assert!(sex_pdx::get_ticks() < deadline, "launch deadline");
            pause()
        }
    }
    fn child_done(cookie: u32) -> (u32, u32) {
        let (row, task) = match next(cookie) {
            l::Event::Started { row, task, .. } if row != 0 && task != 0 => (row, task),
            _ => panic!("no start"),
        };
        match next(cookie) {
            l::Event::Done {
                row: r,
                task: t,
                status: Some(0),
                ..
            } if (row, task) == (r, t) => {}
            _ => panic!("bad child end"),
        };
        sex_pdx::serial_record!(
            "[ribbon.authorized] cookie={} row={} task={} ok=1",
            cookie,
            row,
            task
        );
        (row, task)
    }
    fn refused(cookie: u32) -> u32 {
        match next(cookie) {
            l::Event::Refused { code, .. } => {
                assert_eq!(
                    code,
                    store::NOT_FOUND,
                    "resource refusal is not ownership proof"
                );
                sex_pdx::serial_record!("[ribbon.refused] cookie={} code={} ok=1", cookie, code);
                code
            }
            _ => panic!("unauthorized child"),
        }
    }
    fn child(args: &[u8]) -> ! {
        let parts: alloc::vec::Vec<_> = args
            .split(|b| *b == b' ')
            .filter(|p| !p.is_empty())
            .collect();
        assert!(parts.len() == 3 || parts.len() == 4);
        assert_eq!(parts[0], b"--child");
        assert_eq!(parts[2], MARKER);
        let own = identity();
        let grant = parts[1] != b"reuse";
        if grant {
            assert_eq!(parts.len(), 4);
            assert_eq!(parts[3], PRIVATE);
            assert_eq!(read_private(), Ok(true));
        } else {
            assert_eq!(parts.len(), 3);
            denied_private();
        }
        let phase = core::str::from_utf8(parts[1]).expect("phase");
        sex_pdx::serial_record!(
            "[ribbon.child] phase={} target={} task={} args_ok=1 grant={} ok=1",
            phase,
            own.target_pd,
            own.task_id,
            grant as u8
        );
        launch::exit(0)
    }
    fn seed() -> ! {
        for (path, data) in [(PRIVATE, SECRET), (CHANNEL, &Record::empty().encode()[..])] {
            files::create(path).expect("exclusive seed");
            assert_eq!(
                files::write_at(path, 0, data).expect("seed write"),
                data.len()
            );
        }
        files::sync().expect("seed sync");
        sex_pdx::serial_record!("[ribbon.seed] files=2 ok=1 developer_only=1");
        launch::exit(0)
    }
    fn owner_foreign() {
        let own = identity();
        let ticket = files::prepare(BIN, FOREIGN_ARGS).expect("owner prepare");
        let mut r = record();
        assert_eq!(r.0[0], 0);
        r.0[..4].copy_from_slice(&[1, ticket, own.target_pd, own.task_id]);
        put(r);
        sex_pdx::serial_record!(
            "[ribbon.pending] phase=foreign target={} task={} ticket={}",
            own.target_pd,
            own.task_id,
            ticket
        );
        let deadline = sex_pdx::get_ticks() + 30000;
        loop {
            r = record();
            if r.0[0] == 2 {
                break;
            }
            assert!(sex_pdx::get_ticks() < deadline);
            pause()
        }
        assert_eq!(&r.0[1..4], &[ticket, own.target_pd, own.task_id]);
        assert_ne!(r.0[4], own.target_pd);
        raw(ticket, FOREIGN_MAIN);
        child_done(FOREIGN_MAIN);
        raw(ticket, REPLAY);
        refused(REPLAY);
        r.0[0] = 3;
        put(r);
        sex_pdx::serial_record!(
            "[ribbon.owner.done] ticket={} foreign_denied=1 owner_consumed=1 single_use=1 ok=1",
            ticket
        );
    }
    fn foreign() -> ! {
        let own = identity();
        let mut window = Window::open();
        window.title(b"Ribbon Seal - foreign parent");
        window.show(b"Developer launch-owner qualification: foreign parent");
        denied_private();
        sex_pdx::serial_record!(
            "[ribbon.ready] role=foreign target={} task={} surface={} ok=1",
            own.target_pd,
            own.task_id,
            own.surface_id
        );
        let deadline = sex_pdx::get_ticks() + 90000;
        loop {
            let mut r = record();
            if r.0[0] == 1 {
                assert_ne!(r.0[2], own.target_pd);
                let before_args = {
                    let mut b = [0; 256];
                    let n = files::args(&mut b);
                    (b, n)
                };
                denied_private();
                raw(r.0[1], FOREIGN_RAW);
                let code = refused(FOREIGN_RAW);
                denied_private();
                let mut after = [0; 256];
                let n = files::args(&mut after);
                assert_eq!(&after[..n], &before_args.0[..before_args.1]);
                r.0[0] = 2;
                r.0[4] = own.target_pd;
                r.0[5] = own.task_id;
                r.0[6] = code;
                put(r);
                while record().0[0] != 3 {
                    assert!(sex_pdx::get_ticks() < deadline);
                    pause()
                }
                assert_eq!(files::prepare(BIN, FOREIGN_ARGS), Err(sex_pdx::fs::DENIED));
                sex_pdx::serial_record!("[ribbon.foreign.done] target={} task={} original_args=1 read_denied=1 delegation_denied=1 ok=1",own.target_pd,own.task_id);
                break;
            }
            assert!(sex_pdx::get_ticks() < deadline);
            if let Some(Event::Close) = window.next_within(20) {
                launch::exit(2)
            }
        }
        loop {
            if record().0[0] == 5 {
                sex_pdx::serial_record!("[ribbon.foreign.exit] ok=1");
                launch::exit(0)
            }
            if let Some(Event::Close) = window.next_within(20) {
                launch::exit(2)
            }
        }
    }
    fn reuse() -> ! {
        let own = identity();
        let r = record();
        assert_eq!(r.0[0], 4);
        assert_eq!(r.0[2], own.target_pd);
        assert_ne!(r.0[3], own.task_id);
        denied_private();
        raw(r.0[1], REUSE_RAW);
        refused(REUSE_RAW);
        denied_private();
        launch::run_hidden(b"ribbonseal", PLAIN_ARGS, REUSE_MAIN).expect("new main launch");
        child_done(REUSE_MAIN);
        let mut done = r;
        done.0[0] = 5;
        put(done);
        sex_pdx::serial_record!("[ribbon.reuse.done] target={} old_task={} new_task={} stale_ticket={} stale_refused=1 no_old_grant=1 plain_child=1 composite_scope=1 ok=1",own.target_pd,r.0[3],own.task_id,r.0[1]);
        launch::exit(0)
    }
    fn owner() -> ! {
        let own = identity();
        let mut window = Window::open();
        window.title(b"Ribbon Seal - ticket owner");
        window.show(
            b"Developer launch-owner qualification. V SDK launch; F foreign; R prepare and retire.",
        );
        assert_eq!(read_private(), Ok(true));
        sex_pdx::serial_record!(
            "[ribbon.ready] role=owner target={} task={} surface={} ok=1",
            own.target_pd,
            own.task_id,
            own.surface_id
        );
        let mut phase = 0;
        loop {
            match window.next_within(20) {
                Some(Event::Key(k)) if k.code == 0x2f && phase == 0 => {
                    launch::run_hidden(b"ribbonseal", SDK_ARGS, SDK_MAIN).expect("SDK main launch");
                    child_done(SDK_MAIN);
                    sex_pdx::serial_record!(
                        "[ribbon.sdk.done] inherited_argument=1 inherited_file=1 ok=1"
                    );
                    phase = 1;
                }
                Some(Event::Key(k)) if k.code == 0x21 && phase == 1 => {
                    owner_foreign();
                    phase = 2
                }
                Some(Event::Key(k)) if k.code == 0x13 && phase == 2 => {
                    let ticket = files::prepare(BIN, STALE_ARGS).expect("stale prepare");
                    let mut r = record();
                    r.0[0] = 4;
                    r.0[1] = ticket;
                    r.0[2] = own.target_pd;
                    r.0[3] = own.task_id;
                    put(r);
                    sex_pdx::serial_record!(
                        "[ribbon.retire.pending] target={} task={} ticket={} ok=1",
                        own.target_pd,
                        own.task_id,
                        ticket
                    );
                    launch::exit(0)
                }
                Some(Event::Close) => launch::exit(2),
                _ => {}
            }
        }
    }
    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        let mut args = [0; 256];
        let n = files::args(&mut args);
        let args = &args[..n];
        if args.split(|b| *b == b' ').any(|p| p == b"--seed") {
            seed()
        }
        if args.starts_with(b"--child ") {
            child(args)
        }
        if args.starts_with(b"--owner ") {
            owner()
        }
        if args.starts_with(b"--foreign ") {
            foreign()
        }
        if args.starts_with(b"--reuse ") {
            reuse()
        }
        panic!("bad qualifier mode")
    }
}
