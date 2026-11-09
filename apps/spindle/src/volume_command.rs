// SPDX-License-Identifier: GPL-3.0-only
//! `volume`: the sound mixer Bell keeps (`sex_pdx::sound_mixer`).
//!
//! `volume` shows the master volume; `volume 40`, `volume +5`, `volume -10`, `volume mute`
//! and `volume unmute` change it. `volume list` shows each program's stream; `volume ROW
//! 30`, `volume ROW mute` and `volume ROW unmute` set one program's volume.
//! Bell answers each request with a reply message, which `receive` turns into a line.
use sex_pdx::sound_mixer as mixer;

/// Bell's PD.
pub const BELL_PD: u32 = 10;
const ROWS: u64 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting { Nothing, Master, Row(u64), List { next: u64, shown: u8 } }

pub struct Volume { waiting: Waiting }
pub static mut VOLUME: Volume = Volume { waiting: Waiting::Nothing };

/// What to ask Bell for a command line's words, and what its reply will be.
fn request(args: &[u8]) -> Result<(u64, u64, u64, Waiting), &'static [u8]> {
    let mut words = args.split(|&b| b == b' ').filter(|w| !w.is_empty());
    let first = words.next();
    let second = words.next();
    let number = |w: &[u8]| core::str::from_utf8(w).ok().and_then(|t| t.parse::<i64>().ok());
    Ok(match (first, second) {
        (None, _) => (mixer::STATE, 0, 0, Waiting::Master),
        (Some(b"mute"), None) => (mixer::MUTE_MASTER, 1, 0, Waiting::Master),
        (Some(b"unmute"), None) => (mixer::MUTE_MASTER, 0, 0, Waiting::Master),
        (Some(b"list"), None) => (mixer::STREAM, 0, 0, Waiting::List { next: 0, shown: 0 }),
        (Some(w), None) if w[0] == b'+' || w[0] == b'-' => {
            let step = number(w).filter(|n| (-100..=100).contains(n)).ok_or(USAGE)?;
            (mixer::STEP_MASTER, step as u64, 0, Waiting::Master)
        }
        (Some(w), None) => {
            let v = number(w).filter(|n| (0..=100).contains(n)).ok_or(USAGE)?;
            (mixer::SET_MASTER, v as u64, 0, Waiting::Master)
        }
        (Some(row), Some(what)) => {
            let row = number(row).filter(|n| (0..ROWS as i64).contains(n)).ok_or(USAGE)? as u64;
            let setting = match what {
                b"mute" => 100 | mixer::MUTED,
                b"unmute" => 100,
                w => number(w).filter(|n| (0..=100).contains(n)).ok_or(USAGE)? as u64,
            };
            (mixer::SET_STREAM, row, setting, Waiting::Row(row))
        }
    })
}

const USAGE: &[u8] = b"Use: volume [0-100 | +N | -N | mute | unmute | list | ROW 0-100 | ROW mute | ROW unmute]";

fn ask(action: u64, a: u64, b: u64) -> bool {
    let (status, _) = sex_pdx::pdx_call(sex_pdx::SLOT_BELL, mixer::OP_MIXER, action, a, b);
    status == 0
}

fn percent(out: &mut [u8], at: &mut usize, n: u64) {
    let mut digits = [0u8; 20];
    let mut len = 0;
    let mut v = n;
    loop { digits[len] = b'0' + (v % 10) as u8; len += 1; v /= 10; if v == 0 { break; } }
    for i in (0..len).rev() { if *at < out.len() { out[*at] = digits[i]; *at += 1; } }
}

fn put(out: &mut [u8], at: &mut usize, text: &[u8]) {
    for &b in text { if *at < out.len() { out[*at] = b; *at += 1; } }
}

impl Volume {
    /// Starts a `volume` command; the answer arrives through `receive`.
    pub fn start(&mut self, args: &[u8], sb: &mut crate::Scrollback) {
        match request(args) {
            Err(usage) => { sb.push(usage); }
            Ok((action, a, b, waiting)) => {
                if ask(action, a, b) { self.waiting = waiting; } else { sb.push(b"Volume: the Bell service is unreachable."); }
            }
        }
    }

    /// A reply from Bell. False when it was not for a `volume` command.
    pub fn receive(&mut self, caller: u32, value: u64, sb: &mut crate::Scrollback) -> bool {
        if caller != BELL_PD || self.waiting == Waiting::Nothing { return false; }
        let waiting = core::mem::replace(&mut self.waiting, Waiting::Nothing);
        if value == mixer::REFUSED { sb.push(b"Volume: Bell refused that."); return true; }
        let mut line = [0u8; 96];
        let mut at = 0;
        match waiting {
            Waiting::Nothing => return false,
            Waiting::Master => {
                put(&mut line, &mut at, b"Volume ");
                percent(&mut line, &mut at, mixer::state_volume(value) as u64);
                put(&mut line, &mut at, b"%");
                if mixer::state_muted(value) { put(&mut line, &mut at, b", muted"); }
                if !mixer::state_card(value) { put(&mut line, &mut at, b" (no sound card)"); }
            }
            Waiting::Row(row) | Waiting::List { next: row, .. } => {
                let st = mixer::Stream::of(value);
                let mut shown = 0;
                if let Waiting::List { shown: s, .. } = waiting { shown = s; }
                let listing = matches!(waiting, Waiting::List { .. });
                if !listing || st.pd != 0 {
                    put(&mut line, &mut at, b"Row ");
                    percent(&mut line, &mut at, row);
                    put(&mut line, &mut at, b": program ");
                    percent(&mut line, &mut at, st.pd as u64);
                    put(&mut line, &mut at, b", volume ");
                    percent(&mut line, &mut at, st.volume as u64);
                    put(&mut line, &mut at, b"%");
                    if st.muted { put(&mut line, &mut at, b" muted"); }
                    put(&mut line, &mut at, if st.playing { b", playing at " } else { b", quiet, asks " });
                    percent(&mut line, &mut at, st.asked as u64);
                    put(&mut line, &mut at, b"%");
                    shown += 1;
                }
                if listing {
                    if at > 0 { sb.push(&line[..at]); }
                    if row + 1 < ROWS && ask(mixer::STREAM, row + 1, 0) {
                        self.waiting = Waiting::List { next: row + 1, shown };
                    } else if shown == 0 {
                        sb.push(b"No program has played sound yet.");
                    }
                    return true;
                }
            }
        }
        sb.push(&line[..at]);
        sex_pdx::serial_record!("[spindle.volume] reply={:#x}", value);
        true
    }
}
