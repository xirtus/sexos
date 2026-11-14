// SPDX-License-Identifier: GPL-3.0-only
//! Behaviour of Wasm guests against a recording host: what they can reach, what they
//! cannot, and how they end.
use sex_wasm::{run, Event, Grants, Guest, Host, Limits, Outcome, Refusal};
use std::collections::HashMap;

#[derive(Default)]
struct Rec {
    shown: Vec<Vec<u8>>,
    titles: Vec<Vec<u8>>,
    logs: Vec<Vec<u8>>,
    keys: Vec<Event>,
    files: HashMap<Vec<u8>, Vec<u8>>,
    fetched: Vec<Vec<u8>>,
}
impl Host for Rec {
    fn show(&mut self, text: &[u8]) { self.shown.push(text.to_vec()); }
    fn title(&mut self, name: &[u8]) { self.titles.push(name.to_vec()); }
    fn log(&mut self, line: &[u8]) { self.logs.push(line.to_vec()); }
    fn next(&mut self) -> Event { if self.keys.is_empty() { Event::Close } else { self.keys.remove(0) } }
    fn read(&mut self, path: &[u8], out: &mut [u8]) -> Result<usize, Refusal> {
        let f = self.files.get(path).ok_or(Refusal::NotFound)?;
        let n = f.len().min(out.len());
        out[..n].copy_from_slice(&f[..n]);
        Ok(n)
    }
    fn write(&mut self, path: &[u8], data: &[u8]) -> Result<(), Refusal> { self.files.insert(path.to_vec(), data.to_vec()); Ok(()) }
    fn fetch(&mut self, url: &[u8], out: &mut [u8]) -> Result<(u32, usize), Refusal> {
        self.fetched.push(url.to_vec());
        out[..2].copy_from_slice(b"ok");
        Ok((200, 2))
    }
    fn now(&mut self) -> u64 { 42_000_000 }
}

fn guest(grants: Grants) -> Guest<'static> {
    Guest { name: b"test", args: &[b"one", b"two"], grants, limits: Limits::default() }
}
fn go(wat: &str, grants: Grants, host: Rec) -> (Outcome, Rec) {
    run(&wat::parse_str(wat).unwrap(), &guest(grants), host)
}
fn folder() -> Grants { Grants { folder: Some(b"guests/t".to_vec()), net: false } }

#[test]
fn shows_text_and_title_then_returns() {
    let (o, h) = go(r#"(module
        (import "sex" "show" (func $show (param i32 i32) (result i32)))
        (import "sex" "title" (func $title (param i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 16) "Hello from Wasm")
        (func (export "_start")
            (drop (call $title (i32.const 16) (i32.const 5)))
            (drop (call $show (i32.const 16) (i32.const 15)))))"#, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Exited(0));
    assert_eq!(h.shown, vec![b"Hello from Wasm".to_vec()]);
    assert_eq!(h.titles, vec![b"Hello".to_vec()]);
}

#[test]
fn exit_code_and_keys() {
    // Echo typed bytes until Enter, then exit with the count.
    let wat = r#"(module
        (import "sex" "next" (func $next (result i32)))
        (import "sex" "show" (func $show (param i32 i32) (result i32)))
        (import "sex" "exit" (func $exit (param i32)))
        (memory (export "memory") 1)
        (func (export "_start") (local $k i32) (local $n i32)
            (loop $l
                (local.set $k (call $next))
                (if (i32.eq (local.get $k) (i32.const 10)) (then (call $exit (local.get $n))))
                (if (i32.eq (local.get $k) (i32.const -1)) (then (call $exit (i32.const 99))))
                (i32.store8 (local.get $n) (local.get $k))
                (local.set $n (i32.add (local.get $n) (i32.const 1)))
                (drop (call $show (i32.const 0) (local.get $n)))
                (br $l))))"#;
    let keys = vec![Event::Char(b'h'), Event::Nav(0), Event::Char(b'i'), Event::Char(b'\n')];
    let (o, h) = go(wat, Grants::default(), Rec { keys, ..Default::default() });
    // The Nav key is stored as its code's low byte, so three bytes were kept.
    assert_eq!(o, Outcome::Exited(3));
    assert_eq!(h.shown.last().unwrap()[0], b'h');
    let (o, _) = go(wat, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Exited(99));
}

#[test]
fn a_guest_that_ignores_close_is_ended() {
    let (o, _) = go(r#"(module
        (import "sex" "next" (func $next (result i32)))
        (memory (export "memory") 1)
        (func (export "_start") (loop $l (drop (call $next)) (br $l))))"#, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Closed);
}

const FILES: &str = r#"(module
    (import "sex" "read" (func $read (param i32 i32 i32 i32) (result i32)))
    (import "sex" "write" (func $write (param i32 i32 i32 i32) (result i32)))
    (import "sex" "exit" (func $exit (param i32)))
    (memory (export "memory") 1)
    (data (i32.const 0) "notes.txt")
    (data (i32.const 16) "saved")
    (data (i32.const 32) "../escape")
    (data (i32.const 48) "/etc/x")
    (func (export "_start") (local $r i32)
        (local.set $r (call $write (i32.const 0) (i32.const 9) (i32.const 16) (i32.const 5)))
        (if (i32.ne (local.get $r) (i32.const 0)) (then (call $exit (local.get $r))))
        (local.set $r (call $read (i32.const 0) (i32.const 9) (i32.const 100) (i32.const 50)))
        (if (i32.ne (local.get $r) (i32.const 5)) (then (call $exit (i32.const 50))))
        (if (i32.ne (i32.load8_u (i32.const 100)) (i32.const 115)) (then (call $exit (i32.const 51))))
        (local.set $r (call $read (i32.const 32) (i32.const 9) (i32.const 100) (i32.const 50)))
        (if (i32.ne (local.get $r) (i32.const -3)) (then (call $exit (i32.const 52))))
        (local.set $r (call $write (i32.const 48) (i32.const 6) (i32.const 16) (i32.const 5)))
        (if (i32.ne (local.get $r) (i32.const -3)) (then (call $exit (i32.const 53))))
        ;; An output buffer past the end of memory is refused, not a trap.
        (local.set $r (call $read (i32.const 0) (i32.const 9) (i32.const 65530) (i32.const 50)))
        (if (i32.ne (local.get $r) (i32.const -3)) (then (call $exit (i32.const 54))))
        (call $exit (i32.const 7))))"#;

#[test]
fn files_stay_inside_the_guests_folder() {
    let (o, h) = go(FILES, folder(), Rec::default());
    assert_eq!(o, Outcome::Exited(7));
    assert_eq!(h.files.keys().collect::<Vec<_>>(), vec![&b"guests/t/notes.txt".to_vec()]);
}

#[test]
fn files_refused_without_a_folder_grant() {
    let (o, h) = go(FILES, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Exited(Refusal::NotAllowed.code()));
    assert!(h.files.is_empty());
}

const FETCH: &str = r#"(module
    (import "sex" "fetch" (func $fetch (param i32 i32 i32 i32) (result i64)))
    (import "sex" "exit" (func $exit (param i32)))
    (memory (export "memory") 1)
    (data (i32.const 0) "https://example.org/")
    (func (export "_start") (local $r i64)
        (local.set $r (call $fetch (i32.const 0) (i32.const 20) (i32.const 100) (i32.const 64)))
        (if (i64.lt_s (local.get $r) (i64.const 0)) (then (call $exit (i32.wrap_i64 (local.get $r)))))
        (call $exit (i32.wrap_i64 (i64.shr_u (local.get $r) (i64.const 32))))))"#;

#[test]
fn network_only_with_the_net_grant() {
    let (o, h) = go(FETCH, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Exited(Refusal::NotAllowed.code()));
    assert!(h.fetched.is_empty(), "the host must not be asked");
    let (o, h) = go(FETCH, Grants { folder: None, net: true }, Rec::default());
    assert_eq!(o, Outcome::Exited(200));
    assert_eq!(h.fetched, vec![b"https://example.org/".to_vec()]);
}

#[test]
fn unknown_imports_are_refused_by_name() {
    let (o, _) = go(r#"(module (import "env" "open" (func)) (func (export "_start")))"#, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Rejected("needs env.open, which SexOS does not provide".into()));
    let (o, _) = go(r#"(module (import "sex" "spawn" (func)) (func (export "_start")))"#, Grants::default(), Rec::default());
    assert!(matches!(o, Outcome::Rejected(m) if m.contains("sex.spawn")));
}

#[test]
fn malformed_and_startless_modules_are_rejected() {
    let (o, _) = run(b"\0asm\x01\0\0\0\x05", &guest(Grants::default()), Rec::default());
    assert!(matches!(o, Outcome::Rejected(_)));
    let (o, _) = go("(module)", Grants::default(), Rec::default());
    assert!(matches!(o, Outcome::Rejected(m) if m.contains("_start")));
}

#[test]
fn traps_and_fuel() {
    let (o, _) = go(r#"(module (func (export "_start") unreachable))"#, Grants::default(), Rec::default());
    assert!(matches!(o, Outcome::Trapped(_)));
    let (o, _) = go(r#"(module (memory 1) (func (export "_start") (drop (i32.load (i32.const 70000)))))"#, Grants::default(), Rec::default());
    assert!(matches!(o, Outcome::Trapped(_)));
    let mut g = guest(Grants::default());
    g.limits.fuel = Some(10_000);
    let (o, _) = run(&wat::parse_str(r#"(module (func (export "_start") (loop $l (br $l))))"#).unwrap(), &g, Rec::default());
    assert_eq!(o, Outcome::OutOfFuel);
}

#[test]
fn memory_is_capped() {
    let mut g = guest(Grants::default());
    g.limits.memory_bytes = 2 << 16;
    // Growing past the cap fails (-1), as Wasm requires; a module asking for more at start is not run.
    let (o, _) = run(&wat::parse_str(r#"(module (import "sex" "exit" (func $exit (param i32)))
        (memory (export "memory") 1)
        (func (export "_start") (call $exit (memory.grow (i32.const 4)))))"#).unwrap(), &g, Rec::default());
    assert_eq!(o, Outcome::Exited(-1));
    let (o, _) = run(&wat::parse_str(r#"(module (memory 64) (func (export "_start")))"#).unwrap(), &g, Rec::default());
    assert!(matches!(o, Outcome::Trapped(_) | Outcome::Rejected(_)), "{o:?}");
}

/// A WASI program: prints argv[1], reads a line, prints it back, exits 3.
const WASI: &str = r#"(module
    (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "fd_read" (func $read (param i32 i32 i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "args_sizes_get" (func $asz (param i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "args_get" (func $aget (param i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "clock_time_get" (func $clock (param i32 i64 i32) (result i32)))
    (import "wasi_snapshot_preview1" "path_open" (func $open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
    (memory (export "memory") 1)
    (func (export "_start") (local $argv1 i32)
        (if (call $asz (i32.const 0) (i32.const 4)) (then unreachable))
        (if (i32.ne (i32.load (i32.const 0)) (i32.const 3)) (then (call $exit (i32.const 60))))
        (drop (call $aget (i32.const 100) (i32.const 200)))
        (local.set $argv1 (i32.load (i32.const 104)))
        ;; iovec at 300: argv[1], 3 bytes
        (i32.store (i32.const 300) (local.get $argv1)) (i32.store (i32.const 304) (i32.const 3))
        (drop (call $write (i32.const 1) (i32.const 300) (i32.const 1) (i32.const 308)))
        ;; read a line into 400
        (i32.store (i32.const 320) (i32.const 400)) (i32.store (i32.const 324) (i32.const 64))
        (drop (call $read (i32.const 0) (i32.const 320) (i32.const 1) (i32.const 328)))
        (i32.store (i32.const 304) (i32.load (i32.const 328))) (i32.store (i32.const 300) (i32.const 400))
        (drop (call $write (i32.const 1) (i32.const 300) (i32.const 1) (i32.const 308)))
        (drop (call $clock (i32.const 1) (i64.const 0) (i32.const 512)))
        (if (i64.ne (i64.load (i32.const 512)) (i64.const 42000000)) (then (call $exit (i32.const 61))))
        ;; Not provided: answers ENOSYS (52) instead of failing to link.
        (if (i32.ne (call $open (i32.const 3) (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0) (i64.const 0) (i64.const 0) (i32.const 0) (i32.const 0)) (i32.const 52))
            (then (call $exit (i32.const 62))))
        (call $exit (i32.const 3))))"#;

#[test]
fn wasi_console_args_stdin_and_exit() {
    let keys = vec![Event::Char(b'h'), Event::Char(b'x'), Event::Char(8), Event::Char(b'i'), Event::Char(b'\n')];
    let (o, h) = go(WASI, Grants::default(), Rec { keys, ..Default::default() });
    assert_eq!(o, Outcome::Exited(3));
    // Console: "one", then the typed line echoed ("hx" then backspace), then it printed back.
    assert_eq!(h.shown.last().unwrap(), b"onehi\nhi\n");
}

#[test]
fn closing_during_a_read_is_end_of_input() {
    let (o, h) = go(WASI, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Exited(3));
    assert_eq!(h.shown.last().unwrap(), b"one");
}

#[test]
fn console_keeps_the_tail_on_whole_lines() {
    let wat = r#"(module
        (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "line of text\n")
        (func (export "_start") (local $i i32)
            (i32.store (i32.const 100) (i32.const 0)) (i32.store (i32.const 104) (i32.const 13))
            (loop $l
                (drop (call $write (i32.const 1) (i32.const 100) (i32.const 1) (i32.const 108)))
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br_if $l (i32.lt_u (local.get $i) (i32.const 2000))))))"#;
    let (o, h) = go(wat, Grants::default(), Rec::default());
    assert_eq!(o, Outcome::Exited(0));
    let last = h.shown.last().unwrap();
    assert!(last.len() <= Limits::default().console_bytes);
    assert!(last.starts_with(b"line of text\n") && last.ends_with(b"line of text\n"));
}

/// A real program: Rust's std built for `wasm32-wasip1` (source in `guests/greet`).
#[test]
fn rust_std_program_runs() {
    let wasm = include_bytes!("greet.wasm");
    let mut keys: Vec<Event> = b"b a b\n\n".iter().map(|c| Event::Char(*c)).collect();
    keys.push(Event::Close);
    let g = Guest { name: b"greet", args: &[b"Andreas"], grants: Grants::default(), limits: Limits::default() };
    let (o, h) = run(wasm, &g, Rec { keys, ..Default::default() });
    assert_eq!(o, Outcome::Exited(5));
    let text = String::from_utf8(h.shown.last().unwrap().clone()).unwrap();
    assert_eq!(text, "Hello, Andreas, from Rust on Wasm.\nType words; an empty line ends.\nb a b\n\na: 1\nb: 2\n");
}

#[test]
fn an_empty_read_takes_no_input() {
    let wat = r#"(module
        (import "wasi_snapshot_preview1" "fd_read" (func $read (param i32 i32 i32 i32) (result i32)))
        (import "sex" "next" (func $next (result i32)))
        (import "sex" "exit" (func $exit (param i32)))
        (memory (export "memory") 1)
        (func (export "_start")
            (i32.store (i32.const 8) (i32.const 999))
            (drop (call $read (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 8)))
            (i32.store (i32.const 0) (i32.const 100)) (i32.store (i32.const 4) (i32.const 0))
            (drop (call $read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 12)))
            (if (i32.or (i32.load (i32.const 8)) (i32.load (i32.const 12))) (then (call $exit (i32.const 70))))
            ;; The key is still there for the guest.
            (call $exit (call $next))))"#;
    let (o, h) = go(wat, Grants::default(), Rec { keys: vec![Event::Char(b'q')], ..Default::default() });
    assert_eq!(o, Outcome::Exited(b'q' as i32));
    assert!(h.shown.is_empty());
}

#[test]
fn overlapping_writes_are_capped_in_total() {
    // 1024 iovecs, each the whole 1 MiB memory: one write may take at most io_bytes.
    let wat = r#"(module
        (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
        (import "sex" "exit" (func $exit (param i32)))
        (memory (export "memory") 16)
        (func (export "_start") (local $i i32)
            (loop $l
                (i32.store (i32.add (i32.const 64) (i32.mul (local.get $i) (i32.const 8))) (i32.const 0))
                (i32.store (i32.add (i32.const 68) (i32.mul (local.get $i) (i32.const 8))) (i32.const 1048576))
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br_if $l (i32.lt_u (local.get $i) (i32.const 1024))))
            (drop (call $write (i32.const 1) (i32.const 64) (i32.const 1024) (i32.const 0)))
            (call $exit (i32.load (i32.const 0)))))"#;
    let mut g = guest(Grants::default());
    g.limits.io_bytes = 4096;
    let (o, _) = run(&wat::parse_str(wat).unwrap(), &g, Rec::default());
    assert_eq!(o, Outcome::Exited(4096));
}

/// The `wasm` program's built-in guest. `SEX_WASM_BLESS=1 cargo test -p sex-wasm` rebuilds it.
#[test]
fn hello_guest_is_current() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("guests/hello");
    let built = wat::parse_file(dir.join("hello.wat")).unwrap();
    if std::env::var_os("SEX_WASM_BLESS").is_some() { std::fs::write(dir.join("hello.wasm"), &built).unwrap(); }
    assert_eq!(std::fs::read(dir.join("hello.wasm")).unwrap(), built, "hello.wasm is stale: rerun with SEX_WASM_BLESS=1");
    let keys = vec![Event::Char(b'h'), Event::Char(b'x'), Event::Char(8), Event::Char(b'i'), Event::Nav(0), Event::Char(b'\n')];
    let (o, h) = run(&built, &guest(Grants::default()), Rec { keys, ..Default::default() });
    assert_eq!(o, Outcome::Exited(2));
    assert!(h.shown.last().unwrap().ends_with(b"> hi"));
    assert_eq!(h.titles, vec![b"Hello".to_vec()]);
    assert_eq!(h.logs, vec![b"hello.ready".to_vec()]);
}

/// A terminal host (Term): raw bytes out, its own input stream in, nothing shown.
#[derive(Default)]
struct Tty { out: Vec<u8>, input: Vec<u8>, shown: usize }
impl Host for Tty {
    fn show(&mut self, _text: &[u8]) { self.shown += 1; }
    fn console(&mut self, bytes: &[u8]) -> bool { self.out.extend_from_slice(bytes); true }
    fn input(&mut self, out: &mut [u8]) -> Option<usize> {
        let n = out.len().min(self.input.len());
        out[..n].copy_from_slice(&self.input[..n]);
        self.input.drain(..n);
        Some(n)
    }
    fn now(&mut self) -> u64 { 42_000_000 }
}

#[test]
fn a_terminal_host_gets_raw_bytes_and_supplies_input() {
    let g = Guest { name: b"t", args: &[b"one", b"two"], grants: Grants::default(), limits: Limits::default() };
    let (o, h) = run(&wat::parse_str(WASI).unwrap(), &g, Tty { input: b"\x1b[31mred\n".to_vec(), ..Default::default() });
    assert_eq!(o, Outcome::Exited(3));
    // No echo from the guest side: the terminal echoes what the person types itself.
    assert_eq!(h.out, b"one\x1b[31mred\n");
    assert_eq!(h.shown, 0);
    // End of input reads as zero bytes.
    let (o, h) = run(&wat::parse_str(WASI).unwrap(), &g, Tty::default());
    assert_eq!((o, h.out), (Outcome::Exited(3), b"one".to_vec()));
}
