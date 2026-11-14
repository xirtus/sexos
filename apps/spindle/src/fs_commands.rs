//! File commands. One request at a time, owned by the main loop: `send` delivers it and
//! `receive` takes the answer, so a slow disk never stalls typing.
use crate::Scrollback;
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

const CWD_MAX: usize = 512;
const CAT_MAX: u64 = 32 * 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmd { Ls, Cd, Mkdir, Rm, Mv, Cat, Create, Write, Append(u8), Df, Program(u64, u64), Install, Mount, Unmount, Update, Package }
pub struct Client { sent_at: u64, cmd: Option<Cmd>, seq: u32, kind: u8, path_len: u16, offset: u64, length: u64, sent: bool, cwd: [u8; CWD_MAX], cwd_len: usize, carry: [u8; 160], carry_len: usize, shown: u64, data_len: usize, bare: [u8; 16], bare_len: usize }
/// How Files answered the last update request: OK go on, EXISTS nothing newer, else stop.
pub static mut UPDATE_STATUS: Option<u8> = None;
/// How Files answered the last package request.
pub static mut PACKAGE_STATUS: Option<u8> = None;
/// The last package report's lines meant for Spindle (URLs), newline separated.
pub static mut PACKAGE_MACHINE: ([u8; 4096], usize) = ([0; 4096], 0);
pub static mut FS: Client = Client { sent_at: 0, cmd: None, seq: 0, kind: 0, path_len: 0, offset: 0, length: 0, sent: false, cwd: [0; CWD_MAX], cwd_len: 0, carry: [0; 160], carry_len: 0, shown: 0, data_len: 0, bare: [0; 16], bare_len: 0 };

fn region() -> Option<&'static mut [u8]> {
    let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
}
fn split(args: &[u8]) -> (&[u8], &[u8]) {
    match args.iter().position(|b| *b == b' ') {
        Some(i) => (&args[..i], { let mut r = &args[i + 1..]; while r.first() == Some(&b' ') { r = &r[1..]; } r }),
        None => (args, &args[args.len()..]),
    }
}
struct Text { buf: [u8; 160], len: usize }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
macro_rules! say { ($sb:expr, $($arg:tt)*) => {{
    let mut t = Text { buf: [0; 160], len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!($($arg)*));
    $sb.push(&t.buf[..t.len]);
    sex_pdx::serial_record!("[spindle.fs.line] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}
fn shown(bytes: &[u8]) -> &str { core::str::from_utf8(bytes).unwrap_or("?") }

impl Client {
    pub fn busy(&self) -> bool { self.cmd.is_some() }
    /// `path` made absolute against the working directory, without `.` and `..`.
    pub fn join(&self, path: &[u8], out: &mut [u8]) -> Option<usize> {
        let mut len = 0;
        let mut add = |part: &[u8], out: &mut [u8], len: &mut usize| -> Option<()> {
            for piece in part.split(|b| *b == b'/').filter(|p| !p.is_empty()) {
                if piece == b"." { continue; }
                if piece == b".." {
                    while *len > 0 { *len -= 1; if out[*len] == b'/' { break; } }
                    continue;
                }
                if *len + 1 + piece.len() > out.len() { return None; }
                out[*len] = b'/';
                out[*len + 1..*len + 1 + piece.len()].copy_from_slice(piece);
                *len += 1 + piece.len();
            }
            Some(())
        };
        if path.first() != Some(&b'/') { add(&self.cwd[..self.cwd_len], out, &mut len)?; }
        add(path, out, &mut len)?;
        Some(len)
    }
    fn start(&mut self, cmd: Cmd, kind: u8, path_len: usize, offset: u64, length: u64) {
        self.cmd = Some(cmd); self.kind = kind; self.path_len = path_len as u16; self.offset = offset; self.length = length; self.sent = false;
        self.seq = (self.seq + 1) & 0xFF_FFFF;
        self.carry_len = 0; self.shown = 0;
    }
    /// Asks Files to check (two paths) or apply (three) a downloaded update. The outcome
    /// lands in [`UPDATE_STATUS`].
    pub fn start_update(&mut self, paths: &[&[u8]], sb: &mut Scrollback) -> bool {
        if self.busy() { return false; }
        let Some(region) = region() else { return false };
        let mut at = 0;
        for (i, p) in paths.iter().enumerate() {
            if i > 0 { region[at] = 0; at += 1; }
            if at + p.len() > w::PATH_BYTES { sb.push(b"Path too long."); return false; }
            region[at..at + p.len()].copy_from_slice(p);
            at += p.len();
        }
        unsafe { UPDATE_STATUS = None; }
        self.start(Cmd::Update, w::UPDATE, at, 0, 0);
        true
    }
    /// A package request (`sex_pdx::package`): `arg` is a path, name or key. The outcome
    /// lands in [`PACKAGE_STATUS`] and Files' report goes to the scrollback.
    pub fn start_package(&mut self, op: u64, arg: &[u8], length: u64) -> bool {
        if self.busy() || arg.len() > w::PATH_BYTES { return false; }
        let Some(region) = region() else { return false };
        region[..arg.len()].copy_from_slice(arg);
        unsafe { PACKAGE_STATUS = None; }
        self.start(Cmd::Package, sex_pdx::package::KIND, arg.len(), op, length);
        true
    }
    /// Starts a command. False only for `ls` with arguments unusable here.
    pub fn submit_line(&mut self, cmd: &[u8], args: &[u8], sb: &mut Scrollback) -> bool {
        if self.busy() { sb.push(b"The disk is busy with the last command."); return true; }
        let Some(region) = region() else { return false };
        let (first, rest) = split(args);
        if cmd == b"pwd" {
            if self.cwd_len == 0 { say!(sb, "/"); } else { say!(sb, "{}", shown(&self.cwd[..self.cwd_len])); }
            return true;
        }
        if cmd == b"install" {
            // install [go] [GIB] [IMAGE]: plans, and with `go` installs. GIB sizes the
            // SexOS store (default: all the free space); IMAGE is a boot image file
            // (default: the running system).
            let (mut go, mut gib, mut image): (bool, u64, &[u8]) = (false, 0, b"");
            let mut rest = args;
            while !rest.is_empty() {
                let (word, more) = split(rest);
                if word == b"go" { go = true; }
                else if !word.is_empty() && word.iter().all(u8::is_ascii_digit) { gib = word.iter().fold(0u64, |n, d| n.saturating_mul(10).saturating_add((d - b'0') as u64)); }
                else if !word.is_empty() { image = word; }
                rest = more;
            }
            let mut path = [0u8; w::PATH_BYTES];
            let n = if image.is_empty() { 0 } else { let Some(n) = self.join(image, &mut path) else { sb.push(b"Path too long."); return true }; n };
            region[..n].copy_from_slice(&path[..n]);
            if go { say!(sb, "Installing. Don't turn the machine off."); } else { say!(sb, "Looking at the disk..."); }
            self.start(Cmd::Install, w::INSTALL, n, gib.saturating_mul(1024), if go { w::INSTALL_GO } else { 0 });
            return true;
        }
        if cmd == b"df" && first.is_empty() { self.start(Cmd::Df, w::STATFS, 0, 0, 0); return true; }
        // `unmount NAME` means /media/NAME.
        let mut media = [0u8; 300];
        let first = if cmd == b"unmount" && !first.is_empty() && !first.contains(&b'/') && w::MEDIA.len() + 1 + first.len() <= media.len() {
            let m = w::MEDIA.len();
            media[..m].copy_from_slice(w::MEDIA); media[m] = b'/'; media[m + 1..m + 1 + first.len()].copy_from_slice(first);
            &media[..m + 1 + first.len()]
        } else { first };
        if first.is_empty() && !matches!(cmd, b"ls" | b"cd") { say!(sb, "Usage: {} PATH", shown(cmd)); return true; }
        let mut path = [0u8; w::PATH_BYTES];
        let Some(mut n) = self.join(first, &mut path) else { sb.push(b"Path too long."); return true };
        if cmd == b"cd" && first.is_empty() { self.cwd_len = 0; return true; }
        match cmd {
            b"ls" | b"cd" | b"cat" | b"df" | b"mount" | b"unmount" => {}
            b"mkdir" | b"rm" | b"touch" | b"write" | b"append" => {}
            b"mv" => {
                if rest.is_empty() { sb.push(b"Usage: mv FROM TO"); return true; }
                path[n] = 0;
                let mut to = [0u8; w::PATH_BYTES / 2];
                let Some(m) = self.join(rest, &mut to) else { sb.push(b"Path too long."); return true };
                if n + 1 + m > w::PATH_BYTES { sb.push(b"Path too long."); return true; }
                path[n + 1..n + 1 + m].copy_from_slice(&to[..m]);
                n += 1 + m;
            }
            _ => return false,
        }
        region[..n].copy_from_slice(&path[..n]);
        match cmd {
            b"ls" => self.start(Cmd::Ls, w::LIST, n, 0, 0),
            b"cd" => self.start(Cmd::Cd, w::STAT, n, 0, 0),
            b"mkdir" => self.start(Cmd::Mkdir, w::MKDIR, n, 0, 0),
            b"rm" => self.start(Cmd::Rm, w::REMOVE, n, 0, 0),
            b"mv" => self.start(Cmd::Mv, w::RENAME, n, 0, 0),
            b"cat" => self.start(Cmd::Cat, w::READ, n, 0, 4096),
            b"touch" => self.start(Cmd::Create, w::CREATE, n, 0, 0),
            b"df" => self.start(Cmd::Df, w::STATFS, n, 0, 0),
            b"mount" => self.start(Cmd::Mount, w::MOUNT, n, 0, 0),
            b"unmount" => { say!(sb, "Writing out {}...", shown(&region[..n])); self.start(Cmd::Unmount, w::UNMOUNT, n, 0, 0) }
            _ => {
                // Text after the path, with a line ending. Create it if needed.
                let length = rest.len().min(w::DATA_MAX - 1);
                let data = &mut region[w::PATH_BYTES..];
                data[..length].copy_from_slice(&rest[..length]);
                data[length] = b'\n';
                self.data_len = length + 1;
                self.start(Cmd::Write, w::STAT, n, 0, 0);
                self.cmd = Some(if cmd == b"write" { Cmd::Write } else { Cmd::Append(0) });
                // STAT would overwrite the data area, so hold the text past it.
                data.copy_within(0..length + 1, 64);
                self.kind = w::STAT;
            }
        }
        true
    }
    /// `run` or `spawn` with a path. Files holds the path and answers with a ticket for Shell.
    /// A bare name means the program directory. `args` reach the program, and `flags`
    /// carry what the person allowed this run, such as the network.
    pub fn submit_program(&mut self, target: &[u8], args: &[u8], flags: u64, count: u64, sb: &mut Scrollback) {
        self.submit_program_granted(target, args, flags, count, false, sb)
    }
    /// `packages`: the run may install and remove packages (`run +pkg`). Files sees that as
    /// the argument `+packages`, which only this adds: a typed one is dropped.
    pub fn submit_program_granted(&mut self, target: &[u8], args: &[u8], flags: u64, count: u64, packages: bool, sb: &mut Scrollback) {
        if self.busy() { sb.push(b"The disk is busy with the last command."); return; }
        let Some(region) = region() else { sb.push(b"The disk is not available."); return };
        if args.len() > w::ARGS_MAX { sb.push(b"Arguments too long."); return; }
        let mut path = [0u8; w::PATH_BYTES];
        let mut named = [0u8; 64];
        self.bare_len = 0;
        if !target.contains(&b'/') && target.len() <= self.bare.len() { self.bare[..target.len()].copy_from_slice(target); self.bare_len = target.len(); }
        let target = if target.contains(&b'/') { target } else {
            let bin = sex_pdx::program_store::BIN;
            if bin.len() + target.len() > named.len() { sb.push(b"Name too long."); return; }
            named[..bin.len()].copy_from_slice(bin);
            named[bin.len()..bin.len() + target.len()].copy_from_slice(target);
            &named[..bin.len() + target.len()]
        };
        let Some(n) = self.join(target, &mut path) else { sb.push(b"Path too long."); return };
        // `@PATH` gives the program that one file or folder: it travels as `@/absolute/path`.
        let mut given = [0u8; w::ARGS_MAX];
        let mut m = 0;
        let token = sex_pdx::package::GRANT_TOKEN;
        if packages { given[..token.len()].copy_from_slice(token); m = token.len(); }
        for word in args.split(|b| *b == b' ').filter(|w| !w.is_empty() && *w != token) {
            let mut piece = [0u8; w::ARGS_MAX];
            let word: &[u8] = match word.strip_prefix(b"@") {
                Some(file) => {
                    let Some(mut k) = self.join(file, &mut piece[1..]) else { sb.push(b"Path too long."); return };
                    // The root joins to nothing: it is `/`.
                    if k == 0 { piece[1] = b'/'; k = 1; }
                    piece[0] = b'@';
                    &piece[..k + 1]
                }
                None => word,
            };
            let sep = (m > 0) as usize;
            if m + sep + word.len() > given.len() { sb.push(b"Arguments too long."); return; }
            if sep == 1 { given[m] = b' '; }
            given[m + sep..m + sep + word.len()].copy_from_slice(word);
            m += sep + word.len();
        }
        region[..n].copy_from_slice(&path[..n]);
        region[w::PATH_BYTES..w::PATH_BYTES + m].copy_from_slice(&given[..m]);
        self.start(Cmd::Program(count, flags), w::PREPARE, n, 0, m as u64);
    }
    /// Delivers the current request. Retries a full queue on the next pass.
    pub fn send(&mut self) -> bool {
        if self.cmd.is_none() || self.sent { return false; }
        let r = w::Request { seq: self.seq, kind: self.kind, path_len: self.path_len, offset: self.offset, length: self.length };
        let a = r.args();
        match sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, w::OP, a[0], a[1], a[2]) {
            Ok(_) => { self.sent = true; self.sent_at = sex_pdx::get_ticks(); }
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => {}
            Err(_) => { self.cmd = None; }
        }
        self.sent
    }
    /// A request Files never answers, because it restarted, ends rather than holding the
    /// terminal busy for good. True if it just ended.
    pub fn expired(&mut self, now: u64, sb: &mut Scrollback) -> bool {
        if self.cmd.is_none() || !self.sent || now.saturating_sub(self.sent_at) < 15_000 { return false; }
        self.cmd = None;
        if self.kind == w::UPDATE { unsafe { UPDATE_STATUS = Some(w::DENIED); } }
        sb.push(b"The disk did not answer. Try again.");
        sex_pdx::serial_record!("[spindle.fs.expired] kind={}", self.kind);
        true
    }
    fn next(&mut self, kind: u8, offset: u64, length: u64) {
        self.kind = kind; self.offset = offset; self.length = length; self.sent = false;
        self.seq = (self.seq + 1) & 0xFF_FFFF;
    }
    /// True if `word` answered the request in flight.
    pub fn receive(&mut self, word: u64, sb: &mut Scrollback) -> bool {
        let Some((seq, status, value)) = w::decode_reply(word) else { return false };
        let Some(cmd) = self.cmd else { return false };
        if !self.sent || seq != self.seq { return false; }
        let Some(region) = region() else { self.cmd = None; return true };
        let data_at = w::PATH_BYTES;
        sex_pdx::serial_record!("[spindle.fs.result] kind={} status={} value={}", self.kind, status, value);
        if cmd == Cmd::Package {
            self.cmd = None;
            let bytes = &region[data_at..data_at + (value as usize).min(w::DATA_MAX)];
            let machine = unsafe { &mut *core::ptr::addr_of_mut!(PACKAGE_MACHINE) };
            machine.1 = 0;
            for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                if line[0] == sex_pdx::package::MACHINE {
                    let l = &line[1..];
                    if machine.1 + l.len() + 1 <= machine.0.len() { machine.0[machine.1..machine.1 + l.len()].copy_from_slice(l); machine.0[machine.1 + l.len()] = b'\n'; machine.1 += l.len() + 1; }
                    continue;
                }
                sex_pdx::serial_record!("[spindle.pkg.line] {}", shown(line));
                for part in line.chunks(96) { sb.push(part); }
            }
            if value == 0 && status != w::OK { say!(sb, "pkg: {}", shown(w::message(status))); }
            unsafe { PACKAGE_STATUS = Some(status); }
            return true;
        }
        if cmd == Cmd::Update {
            // Files' report, whatever it decided.
            self.cmd = None;
            let bytes = &region[data_at..data_at + (value as usize).min(w::DATA_MAX)];
            for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                sex_pdx::serial_record!("[spindle.update.line] {}", shown(line));
                for part in line.chunks(96) { sb.push(part); }
            }
            if value == 0 && status != w::OK { say!(sb, "update: {}", shown(w::message(status))); }
            unsafe { UPDATE_STATUS = Some(status); }
            return true;
        }
        if status != w::OK {
            // A missing file is what write and append start from.
            if status == w::NOT_FOUND && matches!(cmd, Cmd::Write | Cmd::Append(0)) && self.kind == w::STAT {
                self.next(w::CREATE, 0, 0);
                return true;
            }
            self.cmd = None;
            // A bare name with no filesystem to prepare it: the disk program store may have it.
            if let (Cmd::Program(0, _), true) = (cmd, self.bare_len > 0) {
                if let Some(name) = sex_pdx::program_store::pack_name(&self.bare[..self.bare_len]) {
                    let asked = sex_pdx::pdx_call_checked(crate::SLOT_SHELL, sex_pdx::program_store::OP_RUN, name[0], name[1], 0).is_ok();
                    say!(sb, "{}", if asked { "Asked for the program. A window opens if it starts." } else { "Could not reach the shell." });
                    return true;
                }
            }
            // A disk too small for a filesystem still has the old object list.
            if status == w::UNAVAILABLE && cmd == Cmd::Ls {
                let accepted = unsafe { !crate::HISTORY_IO.fenced() && crate::STORAGE_PROBE.submit(crate::storage_probe::Job::List) };
                sb.push(if accepted { b"Object listing queued." } else { b"Object listing unavailable." });
                return true;
            }
            say!(sb, "{}", shown(w::message(status)));
            return true;
        }
        match cmd {
            Cmd::Ls => {
                let data = &region[data_at..];
                let next = u64::from_le_bytes(data[..8].try_into().unwrap());
                let mut at = w::LIST_HEAD;
                while at + w::ENTRY_HEAD <= value as usize {
                    let (dir, len) = (data[at] == 1, data[at + 1] as usize);
                    let size = u64::from_le_bytes(data[at + 2..at + 10].try_into().unwrap());
                    let name = shown(&data[at + w::ENTRY_HEAD..at + w::ENTRY_HEAD + len]);
                    if dir { say!(sb, "{}/", name); } else { say!(sb, "{}  {}", name, size); }
                    at += w::ENTRY_HEAD + len;
                }
                if next != 0 { self.next(w::LIST, next, 0); return true; }
            }
            Cmd::Cd => {
                let data = &region[data_at..];
                if u64::from_le_bytes(data[..8].try_into().unwrap()) != 1 { say!(sb, "not a directory"); }
                else if self.path_len as usize <= CWD_MAX {
                    let n = self.path_len as usize;
                    let path = &region[..n];
                    self.cwd[..n].copy_from_slice(path);
                    self.cwd_len = n;
                }
            }
            Cmd::Cat => {
                let bytes = &region[data_at..data_at + value as usize];
                for b in bytes {
                    if *b == b'\n' || self.carry_len == self.carry.len() {
                        sb.push(&self.carry[..self.carry_len]);
                        sex_pdx::serial_record!("[spindle.fs.cat] {}", shown(&self.carry[..self.carry_len]));
                        self.carry_len = 0;
                        if *b == b'\n' { continue; }
                    }
                    self.carry[self.carry_len] = if (32..127).contains(b) { *b } else { b'.' };
                    self.carry_len += 1;
                }
                self.shown += value as u64;
                if value == 4096 && self.shown < CAT_MAX { self.next(w::READ, self.shown, 4096); return true; }
                if self.carry_len > 0 {
                    sb.push(&self.carry[..self.carry_len]);
                    sex_pdx::serial_record!("[spindle.fs.cat] {}", shown(&self.carry[..self.carry_len]));
                }
                if value == 4096 { say!(sb, "(stopped at {} bytes)", self.shown); }
            }
            Cmd::Write | Cmd::Append(_) => {
                let data = &mut region[data_at..];
                match self.kind {
                    w::STAT => {
                        // It exists. Write replaces, so cut it first. Append goes at the end.
                        let size = u64::from_le_bytes(data[8..16].try_into().unwrap());
                        if u64::from_le_bytes(data[..8].try_into().unwrap()) == 1 { say!(sb, "is a directory"); self.cmd = None; return true; }
                        data.copy_within(64..64 + self.data_len, 0);
                        if cmd == Cmd::Write { self.next(w::TRUNCATE, 0, 0); } else { let length = self.data_len as u64; self.next(w::WRITE, size, length); }
                        return true;
                    }
                    w::CREATE => { data.copy_within(64..64 + self.data_len, 0); let length = self.data_len as u64; self.next(w::WRITE, 0, length); return true; }
                    w::TRUNCATE => { let length = self.data_len as u64; self.next(w::WRITE, 0, length); return true; }
                    _ => { if (value as usize) < self.data_len { say!(sb, "disk full: wrote {} bytes", value); } }
                }
            }
            Cmd::Df => {
                let data = &region[data_at..];
                let total = u64::from_le_bytes(data[..8].try_into().unwrap());
                let free = u64::from_le_bytes(data[8..16].try_into().unwrap());
                say!(sb, "{} MiB free of {} MiB", free * 4096 / (1024 * 1024), total * 4096 / (1024 * 1024));
            }
            Cmd::Mkdir | Cmd::Rm | Cmd::Mv | Cmd::Create => { say!(sb, "ok"); }
            Cmd::Install => {
                // Files' report: lines of text, wrapped to the scrollback's width.
                let bytes = &region[data_at..data_at + (value as usize).min(w::DATA_MAX)];
                for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                    sex_pdx::serial_record!("[spindle.install.line] {}", shown(line));
                    for part in line.chunks(96) { sb.push(part); }
                }
            }
            Cmd::Mount => {
                // One line per volume found: name, format, size.
                let text = &region[data_at..data_at + (value as usize).min(2048)];
                for line in text.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                    let name = line.split(|b| *b == b' ').next().unwrap_or(line);
                    let rest = &line[name.len()..];
                    say!(sb, "/media/{} ({})", shown(name), shown(rest.trim_ascii()));
                }
            }
            Cmd::Unmount => { say!(sb, "Safe to remove."); }
            Cmd::Update | Cmd::Package => {}
            Cmd::Program(count, flags) => {
                use sex_pdx::program_store as store;
                // A package's program: ask the person first, or apply what they said.
                let note = sex_pdx::package::Note::decode(&region[data_at..data_at + sex_pdx::package::NOTE_BYTES]);
                let name = if self.bare_len > 0 { &self.bare[..self.bare_len] } else { let path = &region[..self.path_len as usize]; path.rsplit(|b| *b == b'/').next().unwrap_or(path) };
                let Some(flags) = (unsafe { crate::pkg_command::PKG.before_run(note, name, value, flags, count, sb) }) else { self.cmd = None; return true };
                let (op, third) = if count == 0 { (store::OP_RUN, flags) } else { (store::OP_SPAWN, count) };
                let asked = sex_pdx::pdx_call_checked(crate::SLOT_SHELL, op, store::TICKET, value as u64, third).is_ok();
                say!(sb, "{}", if !asked { "Could not reach the shell." } else if count == 0 { "Asked for the program. A window opens if it starts." } else { "Asked for background programs. Check with: records" });
            }
        }
        self.cmd = None;
        true
    }
}
