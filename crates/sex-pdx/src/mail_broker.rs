// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Authority check for the mail broker carried by SexNet's existing web route.
//! The kernel stamps the caller's domain and task. SexNet first checks that
//! stamp against the live app lifecycle, then checks the Shell's current run
//! binding here. A request word, account ID or shared-region byte is never
//! allowed to name its own owner.

use crate::app_catalog::{self, App};
use crate::web as w;

/// `bound_task` comes only from SexNet's Shell-authenticated bind table.
pub fn may_request(caller_pd: u32, caller_task: u32, bound_task: Option<u32>) -> bool {
    if caller_task == 0 { return false; }
    if caller_pd == 3 { return caller_task == 3 && bound_task.is_none(); }
    app_catalog::by_target(caller_pd).is_some_and(|row| row.app == App::Program)
        && bound_task == Some(caller_task)
}

/// Reply to the reserved broker kinds until the account service is installed.
/// This accepts only kernel-stamped caller identity and a Shell-owned binding,
/// and never reads the shared region. Thus a SUBMIT cannot be mistaken for a
/// completed send while the verified dialer is absent.
pub fn unavailable_reply(caller_pd: u32, caller_task: u32, bound_task: Option<u32>, a: u64, value: u64) -> Option<u64> {
    let kind = (a >> 32) as u8;
    if !w::is_mail_kind(kind) { return None; }
    let seq = (a & 0xFF_FFFF) as u32;
    if !may_request(caller_pd, caller_task, bound_task) {
        return Some(w::reply(seq, w::DENIED, 0));
    }
    if seq == 0 || w::flow_of(a) != 0 || (kind == w::MAIL_STATUS && value != 0) {
        return Some(w::reply(seq, w::INVALID, 0));
    }
    Some(w::reply(seq, w::NO_MAIL_ACCOUNT, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_shell_bound_program_run_may_request_mail() {
        assert!(may_request(41, 4101, Some(4101)));
        assert!(!may_request(41, 4101, None));
        assert!(!may_request(41, 4101, Some(4000)));
        assert!(!may_request(41, 4000, Some(4101)));
        assert!(!may_request(41, 0, Some(0)));
        assert!(!may_request(42, 4101, None));
        assert!(!may_request(12, 12, Some(12))); // Spindle is not a mail client.
        assert!(!may_request(15, 15, Some(15))); // Browser has no mail grant.
        assert!(!may_request(13, 13, Some(13))); // SexNet cannot impersonate an app.
    }

    #[test]
    fn only_shells_boot_task_may_enter_setup_path() {
        assert!(may_request(3, 3, None));
        assert!(!may_request(3, 30, None));
        assert!(!may_request(3, 3, Some(3)));
        assert!(!may_request(3, 0, None));
    }

    #[test]
    fn broker_ops_fail_closed_for_stale_or_forged_runs_and_never_report_send() {
        let send = w::request(7, w::MAIL_SUBMIT);
        let answer = |pd, task, bound| w::decode_reply(unavailable_reply(pd, task, bound, send, 42).unwrap()).unwrap();
        assert_eq!(answer(41, 4101, Some(4101)), (7, w::NO_MAIL_ACCOUNT, 0));
        assert_eq!(answer(41, 4101, Some(4000)), (7, w::DENIED, 0));
        assert_eq!(answer(42, 4101, None), (7, w::DENIED, 0));
        assert_eq!(answer(15, 15, None), (7, w::DENIED, 0));
        assert_eq!(answer(3, 30, None), (7, w::DENIED, 0));
        assert_eq!(answer(3, 3, None), (7, w::NO_MAIL_ACCOUNT, 0));
        assert_eq!(w::decode_reply(unavailable_reply(41, 4101, Some(4101), w::request_on(8, w::MAIL_INBOX, 1), 1).unwrap()), Some((8, w::INVALID, 0)));
        assert_eq!(w::decode_reply(unavailable_reply(41, 4101, Some(4101), w::request(9, w::MAIL_STATUS), 1).unwrap()), Some((9, w::INVALID, 0)));
        assert_eq!(unavailable_reply(41, 4101, Some(4101), w::request(10, w::SEND), 42), None);
    }
}
