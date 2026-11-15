//! Private read-only lifecycle observations. This supplies no authority and must
//! never turn an unknown/pre-Started job into a successful retirement.
use crate::{mailbox, SHELL};
use sex_pdx::{restricted_helper::Subject, restricted_job as w, PdxMessage};
pub fn is_event(m: &PdxMessage) -> bool {
    m.caller_pd == SHELL && m.caller_task == SHELL && m.type_id == w::OP_STATUS_EVENT
}
/// Query after observing the current live Shell epoch through the held route.
/// `known` must be the child's identity from this job's authenticated Started.
pub fn query(cookie: u32, known: Option<Subject>, shell_epoch: u64) -> bool {
    let Some(words) = w::status_words(cookie, known, shell_epoch) else {
        return false;
    };
    sex_pdx::pdx_call(
        sex_pdx::SLOT_SHELL,
        w::OP_STATUS,
        words[0],
        words[1],
        words[2],
    )
    .0 == 0
}
/// No wait; Window preserves this namespace. Foreign/stale/malformed events do
/// not match. Caller rechecks the held live peer epoch before relying on it.
pub fn poll(cookie: u32, shell_epoch: u64) -> Option<w::Observation> {
    let mine = |m: &PdxMessage| {
        is_event(m)
            && w::Observation::decode([m.arg0, m.arg1, m.arg2])
                .is_some_and(|o| o.cookie == cookie && o.shell_epoch == shell_epoch)
    };
    while let Some(m) = mailbox::take(|m| {
        is_event(m)
            && w::Observation::decode([m.arg0, m.arg1, m.arg2]).is_none_or(|o| o.cookie == cookie)
    }) {
        if mine(&m) {
            return w::Observation::decode([m.arg0, m.arg1, m.arg2]);
        }
    }
    while let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
        if mine(&m) {
            return w::Observation::decode([m.arg0, m.arg1, m.arg2]);
        }
        // Private status from an old epoch is stale, not a future UI event.
        if !is_event(&m)
            || w::Observation::decode([m.arg0, m.arg1, m.arg2]).is_some_and(|o| o.cookie != cookie)
        {
            mailbox::put(m);
        }
    }
    None
}
