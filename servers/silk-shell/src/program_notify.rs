// SPDX-License-Identifier: GPL-3.0-only
//! Notifications per program (Settings' Notifications page, `notifications.hidden`).
//!
//! When a program row starts a run, the Shell asks Files which program it is
//! (`sex_pdx::config::PROGRAM`): Files knows the folder it bound the run to, so the
//! answer never rests on a name the program gives itself. A notification Bell forwards
//! from a row whose program the person set to hidden is kept out of the panel and the
//! history. Until Files answers, the row's notifications show.
use sex_config::id;

const ROWS: usize = sex_pdx::app_catalog::COUNT;
/// (row, place in `sex_config::PROGRAMS` or -1) for each row Files answered about.
static mut PROGRAMS: [(u32, i32); ROWS] = [(0, -1); ROWS];

fn table() -> &'static mut [(u32, i32); ROWS] { unsafe { &mut *core::ptr::addr_of_mut!(PROGRAMS) } }
fn slot(row: u32) -> Option<usize> { sex_pdx::app_catalog::INCARNATIONS.iter().position(|i| i.target == row) }

/// The Shell bound `row` to a new run: forget the old run's program and ask Files.
pub fn bound(row: u32) {
    let Some(i) = slot(row) else { return };
    table()[i] = (row, -1);
    crate::config_watch::ask_program(row);
}

/// Files' answer: the run in `row` is program `index` (-1: none on the list).
pub fn learned(row: u32, index: i32) {
    let Some(i) = slot(row) else { return };
    table()[i] = (row, index);
    sex_pdx::serial_record!("[shell.notify.program] row={} program={} hidden={}", row, index, hidden(row as u16) as u8);
}

/// True when the person hid notifications from the program running in `sender`'s row.
pub fn hidden(sender: u16) -> bool {
    let Some(i) = slot(sender as u32) else { return false };
    let (row, index) = table()[i];
    row == sender as u32 && index >= 0 && crate::config_watch::holds(id::NOTIFY_HIDDEN, index as usize)
}

/// Program folder name in `sex_config::PROGRAMS` once Files has answered for `row`.
pub fn name(row: u32) -> Option<&'static str> {
    let i = slot(row)?;
    let (r, index) = table()[i];
    if r != row || index < 0 {
        return None;
    }
    sex_config::PROGRAMS.get(index as usize).copied()
}
