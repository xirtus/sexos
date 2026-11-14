// SPDX-License-Identifier: GPL-3.0-only
//! The window: a list, a status line and the keys, as text (the Weft toolkit comes
//! later). Every step the person takes is also a `[aftercare.*]` line in the serial
//! record, which is what scripts/aftercare_gate.py reads.
use crate::rng::HwRng;
use crate::settings::{self, Settings};
use crate::store::{self, DirSink, DirSource, DirStore};
use aftercare_core::{
    Error, ForgetPlan, Id, InitOptions, KdfParams, Node, NodeKind, Observer, Progress, PruneOptions, Repository, Snapshot,
    Timestamp,
};
use sex_std::sex_app::{self, Event, Nav, Window};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const ESC: u8 = 0x01;
const ENTER: u8 = 0x1C;
const BACKSPACE: u8 = 0x0E;
const ROWS: usize = 16;

/// Argon2id cost for repositories this app creates: 8 MiB, 2 passes, 1 lane. Under
/// QEMU's software CPU (TCG) unlocking takes about a second (the gate records it as
/// `[aftercare.unlock] ... ms=`); on hardware a few milliseconds. The engine's default
/// (64 MiB, 3 passes) takes many seconds under TCG. Existing repositories keep the cost
/// stored in their key file.
pub const KDF: KdfParams = KdfParams { m_cost_kib: 8 * 1024, t_cost: 2, p_cost: 1 };

fn rec(line: &str) {
    sex_std::record(line);
}

fn now() -> Timestamp {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    Timestamp { secs: d.as_secs() as i64, nanos: d.subsec_nanos() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Purpose {
    Source,
    Destination,
    /// Restore this path of this snapshot into the chosen folder.
    RestoreTo(Id, String),
}

struct Chooser {
    purpose: Purpose,
    /// The folder shown; `None` lists the folders this run may use.
    at: Option<String>,
    rows: Vec<String>,
}

enum Screen {
    Home,
    Choose(Chooser),
    Snapshots,
    /// A snapshot (index into `snaps`), a folder in it, and that folder's entries.
    Browse(usize, String, Vec<Node>),
    Report(String, Vec<String>),
}

enum Prompt {
    Unlock,
    NewPassword,
    ConfirmPassword(String),
    Keep,
    Schedule,
    NewFolder,
    ConfirmForget(ForgetPlan),
    ConfirmDelete(Id),
}

impl Prompt {
    fn name(&self) -> &'static str {
        match self {
            Prompt::Unlock => "unlock",
            Prompt::NewPassword => "new-password",
            Prompt::ConfirmPassword(_) => "confirm-password",
            Prompt::Keep => "keep",
            Prompt::Schedule => "schedule",
            Prompt::NewFolder => "new-folder",
            Prompt::ConfirmForget(_) => "confirm-forget",
            Prompt::ConfirmDelete(_) => "confirm-delete",
        }
    }
    fn secret(&self) -> bool {
        matches!(self, Prompt::Unlock | Prompt::NewPassword | Prompt::ConfirmPassword(_))
    }
    fn takes_text(&self) -> bool {
        !matches!(self, Prompt::ConfirmForget(_) | Prompt::ConfirmDelete(_))
    }
    fn label(&self) -> String {
        match self {
            Prompt::Unlock => "Password".into(),
            Prompt::NewPassword => "New repository. Choose a password".into(),
            Prompt::ConfirmPassword(_) => "Type the password again".into(),
            Prompt::Keep => "Keep (last= hourly= daily= weekly= monthly= yearly=)".into(),
            Prompt::Schedule => "Daily backup at HH:MM UTC, or off".into(),
            Prompt::NewFolder => "New folder name".into(),
            Prompt::ConfirmForget(p) => format!("Forget {} snapshot(s), keep {}, then prune unused data? Enter: yes, Esc: no", p.remove.len(), p.keep.len()),
            Prompt::ConfirmDelete(id) => format!("Forget snapshot {}? Enter: yes, Esc: no (Prune removes its data)", id.short()),
        }
    }
}

/// Shows progress while the engine works, and stops it on Esc or when the window closes.
struct Watch<'a> {
    window: &'a mut Window,
    what: &'static str,
    started: Instant,
    shown: Option<Instant>,
    cancel: bool,
    closed: bool,
}

impl Watch<'_> {
    fn new<'a>(window: &'a mut Window, what: &'static str) -> Watch<'a> {
        Watch { window, what, started: Instant::now(), shown: None, cancel: false, closed: false }
    }
}

impl Observer for Watch<'_> {
    fn progress(&mut self, p: &Progress) {
        if self.shown.is_some_and(|t| t.elapsed().as_millis() < 250) {
            return;
        }
        self.shown = Some(Instant::now());
        let mut current = p.current.clone();
        if current.len() > 70 {
            current = format!("...{}", &current[current.len() - 67..]);
        }
        let text = format!(
            "Aftercare: {}\n\n  {} files, {} folders\n  {} read, {} new\n  {} s\n\n  {}\n\nEsc cancels.",
            self.what,
            p.files,
            p.dirs,
            settings::bytes(p.bytes),
            settings::bytes(p.bytes_added),
            self.started.elapsed().as_secs(),
            current
        );
        self.window.show(text.as_bytes());
    }
    fn cancelled(&mut self) -> bool {
        while let Some(e) = self.window.poll() {
            match e {
                Event::Key(k) if k.code == ESC => self.cancel = true,
                Event::Close => {
                    self.cancel = true;
                    self.closed = true;
                }
                _ => {}
            }
        }
        self.cancel
    }
}

struct App {
    window: &'static mut Window,
    settings: Settings,
    /// Folders this run was given (`@/...`).
    granted: Vec<String>,
    repo: Option<Repository<DirStore>>,
    rng: Option<HwRng>,
    screen: Screen,
    prompt: Option<(Prompt, String)>,
    status: String,
    cursor: usize,
    top: usize,
    snaps: Vec<(Id, Snapshot)>,
    /// The day the "backup is due" notice was last shown.
    due_noticed: Option<i64>,
    quit: bool,
    /// The question last drawn, so each new one is recorded once.
    asked: Option<&'static str>,
}

impl App {
    fn save(&mut self) {
        match std::fs::write(settings::FILE, self.settings.format()) {
            Ok(()) => rec("[aftercare.settings.saved]"),
            Err(e) => self.status = format!("Could not save the settings: {e}"),
        }
    }

    fn rows(&self) -> usize {
        match &self.screen {
            Screen::Home => 0,
            Screen::Choose(c) => c.rows.len(),
            Screen::Snapshots => self.snaps.len(),
            Screen::Browse(_, _, nodes) => nodes.len(),
            Screen::Report(_, lines) => lines.len(),
        }
    }

    fn draw(&mut self) {
        let asking = self.prompt.as_ref().map(|(p, _)| p.name());
        if asking != self.asked {
            if let Some(name) = asking {
                rec(&format!("[aftercare.prompt] name={name}"));
            }
            self.asked = asking;
        }
        let mut out = String::new();
        let list = |out: &mut String, items: Vec<String>, cursor: usize, top: usize, marker: bool| {
            if items.is_empty() {
                out.push_str("  (nothing here)\n");
            }
            for (i, line) in items.iter().enumerate().skip(top).take(ROWS) {
                out.push_str(if marker && i == cursor { "> " } else { "  " });
                out.push_str(line);
                out.push('\n');
            }
            if items.len() > top + ROWS {
                out.push_str(&format!("  ... {} more\n", items.len() - top - ROWS));
            }
        };
        let keys;
        match &self.screen {
            Screen::Home => {
                out.push_str("Aftercare: backups\n\n");
                let s = &self.settings;
                let none = || "(not chosen)".to_string();
                let state = match (&self.repo, &s.destination) {
                    (Some(_), _) => "unlocked",
                    (None, Some(d)) if DirStore::exists(d) => "locked",
                    (None, Some(_)) => "no repository yet",
                    (None, None) => "",
                };
                out.push_str(&format!("  Source        {}\n", s.source.clone().unwrap_or_else(none)));
                out.push_str(&format!("  Destination   {}  {}\n", s.destination.clone().unwrap_or_else(none), state));
                out.push_str(&format!("  Keep          {}\n", settings::format_keep(&s.keep)));
                let sched = match s.schedule {
                    Some(_) => format!("{} UTC, while Aftercare is open and unlocked", settings::format_time(s.schedule)),
                    None => "off".into(),
                };
                out.push_str(&format!("  Daily backup  {sched}\n"));
                if let Some((id, snap)) = self.snaps.last() {
                    out.push_str(&format!("  Last backup   {} UTC  {}  ({} snapshots)\n", settings::civil(snap.time.secs), id.short(), self.snaps.len()));
                }
                keys = "S source  D destination  U unlock/create  B back up now  L snapshots\nK keep policy  F forget and prune  C check (Shift+C reads all data)\nT daily time  X clear stale locks  F11 close";
            }
            Screen::Choose(c) => {
                let what = match &c.purpose {
                    Purpose::Source => "the folder to back up".to_string(),
                    Purpose::Destination => "the backup destination".to_string(),
                    Purpose::RestoreTo(_, p) => format!("where to restore {p}"),
                };
                out.push_str(&format!("Aftercare: choose {what}\n  {}\n\n", c.at.clone().unwrap_or_else(|| "Folders this run may use".into())));
                list(&mut out, c.rows.clone(), self.cursor, self.top, true);
                keys = "Enter open or use  Backspace up  N new folder  P ask Files for a folder  Esc back";
            }
            Screen::Snapshots => {
                out.push_str("Aftercare: snapshots (newest last)\n\n");
                let items = self
                    .snaps
                    .iter()
                    .map(|(id, s)| {
                        let n = s.summary.files_new + s.summary.files_changed + s.summary.files_unchanged;
                        format!("{} UTC  {}  {} files  {}  {}", settings::civil(s.time.secs), id.short(), n, settings::bytes(s.summary.bytes_total), s.paths.join(" "))
                    })
                    .collect();
                list(&mut out, items, self.cursor, self.top, true);
                keys = "Enter browse  Delete forget this one  Esc back";
            }
            Screen::Browse(i, path, nodes) => {
                let (id, s) = &self.snaps[*i];
                out.push_str(&format!("Aftercare: snapshot {} of {} UTC\n  {}\n\n", id.short(), settings::civil(s.time.secs), path));
                let items = nodes
                    .iter()
                    .map(|n| match n.kind {
                        NodeKind::Dir => format!("{}/", n.name),
                        _ => format!("{}  {}", n.name, settings::bytes(n.size)),
                    })
                    .collect();
                list(&mut out, items, self.cursor, self.top, true);
                keys = "Enter open  Backspace up  R restore to where it was  T restore to another folder  Esc back";
            }
            Screen::Report(title, lines) => {
                out.push_str(&format!("Aftercare: {title}\n\n"));
                list(&mut out, lines.clone(), self.cursor, self.top, false);
                keys = "Up/Down scroll  Esc back";
            }
        }
        out.push('\n');
        if let Some((p, text)) = &self.prompt {
            let shown = if p.secret() { "*".repeat(text.chars().count()) } else { text.clone() };
            if p.takes_text() {
                out.push_str(&format!("{}: {}_\n(Enter: done, Esc: cancel)\n\n", p.label(), shown));
            } else {
                out.push_str(&format!("{}\n\n", p.label()));
            }
        }
        if !self.status.is_empty() {
            out.push_str(&self.status);
            out.push_str("\n\n");
        }
        out.push_str(keys);
        self.window.show(out.as_bytes());
    }

    fn go(&mut self, screen: Screen) {
        self.screen = screen;
        self.cursor = 0;
        self.top = 0;
    }

    // ---- folders ------------------------------------------------------------------

    fn choose(&mut self, purpose: Purpose) {
        let rows = self.roots(&purpose);
        rec(&format!("[aftercare.choose] purpose={} roots={}", purpose_name(&purpose), rows.len()));
        self.go(Screen::Choose(Chooser { purpose, at: None, rows }));
    }

    fn roots(&self, purpose: &Purpose) -> Vec<String> {
        let mut rows = self.granted.clone();
        // The program's own folder can hold a repository or restored files, but it is
        // not a source: its snapshot paths would not say where the files came from.
        if *purpose != Purpose::Source {
            rows.push("/".into());
        }
        rows
    }

    fn open_folder(&mut self, at: String) {
        let Screen::Choose(c) = &mut self.screen else { return };
        match store::list(&at) {
            Ok(entries) => {
                c.rows = std::iter::once("[ Use this folder ]".to_string()).chain(entries.into_iter().filter(|e| e.dir).map(|e| format!("{}/", e.name))).collect();
                rec(&format!("[aftercare.folder] path={at} rows={}", c.rows.len()));
                c.at = Some(at);
                self.cursor = 0;
                self.top = 0;
                self.status.clear();
            }
            Err(e) => self.status = format!("Can't open {at}: {e}"),
        }
    }

    fn chose(&mut self, folder: String) {
        let Screen::Choose(c) = &self.screen else { return };
        let purpose = c.purpose.clone();
        rec(&format!("[aftercare.chose] purpose={} path={folder}", purpose_name(&purpose)));
        match purpose {
            Purpose::Source => {
                if !folder.starts_with("@/") {
                    self.status = "Back up a folder you gave Aftercare (run aftercare @/FOLDER).".into();
                    return;
                }
                self.settings.source = Some(folder.clone());
                self.status = format!("Source: {folder}");
                self.save();
            }
            Purpose::Destination => {
                if self.settings.destination.as_deref() != Some(&folder) {
                    self.repo = None;
                    self.snaps.clear();
                }
                self.status = if DirStore::exists(&folder) {
                    format!("Destination: {folder}. It holds a repository: U unlocks it.")
                } else {
                    format!("Destination: {folder}. U creates a repository there.")
                };
                self.settings.destination = Some(folder);
                self.save();
            }
            Purpose::RestoreTo(id, path) => {
                self.go(Screen::Home);
                self.restore(id, &path, &folder);
                return;
            }
        }
        self.go(Screen::Home);
    }

    fn pick_with_files(&mut self) {
        let mut out = [0u8; sex_std::sex_pdx::fs::PATH_BYTES];
        let picked = sex_app::files::pick(self.window, sex_app::files::Pick::Folder, b"", &mut out).map(|p| String::from_utf8_lossy(p).into_owned());
        match picked {
            Ok(p) => {
                let p = if p.starts_with('@') { p } else { format!("@{p}") };
                if !self.granted.contains(&p) {
                    self.granted.push(p.clone());
                }
                self.chose(p);
            }
            Err(_) => self.status = "Files gave no folder.".into(),
        }
    }

    // ---- repository ---------------------------------------------------------------

    fn unlock(&mut self) {
        let Some(dest) = self.settings.destination.clone() else {
            self.status = "Choose a destination first (D).".into();
            return;
        };
        if self.repo.is_some() {
            self.status = "The repository is already unlocked.".into();
            return;
        }
        if DirStore::exists(&dest) {
            self.prompt = Some((Prompt::Unlock, String::new()));
        } else if self.rng.is_none() {
            self.status = "This computer has no hardware random generator: Aftercare can't make keys here.".into();
        } else {
            if let Err(e) = std::fs::metadata(&dest) {
                self.status = format!("Can't use {dest}: {e}");
                return;
            }
            self.prompt = Some((Prompt::NewPassword, String::new()));
        }
    }

    fn opened(&mut self, repo: Repository<DirStore>) {
        let mut repo = repo;
        repo.set_host("sexos");
        self.repo = Some(repo);
        self.load_snapshots();
    }

    fn load_snapshots(&mut self) {
        let Some(repo) = self.repo.as_mut() else { return };
        match repo.snapshots() {
            Ok(mut v) => {
                v.sort_by_key(|(_, s)| (s.time.secs, s.time.nanos));
                rec(&format!("[aftercare.snapshots] count={}", v.len()));
                self.snaps = v;
            }
            Err(e) => self.status = format!("Can't read the snapshots: {e}"),
        }
    }

    fn submit(&mut self, prompt: Prompt, text: String) {
        match prompt {
            Prompt::Unlock => {
                let dest = self.settings.destination.clone().unwrap_or_default();
                self.window.show(b"Aftercare\n\nUnlocking...");
                let t = Instant::now();
                match Repository::open(DirStore::new(&dest), text.as_bytes()) {
                    Ok(repo) => {
                        rec(&format!("[aftercare.unlock] result=ok ms={}", t.elapsed().as_millis()));
                        self.status = "Unlocked.".into();
                        self.opened(repo);
                    }
                    Err(Error::WrongPassword) => {
                        rec(&format!("[aftercare.unlock] result=wrong-password ms={}", t.elapsed().as_millis()));
                        self.status = "Wrong password. Try again, or Esc.".into();
                        self.prompt = Some((Prompt::Unlock, String::new()));
                    }
                    Err(e) => {
                        rec(&format!("[aftercare.unlock] result=error error={e}"));
                        self.status = format!("Can't open the repository: {e}");
                    }
                }
            }
            Prompt::NewPassword => {
                if text.is_empty() {
                    self.status = "The password can't be empty.".into();
                    self.prompt = Some((Prompt::NewPassword, String::new()));
                } else {
                    self.prompt = Some((Prompt::ConfirmPassword(text), String::new()));
                }
            }
            Prompt::ConfirmPassword(first) => {
                if first != text {
                    self.status = "The passwords differ. Choose one again.".into();
                    self.prompt = Some((Prompt::NewPassword, String::new()));
                    return;
                }
                let dest = self.settings.destination.clone().unwrap_or_default();
                let Some(rng) = self.rng.as_mut() else { return };
                self.window.show(b"Aftercare\n\nCreating the repository...");
                let t = Instant::now();
                let opts = InitOptions { kdf: KDF, ..Default::default() };
                match Repository::init(DirStore::new(&dest), text.as_bytes(), rng, &opts) {
                    Ok(repo) => {
                        rec(&format!("[aftercare.repo.init] path={dest} kdf_kib={} kdf_passes={} ms={}", KDF.m_cost_kib, KDF.t_cost, t.elapsed().as_millis()));
                        self.status = format!("Created a repository in {dest}. Keep the password safe: without it the backups can't be read.");
                        self.opened(repo);
                    }
                    Err(e) => self.status = format!("Can't create the repository: {e}"),
                }
            }
            Prompt::Keep => match settings::parse_keep(&text) {
                Ok(p) => {
                    self.settings.keep = p;
                    rec(&format!("[aftercare.keep] policy={}", settings::format_keep(&self.settings.keep)));
                    self.status = format!("Keep: {}", settings::format_keep(&self.settings.keep));
                    self.save();
                }
                Err(e) => {
                    self.status = e;
                    self.prompt = Some((Prompt::Keep, text));
                }
            },
            Prompt::Schedule => match settings::parse_time(&text) {
                Ok(t) => {
                    self.settings.schedule = t;
                    rec(&format!("[aftercare.schedule] at={}", settings::format_time(t)));
                    self.status = format!("Daily backup: {}", settings::format_time(t));
                    self.save();
                }
                Err(e) => {
                    self.status = e;
                    self.prompt = Some((Prompt::Schedule, text));
                }
            },
            Prompt::NewFolder => {
                let Screen::Choose(c) = &self.screen else { return };
                let Some(at) = c.at.clone() else { return };
                if text.is_empty() || text.contains('/') || text == "." || text == ".." {
                    self.status = "Not a folder name.".into();
                    return;
                }
                let p = store::join(&at, &text);
                match std::fs::create_dir(&p) {
                    Ok(()) => {
                        rec(&format!("[aftercare.mkdir] path={p}"));
                        self.open_folder(at);
                    }
                    Err(e) => self.status = format!("Can't make {p}: {e}"),
                }
            }
            Prompt::ConfirmForget(_) => self.forget_and_prune(),
            Prompt::ConfirmDelete(id) => {
                let (Some(repo), Some(rng)) = (self.repo.as_mut(), self.rng.as_mut()) else { return };
                match repo.forget_snapshots(&[id], now(), rng) {
                    Ok(()) => {
                        rec(&format!("[aftercare.forget] snapshot={}", id.short()));
                        self.status = format!("Forgot snapshot {}. F prunes data no snapshot uses.", id.short());
                    }
                    Err(e) => self.status = format!("Can't forget it: {e}"),
                }
                self.load_snapshots();
                self.cursor = self.cursor.min(self.snaps.len().saturating_sub(1));
            }
        }
    }

    fn need_repo(&mut self) -> bool {
        if self.repo.is_none() {
            self.status = "Unlock the repository first (U).".into();
            return false;
        }
        true
    }

    fn backup(&mut self, scheduled: bool) {
        if !self.need_repo() {
            return;
        }
        let Some(source) = self.settings.source.clone() else {
            self.status = "Choose a source folder first (S).".into();
            return;
        };
        let Some(rng) = self.rng.as_mut() else {
            self.status = "No hardware random generator.".into();
            return;
        };
        let repo = self.repo.as_mut().unwrap();
        let path = source[1..].to_string();
        let opts = aftercare_core::BackupOptions {
            host: "sexos".into(),
            tags: if scheduled { vec!["scheduled".into()] } else { vec![] },
            time: now(),
            parent: aftercare_core::ParentChoice::Latest,
        };
        let nonce = rand_core::RngCore::next_u32(rng);
        let mut watch = Watch::new(self.window, "backing up");
        let started = Instant::now();
        let r = repo.backup(&mut DirSource::new("@", nonce), &[path.as_str()], &opts, rng, &mut watch);
        let closed = watch.closed;
        let ms = started.elapsed().as_millis();
        match r {
            Ok((id, snap)) => {
                let s = &snap.summary;
                rec(&format!(
                    "[aftercare.backup.done] snapshot={} files_new={} files_changed={} files_unchanged={} dirs={} bytes={} added={} errors={} scheduled={} ms={ms}",
                    id.short(), s.files_new, s.files_changed, s.files_unchanged, s.dirs, s.bytes_total, s.bytes_added, s.errors, scheduled as u8
                ));
                self.status = format!(
                    "Backed up {} files ({}): {} new, {} changed, {} stored. Snapshot {}.{}",
                    s.files_new + s.files_changed + s.files_unchanged,
                    settings::bytes(s.bytes_total),
                    s.files_new,
                    s.files_changed,
                    settings::bytes(s.bytes_stored),
                    id.short(),
                    if s.errors > 0 { format!(" {} could not be read.", s.errors) } else { String::new() }
                );
            }
            Err(Error::Cancelled) => {
                rec(&format!("[aftercare.backup.cancelled] ms={ms}"));
                self.status = "Backup cancelled. What was already stored is reused by the next one.".into();
            }
            Err(e) => {
                rec(&format!("[aftercare.backup.failed] error={e}"));
                self.status = match e {
                    Error::Locked(_) => format!("{e}. If no other backup is running, X clears stale locks."),
                    e => format!("Backup failed: {e}"),
                };
            }
        }
        self.quit |= closed;
        self.load_snapshots();
    }

    fn restore(&mut self, id: Id, path: &str, target: &str) {
        let Some(repo) = self.repo.as_mut() else { return };
        let mut sink = DirSink::new(target);
        let mut watch = Watch::new(self.window, "restoring");
        let started = Instant::now();
        let r = repo.restore(&id, path, &mut sink, &mut watch);
        self.quit |= watch.closed;
        for w in &sink.written {
            rec(&format!("[aftercare.restore.file] path={} bytes={} sha256={}", w.path, w.bytes, w.sha256));
        }
        match r {
            Ok(s) => {
                rec(&format!("[aftercare.restore.done] path={path} into={target} files={} dirs={} bytes={} ms={}", s.files, s.dirs, s.bytes, started.elapsed().as_millis()));
                self.status = format!("Restored {path} into {target}: {} files, {} (every chunk verified).", s.files, settings::bytes(s.bytes));
            }
            Err(Error::Cancelled) => {
                rec("[aftercare.restore.cancelled]");
                self.status = format!("Restore cancelled after {} files.", sink.written.len());
            }
            Err(e) => {
                rec(&format!("[aftercare.restore.failed] error={e}"));
                self.status = format!("Restore failed: {e}");
            }
        }
    }

    fn forget_and_prune(&mut self) {
        let (Some(repo), Some(rng)) = (self.repo.as_mut(), self.rng.as_mut()) else { return };
        self.window.show(b"Aftercare\n\nForgetting and pruning...");
        let t = now();
        let plan = match repo.forget(&self.settings.keep, t, rng) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Forget failed: {e}");
                return;
            }
        };
        rec(&format!("[aftercare.forget] kept={} removed={}", plan.keep.len(), plan.remove.len()));
        match repo.prune(&PruneOptions { now: t, ..Default::default() }, rng) {
            Ok(p) => {
                rec(&format!(
                    "[aftercare.prune] packs_deleted={} packs_repacked={} blobs_removed={} bytes_before={} bytes_after={}",
                    p.packs_deleted, p.packs_repacked, p.blobs_removed, p.bytes_before, p.bytes_after
                ));
                self.status = format!(
                    "Forgot {} snapshot(s), kept {}. Pruned {} unused pieces: {} -> {}.",
                    plan.remove.len(),
                    plan.keep.len(),
                    p.blobs_removed,
                    settings::bytes(p.bytes_before),
                    settings::bytes(p.bytes_after)
                );
            }
            Err(e) => self.status = format!("Forgot {} snapshot(s); prune failed: {e}", plan.remove.len()),
        }
        self.load_snapshots();
    }

    fn check(&mut self, read_data: bool) {
        let Some(repo) = self.repo.as_mut() else { return };
        self.window.show(if read_data { b"Aftercare\n\nChecking, reading all data..." } else { b"Aftercare\n\nChecking..." });
        let t = Instant::now();
        match repo.check(read_data) {
            Ok(r) => {
                rec(&format!(
                    "[aftercare.check] errors={} warnings={} snapshots={} trees={} packs={} blobs_verified={} read_data={} ms={}",
                    r.errors.len(), r.warnings.len(), r.snapshots, r.trees, r.packs, r.blobs_verified, read_data as u8, t.elapsed().as_millis()
                ));
                let mut lines = vec![format!(
                    "{}: {} snapshots, {} folders, {} packs{}",
                    if r.is_ok() { "No errors" } else { "ERRORS FOUND" },
                    r.snapshots,
                    r.trees,
                    r.packs,
                    if read_data { format!(", {} pieces read and verified", r.blobs_verified) } else { String::new() }
                )];
                lines.extend(r.errors.iter().map(|e| format!("error: {e}")));
                lines.extend(r.warnings.iter().map(|w| format!("warning: {w}")));
                self.go(Screen::Report("check".into(), lines));
            }
            Err(e) => self.status = format!("Check failed: {e}"),
        }
    }

    fn browse(&mut self, i: usize, path: String) {
        let id = self.snaps[i].0;
        let Some(repo) = self.repo.as_mut() else { return };
        match repo.ls(&id, &path) {
            Ok(nodes) => {
                rec(&format!("[aftercare.browse] snapshot={} path={path} rows={}", id.short(), nodes.len()));
                self.go(Screen::Browse(i, path, nodes));
            }
            Err(e) => self.status = format!("Can't open {path}: {e}"),
        }
    }

    /// The daily backup, while the app is open (a background scheduler is a later slice).
    fn tick(&mut self) -> bool {
        let t = now().secs;
        if self.prompt.is_some() || !self.settings.due(t) {
            return false;
        }
        let day = t.div_euclid(86_400);
        if self.repo.is_none() || self.settings.source.is_none() {
            if self.due_noticed != Some(day) {
                self.due_noticed = Some(day);
                rec("[aftercare.schedule.due] unlocked=0");
                self.status = "The daily backup is due: unlock the repository (U) and it starts.".into();
                return true;
            }
            return false;
        }
        self.settings.last_scheduled_day = Some(day);
        self.save();
        rec("[aftercare.schedule.run]");
        self.go(Screen::Home);
        self.backup(true);
        true
    }

    // ---- keys -----------------------------------------------------------------------

    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        if let Some((prompt, mut text)) = self.prompt.take() {
            match key.code {
                ESC => {
                    self.status = "Cancelled.".into();
                    return;
                }
                ENTER => {
                    self.status.clear();
                    return self.submit(prompt, text);
                }
                BACKSPACE => {
                    text.pop();
                }
                _ => {
                    if let Some(c) = sex_app::text(key).filter(|c| !c.is_control()) {
                        if prompt.takes_text() && text.len() < 200 {
                            text.push(c);
                        }
                    }
                }
            }
            rec(&format!("[aftercare.input] len={}", text.chars().count()));
            self.prompt = Some((prompt, text));
            return;
        }
        let rows = self.rows();
        match sex_app::nav(key) {
            Some(Nav::Down) => {
                self.cursor = (self.cursor + 1).min(rows.saturating_sub(1));
                if self.cursor >= self.top + ROWS {
                    self.top = self.cursor + 1 - ROWS;
                }
                rec(&format!("[aftercare.cursor] row={}", self.cursor));
                return;
            }
            Some(Nav::Up) => {
                self.cursor = self.cursor.saturating_sub(1);
                self.top = self.top.min(self.cursor);
                rec(&format!("[aftercare.cursor] row={}", self.cursor));
                return;
            }
            Some(Nav::Home) => {
                self.cursor = 0;
                self.top = 0;
                return;
            }
            Some(Nav::End) => {
                self.cursor = rows.saturating_sub(1);
                self.top = self.cursor.saturating_sub(ROWS - 1);
                return;
            }
            Some(Nav::Delete) => {
                if let Screen::Snapshots = self.screen {
                    if let Some((id, _)) = self.snaps.get(self.cursor) {
                        self.prompt = Some((Prompt::ConfirmDelete(*id), String::new()));
                    }
                }
                return;
            }
            _ => {}
        }
        let ch = sex_app::typed(key).map(|c| c.to_ascii_lowercase());
        match &self.screen {
            Screen::Home => match ch {
                Some(b's') => self.choose(Purpose::Source),
                Some(b'd') => self.choose(Purpose::Destination),
                Some(b'u') => self.unlock(),
                Some(b'b') => self.backup(false),
                Some(b'l') => {
                    if self.need_repo() {
                        self.load_snapshots();
                        let last = self.snaps.len().saturating_sub(1);
                        self.go(Screen::Snapshots);
                        self.cursor = last;
                        self.top = last.saturating_sub(ROWS - 1);
                    }
                }
                Some(b'k') => self.prompt = Some((Prompt::Keep, settings::format_keep(&self.settings.keep))),
                Some(b't') => self.prompt = Some((Prompt::Schedule, settings::format_time(self.settings.schedule))),
                Some(b'f') => {
                    if self.need_repo() {
                        match self.repo.as_mut().unwrap().plan_forget(&self.settings.keep) {
                            Ok(plan) => self.prompt = Some((Prompt::ConfirmForget(plan), String::new())),
                            Err(e) => self.status = format!("Can't plan: {e}"),
                        }
                    }
                }
                Some(b'c') => {
                    if self.need_repo() {
                        self.check(key.shift);
                    }
                }
                Some(b'x') => {
                    if self.need_repo() {
                        match self.repo.as_mut().unwrap().remove_locks(None) {
                            Ok(n) => {
                                rec(&format!("[aftercare.locks.removed] count={n}"));
                                self.status = format!("Removed {n} lock(s).");
                            }
                            Err(e) => self.status = format!("Can't remove locks: {e}"),
                        }
                    }
                }
                _ => {}
            },
            Screen::Choose(c) => {
                let (at, row) = (c.at.clone(), c.rows.get(self.cursor).cloned());
                let purpose = c.purpose.clone();
                match (key.code, ch) {
                    (ESC, _) => {
                        self.status.clear();
                        self.go(Screen::Home)
                    }
                    (ENTER, _) => match (at, row) {
                        (None, Some(root)) => self.open_folder(root),
                        (Some(at), Some(_)) if self.cursor == 0 => self.chose(at),
                        (Some(at), Some(row)) => self.open_folder(store::join(&at, row.trim_end_matches('/'))),
                        _ => {}
                    },
                    (BACKSPACE, _) => match at {
                        Some(at) if self.roots(&purpose).contains(&at) => {
                            let rows = self.roots(&purpose);
                            if let Screen::Choose(c) = &mut self.screen {
                                c.at = None;
                                c.rows = rows;
                            }
                            self.cursor = 0;
                            self.top = 0;
                        }
                        Some(at) => self.open_folder(store::parent(&at)),
                        None => {}
                    },
                    (_, Some(b'n')) if at.is_some() => self.prompt = Some((Prompt::NewFolder, String::new())),
                    (_, Some(b'p')) => self.pick_with_files(),
                    _ => {}
                }
            }
            Screen::Snapshots => match key.code {
                ESC => self.go(Screen::Home),
                ENTER if self.cursor < self.snaps.len() => self.browse(self.cursor, "/".into()),
                _ => {}
            },
            Screen::Browse(i, path, nodes) => {
                let (i, path, node) = (*i, path.clone(), nodes.get(self.cursor).cloned());
                let id = self.snaps[i].0;
                match (key.code, ch) {
                    (ESC, _) => {
                        let last = i;
                        self.go(Screen::Snapshots);
                        self.cursor = last;
                    }
                    (ENTER, _) => {
                        if let Some(n) = node.filter(|n| n.kind == NodeKind::Dir) {
                            self.browse(i, store::join(&path, &n.name));
                        }
                    }
                    (BACKSPACE, _) => {
                        if path == "/" {
                            self.go(Screen::Snapshots);
                            self.cursor = i;
                        } else {
                            self.browse(i, store::parent(&path));
                        }
                    }
                    (_, Some(b'r')) => {
                        if let Some(n) = node {
                            let target = format!("@{path}");
                            self.restore(id, &store::join(&path, &n.name), &target);
                        }
                    }
                    (_, Some(b't')) => {
                        if let Some(n) = node {
                            self.choose(Purpose::RestoreTo(id, store::join(&path, &n.name)));
                        }
                    }
                    _ => {}
                }
            }
            Screen::Report(..) => {
                if key.code == ESC {
                    self.go(Screen::Home);
                }
            }
        }
    }
}

fn purpose_name(p: &Purpose) -> &'static str {
    match p {
        Purpose::Source => "source",
        Purpose::Destination => "destination",
        Purpose::RestoreTo(..) => "restore",
    }
}

pub fn main() {
    let granted: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a.starts_with("@/"))
        .map(|a| if a == "@/" { a } else { a.trim_end_matches('/').to_string() })
        .collect();
    let settings = std::fs::read_to_string(settings::FILE).map(|t| Settings::parse(&t)).unwrap_or_default();
    let window = sex_std::window();
    window.title(b"Aftercare");
    let (target, task) = sex_std::identity();
    let rng = HwRng::new();
    rec(&format!(
        "[aftercare.ready] target={target} task={task} granted={} source={} destination={} rng={}",
        granted.join(","),
        settings.source.as_deref().unwrap_or("-"),
        settings.destination.as_deref().unwrap_or("-"),
        rng.is_some() as u8
    ));
    let mut app = App {
        window,
        settings,
        granted,
        repo: None,
        rng,
        screen: Screen::Home,
        prompt: None,
        status: String::new(),
        cursor: 0,
        top: 0,
        snaps: Vec::new(),
        due_noticed: None,
        quit: false,
        asked: None,
    };
    if app.granted.is_empty() {
        app.status = "Give Aftercare the folders it may use when starting it, for example:\n  run aftercare @/home/ann @/media/USB".into();
    } else if let Some(d) = app.settings.destination.clone() {
        if DirStore::exists(&d) {
            app.status = format!("{d} holds a repository: U unlocks it.");
        }
    }
    app.draw();
    loop {
        let changed = match app.window.next_within(1000) {
            Some(Event::Close) => break,
            Some(Event::Key(k)) => {
                app.key(k);
                true
            }
            Some(Event::Resize { .. }) => true,
            _ => app.tick(),
        };
        if app.quit {
            break;
        }
        if changed {
            app.draw();
        }
    }
    rec("[aftercare.closed]");
    std::process::exit(0);
}
