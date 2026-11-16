//! Diagnostic-only passive read of PUBLIC domain accounting, never a wake or IPC.
//! Built only with sexos_input_payload_proof; normal images omit this module.
const PERIOD: u64 = 250;
const LIMIT: u64 = 512;
static mut SAMPLES: u64 = 0;
static mut LAST: u64 = 0;

/// Called by Shell's existing loop. An idle Shell is not woken for observation.
pub fn poll() {
    // SAFETY: Shell's existing main loop is this state's sole caller.
    unsafe {
        if SAMPLES >= LIMIT { return; }
        let before = sex_pdx::get_ticks();
        if SAMPLES != 0 && before.saturating_sub(LAST) < PERIOD { return; }
        if SAMPLES == 0 {
            sex_pdx::serial_record!("[shell.wait_observer.ready] mode=proof target=13 sys=87 period=250 limit=512");
        }
        LAST = before;
        SAMPLES += 1;
        let peer_before = sex_pdx::crash_log::peer_state(sex_pdx::web::SHELL_SLOT);
        let sample = sex_pdx::system_stats::domain(13);
        let peer_after = sex_pdx::crash_log::peer_state(sex_pdx::web::SHELL_SLOT);
        let after = sex_pdx::get_ticks();
        let live_first_epoch = |peer: &Result<sex_pdx::crash_log::PeerState, u64>| {
            peer.as_ref().is_ok_and(|peer| peer.epoch == 1 && peer.live())
        };
        if let Some(sample) = sample.filter(|sample| sample.pd == 13 && sample.task == 0
            && live_first_epoch(&peer_before) && live_first_epoch(&peer_after)) {
            sex_pdx::serial_record!("[shell.wait_observer] seq={} target=13 before={} after={} syscalls={} task=0 slot=262 epoch_before=1 life_before=1 epoch_after=1 life_after=1 ok=1", SAMPLES, before, after, sample.syscalls);
        } else {
            sex_pdx::serial_record!("[shell.wait_observer] seq={} target=13 before={} after={} ok=0", SAMPLES, before, after);
        }
    }
}
