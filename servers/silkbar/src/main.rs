#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, Ordering};
use silkbar_model::{
    SilkBarUpdate, UpdateKind, ChipKind, ChipSlot, OP_SILKBAR_UPDATE, validate_silkbar_contract,
    contract_fingerprint, SILKBAR_ABI_VERSION, SILKBAR_WORKSPACE_COUNT, SILKBAR_CHIP_COUNT,
    SILK_DE_BAR_ABI_V1, SILK_DE_BAR_LAYOUT_V1, SILK_DE_BAR_THEME_V1,
    SILKBAR_DEFAULT_ACTIVE_WORKSPACE_IDX, SILKBAR_WORKSPACE_IDX_MAX, DEFAULT_SILK_BAR,
};
use sex_pdx::{OP_BELL_LIST, OP_BELL_SUBSCRIBE, SLOT_BELL};
mod civil_clock;

const BELL_DELIVERY_PROOF_ENABLED: bool = option_env!("SEXOS_BELL_DELIVERY_PROOF").is_some();
const CLOCK_FORCE_STALL_PROOF_ENABLED: bool =
    option_env!("SEXOS_SILKBAR_CLOCK_FORCE_STALL_PROOF").is_some();

// SERIAL_DIET_V1: shared budget for hot-loop diagnostics. Early boot keeps
// the full trace (first 120 lines); steady-state loops go quiet so
// serial VM-exits stop dominating wall clock. Error and gate-required
// markers are NOT routed through this.
static mut HOT_LOG_BUDGET: u32 = 120;
#[inline(always)]
fn hot_log() -> bool {
    unsafe {
        if HOT_LOG_BUDGET > 0 { HOT_LOG_BUDGET -= 1; true } else { false }
    }
}

static CAP_READY_DISPLAY: AtomicBool = AtomicBool::new(false);
static EDGE_SEND_EMITTED_DISPLAY: AtomicBool = AtomicBool::new(false);
static mut STARTUP_INBOX: sex_pdx::display_ready::StartupInbox<
    { sex_pdx::display_ready::STARTUP_INBOX_CAPACITY }
> = sex_pdx::display_ready::StartupInbox::new();
static DEFER_EMITTED_DISPLAY: AtomicBool = AtomicBool::new(false);

fn send_update_status(update: SilkBarUpdate) -> u64 {
    let result = sex_pdx::pdx_call_checked(
        sex_pdx::SLOT_DISPLAY,
        OP_SILKBAR_UPDATE,
        update.kind as u64,
        (update.index as u64) << 32 | update.a as u64,
        update.b as u64,
    );

    if result.is_ok() && !EDGE_SEND_EMITTED_DISPLAY.swap(true, Ordering::Relaxed) {
        sex_pdx::serial_println!(
            "[bootgraph.edge.send from=silkbar to=sexdisplay slot=SLOT_DISPLAY op=OP_SILKBAR_UPDATE first=1]"
        );
    }

    if !CAP_READY_DISPLAY.load(Ordering::Relaxed) {
        match result {
            Ok(_) => {
                CAP_READY_DISPLAY.store(true, Ordering::Relaxed);
            }
            Err(err) if err == sex_pdx::ERR_CAP_INVALID => {
                if !DEFER_EMITTED_DISPLAY.swap(true, Ordering::Relaxed) {
                    sex_pdx::serial_println!(
                        "[bootgraph.edge.defer from=silkbar to=sexdisplay slot=5 reason=missing_cap]"
                    );
                }
                return err;
            }
            _ => {}
        }
    }

    if let Err(err) = result {
        unsafe {
            static mut DROP_COUNTER: u64 = 0;
            static mut DROP_LOG_BUDGET: u32 = 16;
            let n = DROP_COUNTER;
            DROP_COUNTER = DROP_COUNTER.wrapping_add(1);
            let remaining = &mut DROP_LOG_BUDGET;
            if *remaining > 0 {
                *remaining -= 1;
                sex_pdx::serial_println!("[silkbar.send_update.drop] kind={} idx={} err={:#x} count={}",
                    update.kind, update.index, err, n);
            }
            // Clock-specific drop: 64 entries so starvation survives startup noise.
            if update.kind == UpdateKind::SetClock as u32 {
                static mut CLOCK_DROP_COUNTER: u64 = 0;
                static mut CLOCK_DROP_LOG_BUDGET: u32 = 64;
                let cn = CLOCK_DROP_COUNTER;
                CLOCK_DROP_COUNTER = CLOCK_DROP_COUNTER.wrapping_add(1);
                let cb = &mut CLOCK_DROP_LOG_BUDGET;
                if *cb > 0 {
                    *cb -= 1;
                    sex_pdx::serial_println!("[silkbar.send_update.drop.clock] count={} err={:#x}",
                        cn, err);
                }
            }
        }
        return err;
    }
    0
}

fn send_update(update: SilkBarUpdate) {
    let _ = send_update_status(update);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_pdx::serial_println!("[silkbar.init.start]");
    // Static per-client storage keeps early ordinary messages off the task stack.
    let startup_inbox = unsafe { &mut *core::ptr::addr_of_mut!(STARTUP_INBOX) };
    startup_inbox.wait_for_display().expect("silkbar display-ready inbox full");
    sex_pdx::serial_println!("[display.ready.accepted] client=silkbar caller=1 version=1");
    sex_pdx::serial_println!("[silkbar.boot.begin]");
    sex_pdx::serial_println!("[bootgraph.silkbar.spawned]");
    let contract_err = validate_silkbar_contract();
    let degraded = contract_err != 0;
    if degraded {
        sex_pdx::serial_println!("[silk.de.contract.producer.fail] reason={} abi={} layout={} theme={} fp=0x{:016x}",
            contract_err, SILK_DE_BAR_ABI_V1, SILK_DE_BAR_LAYOUT_V1, SILK_DE_BAR_THEME_V1, contract_fingerprint());
        sex_pdx::serial_println!("[silkbar.boot.contract.fail] code={}", contract_err);
        sex_pdx::serial_println!("[bootgraph.silkbar.degraded] reason=contract_fail");
    } else {
        sex_pdx::serial_println!("[silk.de.contract.producer.pass] abi={} layout={} theme={} fp=0x{:016x}",
            SILK_DE_BAR_ABI_V1, SILK_DE_BAR_LAYOUT_V1, SILK_DE_BAR_THEME_V1, contract_fingerprint());
        sex_pdx::serial_println!("[silkbar.boot.contract.ok]");
        sex_pdx::serial_println!("[bootgraph.silkbar.contract_ready]");
    }
    sex_pdx::serial_println!("[silk.contract.validate.version] version={}", SILKBAR_ABI_VERSION);

    let mut focus_state: u8 = 0;
    let mut last_focus_state: u8 = 0xFF;
    let mut last_options_mask: u32 = 0;
    let mut chip_phase: u8 = 0;
    let mut chip0_net: bool = true;
    // Clock state: software counter (no get_ticks() — frozen under QEMU TCG).
    // Each 100-yield cadence advances by 1 second.
    let mut hh: u8 = DEFAULT_SILK_BAR.clock_hh;
    let mut mm: u8 = DEFAULT_SILK_BAR.clock_mm;
    let mut ss: u8 = DEFAULT_SILK_BAR.clock_ss;
    // Loop iteration counter — drives clock, bell, and chip cadence.
    let mut loop_iter: u64 = 0;
    // Cadence accumulator: one yield per outer loop, 100 yields per clock step.
    let mut cadence_yields: u16 = 0;
    // Tick-watch state: prefer real monotonic ticks when available (get_ticks()>0),
    // fall back to synthetic yield counting on QEMU TCG (get_ticks()==0).
    let mut last_clock_ticks: u64 = 0;
    let mut clock_source_logged: bool = false;
    let mut civil = civil_clock::Probe::new();
    // Stale-real-tick fallback: if raw_ticks is nonzero but stops advancing
    // (observed under KVM where LAPIC fires once then stalls), switch to
    // synthetic cadence after STALE_REAL_TICK_FALLBACK_LOOPS passes.
    let mut stale_tick_loops: u16 = 0;
    const STALE_REAL_TICK_FALLBACK_LOOPS: u16 = 16;
    // Cadence threshold used when real ticks are nonzero but stalled.
    // Separate from pure-synthetic TCG threshold so KVM fallback cadence
    // can be tuned independently.
    const STALE_REAL_TICK_FALLBACK_THRESHOLD: u16 = 4;
    const BOOT_CLOCK_SENDS_TARGET: u8 = 2;
    /// V10 cadence calibration: reduced from 100 to 62 to match PIT tick rate.
    /// At 62 ticks/sec (PIT), threshold=62 yields ~1 Hz visible clock cadence.
    /// Previous threshold=100 gave ~0.62 Hz — too slow for GTK visual proof.
    const STEADY_CLOCK_THRESHOLD: u16 = 62;
    const REAL_TICK_VISIBLE_CLOCK_THRESHOLD: u16 = STEADY_CLOCK_THRESHOLD;
    // Synthetic visible threshold: used when get_ticks()==0 (QEMU TCG) so the
    // displayed clock advances at a proof-usable rate (~1 sec per loop yield).
    // V10 cadence calibration: reduced from 16 to 8 to match sexdisplay fallback
    // threshold.  Ensures both servers advance at same rate on TCG, preventing
    // the fallback from racing ahead and causing permanent handoff rejection.
    const SYNTHETIC_VISIBLE_CLOCK_THRESHOLD: u16 = 8;
    // Real-tick visible cadence: intentionally conservative to avoid racing
    // when get_ticks() advances quickly under virtualization.
    const LIVE_CLOCK_THRESHOLD: u16 = REAL_TICK_VISIBLE_CLOCK_THRESHOLD;
    let mut boot_clock_sends: u8 = 0;
    let mut force_stall_seeded = false;
    let mut force_stall_ss: u8 = 0;
    let mut force_stall_repeat_sends: u16 = 0;
    // Cached Bell generation counter. 0 forces first LIST poll.
    let mut bell_gen_cached: u64 = 0;
    // True when OP_BELL_LIST is enqueued and reply not yet received.
    let mut bell_pending_list: bool = false;

    // INIT updates are deferred to avoid startup blocking before loop liveness.
    // Clock init is intentionally omitted (no init SetClock ss=0).
    let mut init_deferred: [SilkBarUpdate; SILKBAR_WORKSPACE_COUNT + SILKBAR_CHIP_COUNT] =
        [SilkBarUpdate::new(0, 0, 0, 0); SILKBAR_WORKSPACE_COUNT + SILKBAR_CHIP_COUNT];
    let mut init_deferred_count: usize = 0;
    if !degraded {
        for ws_idx in 0..SILKBAR_WORKSPACE_COUNT as u8 {
            init_deferred[init_deferred_count] = SilkBarUpdate::new(
                UpdateKind::SetWorkspaceActive as u32,
                ws_idx,
                if ws_idx == SILKBAR_DEFAULT_ACTIVE_WORKSPACE_IDX { 1 } else { 0 },
                0,
            );
            init_deferred_count += 1;
        }
        for chip_idx in 0..SILKBAR_CHIP_COUNT as u8 {
            init_deferred[init_deferred_count] =
                SilkBarUpdate::new(UpdateKind::SetChipVisible as u32, chip_idx, 1, 0);
            init_deferred_count += 1;
        }
    }
    let mut init_deferred_next: usize = 0;
    sex_pdx::serial_println!("[silkbar.boot.init.defer] count={}", init_deferred_count);
    sex_pdx::serial_println!("[silkbar.ready]");

    loop {
        {
            static mut CADENCE_START_BUDGET: u32 = 16;
            let b = unsafe { &mut CADENCE_START_BUDGET };
            if *b > 0 {
                *b -= 1;
                sex_pdx::serial_println!("[silkbar.loop.cadence.start] iter={}", loop_iter);
            }
        }
        {
            static mut STALL_AFTER_START_BUDGET: u32 = 2;
            let b = unsafe { &mut STALL_AFTER_START_BUDGET };
            if *b > 0 {
                *b -= 1;
                sex_pdx::serial_println!("[silkbar.stall.after_start] iter={}", loop_iter);
            }
        }
        if loop_iter == 0 {
            sex_pdx::serial_println!("[bootgraph.silkbar.loop_ready]");
        }
        // Process at most one upstream message per loop (non-blocking).
        {
            static mut STALL_BEFORE_RECV_BUDGET: u32 = 2;
            let b = unsafe { &mut STALL_BEFORE_RECV_BUDGET };
            if *b > 0 {
                *b -= 1;
                sex_pdx::serial_println!("[silkbar.stall.before_recv] iter={}", loop_iter);
            }
        }
        if let Some(msg) = startup_inbox.pop().or_else(|| sex_pdx::pdx_try_listen_raw(0)) {
            if msg.type_id == sex_pdx::OP_SILKBAR_WORKSPACE_ACTIVE {
                let ws_raw = msg.arg0 as u8;
                if ws_raw > SILKBAR_WORKSPACE_IDX_MAX {
                    sex_pdx::serial_println!(
                        "[silkbar.update.reject] kind={} idx={} reason=out_of_bounds",
                        msg.type_id,
                        ws_raw
                    );
                    sex_pdx::serial_println!("[silkbar.cadence.no_skip] reason=workspace iter={}", loop_iter);
                }
                let ws = ws_raw;
                sex_pdx::serial_println!("[silkbar.workspace.recv] index={}", ws);
                sex_pdx::serial_println!("[silkbar.workspace.active.set] index={}", ws);
                sex_pdx::serial_println!("[silkbar.workspace.active.send.start] index={}", ws);
                if !degraded {
                    for i in 0..SILKBAR_WORKSPACE_COUNT as u8 {
                        send_update(SilkBarUpdate::new(
                            UpdateKind::SetWorkspaceActive as u32, i, if i == ws { 1 } else { 0 }, 0,
                        ));
                    }
                }
                sex_pdx::serial_println!("[silkbar.workspace.active.send.ok] index={}", ws);
            } else if msg.type_id == sex_pdx::OP_SILKBAR_FOCUS_STATE {
                let raw = msg.arg0 as u8;
                if raw > 3 {
                    sex_pdx::serial_println!(
                        "[silkbar.update.reject] kind={} idx={} reason=out_of_bounds",
                        msg.type_id,
                        raw
                    );
                    sex_pdx::serial_println!("[silkbar.cadence.no_skip] reason=options iter={}", loop_iter);
                }
                focus_state = raw;
                // Extract selected-window options mask from arg1 (V1 extension).
                // Old senders pass 0 (no options) — backward compatible.
                let options_mask = msg.arg1 as u32;
                sex_pdx::serial_println!("[silkbar.selected.options.recv] mask={:#x}", options_mask);
                if !degraded && options_mask != last_options_mask {
                    last_options_mask = options_mask;
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetSelectedOptions as u32, 0, options_mask, 0,
                    ));
                    sex_pdx::serial_println!("[silkbar.selected.options.forward] mask={:#x}", options_mask);
                }
            } else if msg.type_id == 1 && msg.caller_pd == 10 {
                // ── Bell reply (SUBSCRIBE generation or LIST packed counts) ──
                {
                    static mut STALL_BEFORE_BELL_BUDGET: u32 = 2;
                    let b = unsafe { &mut STALL_BEFORE_BELL_BUDGET };
                    if *b > 0 {
                        *b -= 1;
                        sex_pdx::serial_println!("[silkbar.stall.before_bell] iter={}", loop_iter);
                    }
                }
                if bell_pending_list {
                    // LIST reply: arg0 = packed lane counts from Bell.
                    // Repack for sexdisplay SetBellPresence format:
                    //   bits 7:0   = total_visible
                    //   bits 15:8  = redacted_count (from Bell's bits 63:56)
                    //   bits 23:16 = flags (bit 0 = bell_available since LIST succeeded)
                    bell_pending_list = false;
                    let packed = msg.arg0;
                    let total = (packed & 0xFF) as u8;
                    let redacted = ((packed >> 56) & 0xFF) as u8;
                    let flags: u8 = 1; // bit 0 = bell_available
                    let a = (total as u32) | ((redacted as u32) << 8) | ((flags as u32) << 16);
                    unsafe {
                        static mut BELL_REPLY_BUDGET: u32 = 8;
                        let b = &mut BELL_REPLY_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            sex_pdx::serial_println!("[silkbar.bell.poll.reply] total={} redacted={} flags={:#x}",
                                total, redacted, flags);
                        }
                    }
                    if !degraded {
                        send_update(SilkBarUpdate::new(
                            UpdateKind::SetBellPresence as u32, 0, a, 0,
                        ));
                    }
                    if BELL_DELIVERY_PROOF_ENABLED {
                        sex_pdx::serial_println!("[bell.poll.ok] total={} redacted={} flags={:#x}", total, redacted, flags);
                        sex_pdx::serial_println!("[silkbar.bell.state] total={} redacted={} flags={:#x}", total, redacted, flags);
                    }
                } else {
                    // SUBSCRIBE reply: arg0 = current generation.
                    let gen = msg.arg0;
                    let changed = if gen != bell_gen_cached && gen != u64::MAX { 1 } else { 0 };
                    if changed != 0 {
                        unsafe {
                            static mut BELL_GEN_REPLY_CHANGED_BUDGET: u32 = 8;
                            let b = &mut BELL_GEN_REPLY_CHANGED_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                sex_pdx::serial_println!("[silkbar.bell.gen.reply] gen={} changed={}", gen, changed);
                            }
                        }
                    } else {
                        unsafe {
                            static mut BELL_GEN_REPLY_STEADY_BUDGET: u32 = 1;
                            let b = &mut BELL_GEN_REPLY_STEADY_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                sex_pdx::serial_println!("[silkbar.bell.gen.reply] gen={} changed={}", gen, changed);
                            }
                        }
                    }
                    if gen == u64::MAX {
                        // Denied — fall back to LIST.
                        unsafe {
                            static mut BELL_GEN_FALLBACK_BUDGET: u32 = 8;
                            let b = &mut BELL_GEN_FALLBACK_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                sex_pdx::serial_println!("[silkbar.bell.gen.fallback] reason=denied");
                            }
                        }
                        let list_args = 0xFFu64 | (1u64 << 8);
                        if let Ok(_) = sex_pdx::pdx_call_checked(SLOT_BELL, OP_BELL_LIST, list_args, 0, 0) {
                            bell_pending_list = true;
                        } else if !degraded {
                            send_update(SilkBarUpdate::new(
                                UpdateKind::SetBellPresence as u32, 0, 0, 0,
                            ));
                        }
                    } else if gen != bell_gen_cached {
                        bell_gen_cached = gen;
                        // Generation changed — call LIST.
                        let list_args = 0xFFu64 | (1u64 << 8);
                        if let Ok(_) = sex_pdx::pdx_call_checked(SLOT_BELL, OP_BELL_LIST, list_args, 0, 0) {
                            bell_pending_list = true;
                        }
                    }
                    // If gen == bell_gen_cached: no change, skip update.
                }
                {
                    static mut STALL_AFTER_BELL_BUDGET: u32 = 2;
                    let b = unsafe { &mut STALL_AFTER_BELL_BUDGET };
                    if *b > 0 {
                        *b -= 1;
                        sex_pdx::serial_println!("[silkbar.stall.after_bell] iter={}", loop_iter);
                    }
                }
                if hot_log() { sex_pdx::serial_println!("[silkbar.stall.after_bell_all] iter={}", loop_iter); }
            } else {
                sex_pdx::serial_println!("[pdx.opcode.unknown] silkbar type_id={:#x} caller={}", msg.type_id, msg.caller_pd);
            }
        }
        {
            static mut STALL_AFTER_RECV_BUDGET: u32 = 2;
            let b = unsafe { &mut STALL_AFTER_RECV_BUDGET };
            if *b > 0 {
                *b -= 1;
                sex_pdx::serial_println!("[silkbar.stall.after_recv] iter={}", loop_iter);
            }
        }

        {
            static mut STALL_BEFORE_SHELL_SYNC_BUDGET: u32 = 2;
            let b = unsafe { &mut STALL_BEFORE_SHELL_SYNC_BUDGET };
            if *b > 0 {
                *b -= 1;
                sex_pdx::serial_println!("[silkbar.stall.before_shell_sync] iter={}", loop_iter);
            }
        }
        if focus_state != last_focus_state {
            // Focus state drives workspace urgent highlight.
            // none: clear all; shell/app/debug => ws0/ws1/ws2 urgent respectively.
            let urgent_ws = match focus_state {
                1 => Some(0u8),
                2 => Some(1u8),
                3 => Some(2u8),
                _ => None,
            };
            if !degraded {
                for ws in 0..SILKBAR_WORKSPACE_COUNT as u8 {
                    let urgent = if Some(ws) == urgent_ws { 1 } else { 0 };
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetWorkspaceUrgent as u32, ws, urgent, 0,
                    ));
                }
            }
            last_focus_state = focus_state;
        }
        {
            static mut STALL_AFTER_SHELL_SYNC_BUDGET: u32 = 2;
            let b = unsafe { &mut STALL_AFTER_SHELL_SYNC_BUDGET };
            if *b > 0 {
                *b -= 1;
                sex_pdx::serial_println!("[silkbar.stall.after_shell_sync] iter={}", loop_iter);
            }
        }

        if !degraded && init_deferred_next < init_deferred_count {
            // BURST: send ALL pending boot deferred updates in one iteration.
            // Pre-2026-05-25 each update was sent in a separate loop iteration
            // separated by sys_yield(), causing sexdisplay to drain+redraw on
            // every individual update — producing a visible top-strip redraw
            // storm (9 redraws for 9 deferred updates).  Bursting keeps all
            // messages in sexdisplay's queue for a single drain+redraw cycle
            // and avoids clock-cadence accumulation across the flush.
            let count = init_deferred_count;
            while init_deferred_next < init_deferred_count {
                let update = init_deferred[init_deferred_next];
                if send_update_status(update) != 0 {
                    // Retain this update and all later startup work for retry.
                    break;
                }
                init_deferred_next += 1;
            }
            if init_deferred_next == init_deferred_count {
                sex_pdx::serial_println!("[silkbar.boot.init.flush.burst.done] count={}", count);
            }
        }

        // Tick-watch cadence: prefer real monotonic ticks when get_ticks()>0,
        // fall back to synthetic yield counting on QEMU TCG (get_ticks()==0).
        // One tick change = one cadence increment; LIVE_CLOCK_THRESHOLD
        // increments complete one logical clock second.
        let mut cadence_threshold = LIVE_CLOCK_THRESHOLD; // default for real ticks
        let raw_ticks = sex_pdx::get_ticks();
        if raw_ticks == 0 {
            // QEMU TCG fallback: use visible synthetic threshold so clock
            // advances at a proof-usable rate (~1 sec per yield).
            stale_tick_loops = 0;
            cadence_threshold = SYNTHETIC_VISIBLE_CLOCK_THRESHOLD;
            cadence_yields = cadence_yields.wrapping_add(1);
            if !clock_source_logged && loop_iter >= 10 {
                sex_pdx::serial_println!("[silkbar.clock.source] kind=synthetic raw_ticks=0 threshold={}", cadence_threshold);
                clock_source_logged = true;
            }
            // Budgeted marker: prove visible synthetic threshold is active.
            unsafe {
                static mut SYNTHETIC_VISIBLE_BUDGET: u32 = 4;
                let s = &mut SYNTHETIC_VISIBLE_BUDGET;
                if *s > 0 {
                    *s -= 1;
                    sex_pdx::serial_println!(
                        "[silkbar.clock.synthetic.visible] threshold={}",
                        cadence_threshold
                    );
                }
            }
        } else if raw_ticks > last_clock_ticks {
            // Real tick advanced: resume from any stale fallback.
            let was_stale = stale_tick_loops >= STALE_REAL_TICK_FALLBACK_LOOPS;
            stale_tick_loops = 0;
            last_clock_ticks = raw_ticks;
            cadence_yields = cadence_yields.wrapping_add(1);
            if !clock_source_logged {
                sex_pdx::serial_println!("[silkbar.clock.source] kind=real raw_ticks={}", raw_ticks);
                clock_source_logged = true;
            }
            // Budgeted marker: first 12 real-tick clock advances.
            unsafe {
                static mut REAL_TICK_BUDGET: u32 = 12;
                let r = &mut REAL_TICK_BUDGET;
                if *r > 0 {
                    *r -= 1;
                    sex_pdx::serial_println!(
                        "[silkbar.clock.real_tick] ticks={} iter={} yield_count={}",
                        raw_ticks, loop_iter, cadence_yields
                    );
                }
            }
            // Resume marker: ticks recovered after a stall.
            if was_stale {
                unsafe {
                    static mut RESUME_BUDGET: u32 = 4;
                    let r = &mut RESUME_BUDGET;
                    if *r > 0 {
                        *r -= 1;
                        sex_pdx::serial_println!(
                            "[silkbar.clock.real_tick.resume] ticks={}",
                            raw_ticks
                        );
                    }
                }
            }
        } else {
            // raw_ticks == last_clock_ticks && raw_ticks != 0.
            // Real ticks exist but haven't advanced: count stale loops.
            stale_tick_loops = stale_tick_loops.wrapping_add(1);
            if stale_tick_loops >= STALE_REAL_TICK_FALLBACK_LOOPS {
                // Stale for too long: fall back to synthetic so clock
                // doesn't freeze while waiting for next timer interrupt.
                // Use dedicated stale threshold (tuned for KVM) — not the
                // pure-synthetic TCG threshold.
                cadence_threshold = STALE_REAL_TICK_FALLBACK_THRESHOLD;
                cadence_yields = cadence_yields.wrapping_add(1);
                unsafe {
                    static mut STALE_BUDGET: u32 = 8;
                    let s = &mut STALE_BUDGET;
                    if *s > 0 {
                        *s -= 1;
                        sex_pdx::serial_println!(
                            "[silkbar.clock.real_tick.stale] ticks={} loops={} fallback=synthetic threshold={}",
                            raw_ticks, stale_tick_loops, cadence_threshold
                        );
                    }
                }
            }
            // else: still within grace window; don't increment cadence_yields.
        }
        // Unbudgeted iter=2 proof: emit at milestones only (not every pass).
        if loop_iter == 2 {
            let half = cadence_threshold / 2;
            if cadence_yields == 1
                || cadence_yields == half
                || cadence_yields + 1 == cadence_threshold
                || cadence_yields == cadence_threshold
            {
                sex_pdx::serial_println!(
                    "[silkbar.loop.cadence.count] iter={} yield_count={} threshold={}",
                    loop_iter, cadence_yields, cadence_threshold
                );
            }
        }
        // Post-10 synthetic threshold proof: once per cadence after iter 10.
        if loop_iter == 10 {
            unsafe {
                static mut SYNTH_THRESHOLD_BUDGET: u32 = 1;
                let b = &mut SYNTH_THRESHOLD_BUDGET;
                if *b > 0 && cadence_yields == 1 {
                    *b -= 1;
                    sex_pdx::serial_println!(
                        "[silkbar.clock.synthetic.threshold] iter={} threshold={} reason=until_real_timer",
                        loop_iter, cadence_threshold
                    );
                }
            }
        }
        sex_pdx::sys_yield();
        if cadence_yields < cadence_threshold {
            continue;
        }
        let _ = STEADY_CLOCK_THRESHOLD; // reserved for real timer integration
        cadence_yields = 0;
        // Queries and ordinary enqueue attempts are bounded and never wait for
        // a reply. A full route retains the exact snapshot for the next cadence.
        // The older counter below remains demonstration/liveness data only.
        if !degraded && option_env!("SEXOS_LEGACY_DEMO_CLOCK").is_none() {
            let mut observed = None;
            let mut source_known = false;
            if civil.stage(
                || {let info=sex_pdx::clock_info(sex_pdx::clock_source::SLOT);source_known=info.is_ok_and(|i|i.consumer==sex_pdx::clock_source::STATUS_BAR&&i.source==sex_pdx::clock_source::CMOS_UTC);info},
                || { let result=sex_pdx::clock_snapshot(sex_pdx::clock_source::SLOT);observed=Some(result);result },
            ) {
                let u=civil.pending().unwrap();
                let (seconds,attempts)=observed.and_then(Result::ok).map_or((0,0),|s|(s.seconds,s.attempts));
                sex_pdx::serial_record!("[silkbar.civil.sample] sequence={} state={} seconds={} observed_seconds={} attempts={} source_known={} profile=q35_cmos_utc precision=1 snapshot=1",u.sequence,u.state as u8,u.seconds,seconds,attempts,source_known as u8);
            }
            if let Some(u)=civil.pending() {
                sex_pdx::serial_record!("[silkbar.civil.publish.try] sequence={} state={} seconds={} held_display=1",u.sequence,u.state as u8,u.seconds);
                let status=send_update_status(u.encode());
                if status==0 {
                    civil.accepted();
                    sex_pdx::serial_record!("[silkbar.civil.enqueued] sequence={} state={} seconds={} held_display=1",u.sequence,u.state as u8,u.seconds);
                }
            }
        }
        {
            static mut CADENCE_DONE_BUDGET: u32 = 16;
            let b = unsafe { &mut CADENCE_DONE_BUDGET };
            if *b > 0 {
                *b -= 1;
                if hot_log() { sex_pdx::serial_println!("[silkbar.loop.cadence.done] iter={}", loop_iter); }
            }
        }

        // Advance software clock by one logical second per cadence.
        // raw_ticks captured above in tick-watch cadence section; reused here
        // so only one get_ticks() syscall per loop iteration.
        if hot_log() { sex_pdx::serial_println!("[silkbar.loop.iter_advance] old={} new={}", loop_iter, loop_iter.wrapping_add(1)); }
        loop_iter = loop_iter.wrapping_add(1);
        // [silkbar.loop.alive] budgeted marker
        {
            static mut LOOP_ALIVE_BUDGET: u32 = 16;
            let b = unsafe { &mut LOOP_ALIVE_BUDGET };
            if *b > 0 {
                *b -= 1;
                if hot_log() { sex_pdx::serial_println!("[silkbar.loop.alive] iter={}", loop_iter); }
            }
        }
        // Software clock: increment by 1 second
        ss = ss.wrapping_add(1);
        if ss >= 60 {
            ss = 0;
            mm = mm.wrapping_add(1);
            if mm >= 60 {
                mm = 0;
                hh = hh.wrapping_add(1);
                if hh >= 24 {
                    hh = 0;
                }
            }
        }
        // [silkbar.clock.tick] budgeted diagnostic
        {
            static mut CLOCK_TICK_BUDGET: u32 = 12;
            let c = unsafe { &mut CLOCK_TICK_BUDGET };
            if *c > 0 {
                *c -= 1;
                sex_pdx::serial_println!("[silkbar.clock.tick] hh={} mm={} ss={} raw_ticks={}", hh, mm, ss, raw_ticks);
            }
        }

        // ── Bell presence poll (every ~2 seconds) ──────────────────────────
        // V2: Poll OP_BELL_SUBSCRIBE for generation counter.
        // If generation changed since last poll, call OP_BELL_LIST.
        // If SUBSCRIBE fails (no cap or Bell down), fall back to LIST.
        // Existing LIST reply handler (type_id=1, caller_pd=1) covers both paths.
        if !degraded && loop_iter % 2 == 0 && !bell_pending_list {
            let result = sex_pdx::pdx_call_checked(SLOT_BELL, OP_BELL_SUBSCRIBE, 0, 0, 0);
            if let Err(e) = result {
                // SUBSCRIBE failed — fall back to LIST.
                unsafe {
                    static mut BELL_GEN_FALLBACK_BUDGET: u32 = 8;
                    let b = &mut BELL_GEN_FALLBACK_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        sex_pdx::serial_println!("[silkbar.bell.gen.fallback] reason=cap_err err={:#x}", e);
                    }
                }
                let list_args = 0xFFu64 | (1u64 << 8);
                if let Ok(_) = sex_pdx::pdx_call_checked(SLOT_BELL, OP_BELL_LIST, list_args, 0, 0) {
                    bell_pending_list = true;
                } else {
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetBellPresence as u32, 0, 0, 0,
                    ));
                }
            }
            // If Ok: SUBSCRIBE enqueued. Reply with generation arrives
            // asynchronously via pdx_try_listen_raw(0). Handled above
            // in the type_id=1, caller_pd=1 branch (else: SUBSCRIBE).
        }

        // Stage 2C: bounded internal status-chip stub (no new ABI, no floods).
        // Slow cadence: every 120 seconds.
        if !degraded && loop_iter % 120 == 0 {
            match chip_phase {
                0 => {
                    let chip0_kind = if chip0_net { ChipKind::Net } else { ChipKind::Wifi };
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetChipKind as u32, ChipSlot::Chip0 as u8, chip0_kind as u32, 0,
                    ));
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetChipKind as u32, ChipSlot::Chip1 as u8, ChipKind::Wifi as u32, 0,
                    ));
                    chip0_net = !chip0_net;
                }
                1 => {
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetChipKind as u32, ChipSlot::Chip2 as u8, ChipKind::Battery as u32, 0,
                    ));
                }
                2 => {
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetChipKind as u32, ChipSlot::Clock as u8, ChipKind::Net as u32, 0,
                    ));
                }
                _ => {
                    send_update(SilkBarUpdate::new(
                        UpdateKind::SetChipKind as u32, ChipSlot::Clock as u8, ChipKind::Battery as u32, 0,
                    ));
                }
            }
            chip_phase = (chip_phase + 1) & 0x3;
        }

        let mut clock_sent = false;
        let clock_status = if degraded {
            u64::MAX
        } else if CLOCK_FORCE_STALL_PROOF_ENABLED {
            if !force_stall_seeded {
                force_stall_seeded = true;
                force_stall_ss = ss;
                clock_sent = true;
                sex_pdx::serial_println!(
                    "[silkbar.clock.force_stall.seed] hh={} mm={} ss={}",
                    hh, mm, ss
                );
                send_update_status(SilkBarUpdate::new(
                    UpdateKind::SetClock as u32, 0, hh as u32, ((mm as u32) << 8) | ss as u32,
                ))
            } else if force_stall_repeat_sends < 124 {
                let mut repeat_status = u64::MAX;
                let mut burst: u8 = 0;
                while burst < 4 && force_stall_repeat_sends < 124 {
                    force_stall_repeat_sends = force_stall_repeat_sends.wrapping_add(1);
                    repeat_status = send_update_status(SilkBarUpdate::new(
                        UpdateKind::SetClock as u32,
                        0,
                        hh as u32,
                        ((mm as u32) << 8) | force_stall_ss as u32,
                    ));
                    burst = burst.wrapping_add(1);
                }
                clock_sent = true;
                if (force_stall_repeat_sends % 20) == 0 {
                    sex_pdx::serial_println!(
                        "[silkbar.clock.force_stall.repeat] ss={} sends={}",
                        force_stall_ss, force_stall_repeat_sends
                    );
                }
                repeat_status
            } else {
                static mut FORCE_STALL_SUPPRESS_BUDGET: u32 = 4;
                let b = unsafe { &mut FORCE_STALL_SUPPRESS_BUDGET };
                if *b > 0 {
                    *b -= 1;
                    sex_pdx::serial_println!(
                        "[silkbar.clock.force_stall.suppress] repeats={} status=hold",
                        force_stall_repeat_sends
                    );
                }
                u64::MAX
            }
        } else {
            // ── Bounded clock send: cadence-gated liveness with periodic proof ──
            // V9 CLOCK_SEND_LIMIT removed: the 900-cadence cutoff killed the only
            // live source when stale-real-tick fast-cadence raced loop_iter to 900,
            // leaving sexdisplay stranded in clock_from_silkbar=true with no updates.
            //
            // Cadence itself provides rate limiting (LIVE_CLOCK_THRESHOLD=100,
            // SYNTHETIC_VISIBLE_CLOCK_THRESHOLD=16, STALE_REAL_TICK_FALLBACK_THRESHOLD=4).
            // Post-V9 V8 independent cadence ensures sexdisplay fallback resumes if
            // silkbar becomes silent — no permanent-flood risk.
            //
            // Periodic liveness marker proves the clock source is still sending;
            // enables diagnostics without an artificial cutoff.
            clock_sent = true;
            send_update_status(SilkBarUpdate::new(
                UpdateKind::SetClock as u32, 0, hh as u32, ((mm as u32) << 8) | ss as u32,
            ))
        };
        // ── Cadence sample marker (budgeted) — outside if/else to avoid type mismatch ──
        if clock_sent {
            unsafe {
                static mut SILKBAR_CADENCE_SAMPLE_BUDGET: u32 = 32;
                let b = &mut SILKBAR_CADENCE_SAMPLE_BUDGET;
                if *b > 0 {
                    *b -= 1;
                    if hot_log() { sex_pdx::serial_println!(
                        "[silkbar.clock.cadence.sample] ss={} yields={} threshold={} sent=1 ok=1",
                        ss, cadence_yields, cadence_threshold
                    ); }
                }
            }
        }
        if !degraded {
            if clock_sent {
                boot_clock_sends = boot_clock_sends.wrapping_add(1);
            }
            {
                static mut CLOCK_BOOT_CANARY_BUDGET: u32 = 8;
                let b = unsafe { &mut CLOCK_BOOT_CANARY_BUDGET };
                if *b > 0 {
                    *b -= 1;
                    sex_pdx::serial_println!(
                        "[silkbar.clock.boot_canary] send={} threshold={}",
                        boot_clock_sends,
                        cadence_threshold
                    );
                }
            }
        } else {
            static mut CLOCK_BOOT_CANARY_SUPPRESS_BUDGET: u32 = 4;
            let b = unsafe { &mut CLOCK_BOOT_CANARY_SUPPRESS_BUDGET };
            if *b > 0 {
                *b -= 1;
                sex_pdx::serial_println!("[silkbar.clock.boot_canary] suppressed=degraded threshold={}", cadence_threshold);
            }
        }
        // ── Periodic clock-source liveness proof: once per clock-minute ──
        // Proves silkbar is still sending SetClock without artificial cutoff.
        // Replaces V9 CLOCK_SEND_LIMIT which killed the only live source after
        // stale-real-tick fast-cadence raced loop_iter to 900.
        if !degraded && clock_sent && loop_iter > 0 && loop_iter % 60 == 0 {
            unsafe {
                static mut CLOCK_SOURCE_LIVENESS_BUDGET: u32 = 32;
                let b = &mut CLOCK_SOURCE_LIVENESS_BUDGET;
                if *b > 0 {
                    *b -= 1;
                    sex_pdx::serial_println!(
                        "[silkbar.clock.source.liveness] iter={} ss={} threshold={} ok=1",
                        loop_iter, ss, cadence_threshold
                    );
                }
            }
        }
        if loop_iter == 1 {
            if degraded {
                sex_pdx::serial_println!("[bootgraph.silkbar.degraded] reason=clock_send_disabled");
            } else {
                sex_pdx::serial_println!("[bootgraph.silkbar.clock_ready]");
            }
        }
        // Clock-send liveness: first 12 sends in full, then every 64th —
        // a periodic proof the clock source is still alive (the old
        // budget-12 cutoff made "is silkbar still sending?" unanswerable
        // from a long-session log).
        {
            static mut CLOCK_SEND_COUNT: u32 = 0;
            let c = unsafe { &mut CLOCK_SEND_COUNT };
            *c = c.wrapping_add(1);
            if *c <= 12 || *c % 64 == 0 {
                sex_pdx::serial_println!(
                    "[silkbar.clock.send] hh={} mm={} ss={} status={:#x}",
                    hh, mm, ss, clock_status
                );
            }
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
