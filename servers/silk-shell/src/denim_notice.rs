// SPDX-License-Identifier: GPL-3.0-only
//! Denim (compressed memory) started again, so the programs whose memory it held packed
//! had to end: the kernel names each (`sex_pdx::denim::OP_LOST`), and the Shell says so
//! in a System notice, as it does for a program ended when memory runs out.

/// Whether `msg` was the kernel's word that a program run lost its compressed memory.
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if msg.type_id != sex_pdx::denim::OP_LOST || msg.caller_pd != 0 { return false; }
    let target = msg.arg0 as u32;
    let name = sex_pdx::system_stats::name(target);
    let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
    let name = if name.is_empty() { &b"A program"[..] } else { name };
    let mut text = [0u8; sex_pdx::notify::TEXT_MAX];
    let mut k = 0;
    for &b in name.iter().chain(b" was closed: its compressed memory was lost\nDenim, which kept part of its memory packed, had to start again.".iter()) {
        if k < text.len() { text[k] = b; k += 1; }
    }
    crate::shell_notice(&text[..k], 2);
    sex_pdx::serial_record!("[shell.denim.lost] target={} task={} pages={}", target, msg.arg1, msg.arg2);
    true
}
