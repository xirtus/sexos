// SPDX-License-Identifier: GPL-3.0-only
//! Bell receipt for due notices. Ordinary notifications keep their one-way API.
use core::sync::atomic::{AtomicU32, Ordering};
use sex_pdx::notify as n;

static NEXT_TICKET: AtomicU32 = AtomicU32::new(1);

/// Bell's disposition of one due notice after Shell has handled it. A timeout
/// is uncertain: Shell may have queued it and the caller may retry.
pub fn post_confirmed(summary: &[u8], body: &[u8], urgency: u8) -> n::Disposition {
    let Some((text, len)) = n::compose(summary, body) else { return n::Disposition::Retry };
    let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
    for (head, a, b) in n::chunks(&text[..len], urgency, ticket as u16, (ticket >> 16) as u16) {
        if sex_pdx::pdx_call(sex_pdx::SLOT_BELL, n::OP_BELL_POST_CONFIRMED, head, a, b).0 != 0 {
            return n::Disposition::Retry;
        }
    }
    let wanted = |m: &sex_pdx::PdxMessage| {
        m.type_id == 1 && m.caller_pd == n::BELL_PD
            && n::decode_receipt(m.arg0).is_some_and(|(for_ticket, _)| for_ticket == ticket)
    };
    let deadline = sex_pdx::get_ticks().saturating_add(2_000);
    loop {
        if let Some(m) = crate::mailbox::take(wanted).or_else(|| sex_pdx::pdx_try_listen_raw(0)) {
            if wanted(&m) {
                return n::decode_receipt(m.arg0).map_or(n::Disposition::Retry, |(_, disposition)| disposition);
            }
            crate::mailbox::put(m);
        }
        if sex_pdx::get_ticks() >= deadline { return n::Disposition::Retry; }
        let _ = sex_pdx::sys_wait_self_ticks(4);
    }
}
