//! `update [URL]`: downloads a release's signed index and signature, asks Files whether it
//! is good and newer, then downloads the image and has Files write it into the other
//! system slot. URL is the folder holding the release files; by default the project's
//! latest GitHub release. Driven one step at a time from the main loop, like `fetch`.
use crate::fetch_command::FETCH;
use crate::fs_commands::{FS, UPDATE_STATUS};
use crate::Scrollback;
use sex_pdx::fs as w;

pub const DEFAULT_BASE: &[u8] = b"https://github.com/xirtus/sexos/releases/latest/download/";
const NAMES: [&[u8]; 3] = [b"sexos-update.txt", b"sexos-update.sig", b"sexos-boot.img"];
const FILES: [&[u8]; 3] = [b"/sexos-update.txt", b"/sexos-update.sig", b"/sexos-boot.img"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage { Idle, Index, Sig, Check, Image, Apply }
pub struct Update { stage: Stage, base: [u8; 400], base_len: usize, asked: bool }
pub static mut UPDATE: Update = Update { stage: Stage::Idle, base: [0; 400], base_len: 0, asked: false };

impl Update {
    pub fn busy(&self) -> bool { self.stage != Stage::Idle }
    pub fn start(&mut self, args: &[u8], sb: &mut Scrollback) {
        if self.busy() { sb.push(b"An update is already running."); return; }
        let url = args.trim_ascii();
        let url = if url.is_empty() { DEFAULT_BASE } else { url };
        if !(url.starts_with(b"http://") || url.starts_with(b"https://")) || url.len() + 2 > self.base.len() { sb.push(b"Usage: update [URL of the release folder]"); return; }
        self.base[..url.len()].copy_from_slice(url);
        self.base_len = url.len();
        if !url.ends_with(b"/") { self.base[self.base_len] = b'/'; self.base_len += 1; }
        sex_pdx::serial_record!("[spindle.update.start] base={}", core::str::from_utf8(&self.base[..self.base_len]).unwrap_or("?"));
        sb.push(b"Looking for an update...");
        self.fetch(0, sb);
    }
    fn fetch(&mut self, which: usize, sb: &mut Scrollback) {
        let mut line = [0u8; 600];
        let (b, n, f) = (&self.base[..self.base_len], NAMES[which], FILES[which]);
        let mut at = 0;
        for part in [b, n, b" ", f] { line[at..at + part.len()].copy_from_slice(part); at += part.len(); }
        self.stage = [Stage::Index, Stage::Sig, Stage::Image][which];
        if which == 2 { sb.push(b"Downloading the system image. It takes a while; the terminal keeps working."); }
        unsafe { FETCH.start(&line[..at], sb, |p, o| { let k = p.len(); if k > o.len() { return None; } o[..k].copy_from_slice(p); Some(k) }); }
        if unsafe { !FETCH.busy() } { self.stop(sb, "the download could not start"); }
    }
    fn ask(&mut self, apply: bool, sb: &mut Scrollback) -> bool {
        let paths: &[&[u8]] = if apply { &FILES } else { &FILES[..2] };
        unsafe { FS.start_update(paths, sb) }
    }
    fn stop(&mut self, sb: &mut Scrollback, why: &str) {
        let mut t = [0u8; 120];
        let msg = b"update: ";
        t[..msg.len()].copy_from_slice(msg);
        let n = why.len().min(t.len() - msg.len());
        t[msg.len()..msg.len() + n].copy_from_slice(&why.as_bytes()[..n]);
        sb.push(&t[..msg.len() + n]);
        sex_pdx::serial_record!("[spindle.update.stopped] {}", why);
        self.stage = Stage::Idle;
    }
    /// Moves on when the last step finished. True when it printed something.
    pub fn step(&mut self, sb: &mut Scrollback) -> bool {
        match self.stage {
            Stage::Idle => false,
            Stage::Index | Stage::Sig | Stage::Image => {
                if unsafe { FETCH.busy() } { return false; }
                if unsafe { FETCH.result } != Some(true) { self.stop(sb, "a download failed"); return true; }
                match self.stage {
                    Stage::Index => self.fetch(1, sb),
                    Stage::Sig => { self.stage = Stage::Check; self.asked = false; }
                    _ => { self.stage = Stage::Apply; self.asked = false; }
                }
                true
            }
            Stage::Check | Stage::Apply => {
                if !self.asked {
                    if unsafe { FS.busy() } { return false; }
                    self.asked = self.ask(self.stage == Stage::Apply, sb);
                    if self.asked && self.stage == Stage::Apply { sb.push(b"Checking and writing the update. Don't turn the machine off."); }
                    return self.asked;
                }
                let Some(status) = (unsafe { UPDATE_STATUS }) else { return false };
                unsafe { UPDATE_STATUS = None; }
                match (self.stage, status) {
                    (Stage::Check, w::OK) => self.fetch(2, sb),
                    (Stage::Apply, w::OK) => { sex_pdx::serial_record!("[spindle.update.done]"); self.stage = Stage::Idle; }
                    _ => self.stage = Stage::Idle,
                }
                true
            }
        }
    }
}
