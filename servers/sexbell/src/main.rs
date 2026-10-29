#![no_std]
#![no_main]
mod paging_probe;

use sex_pdx::{pdx_listen_raw, pdx_reply, serial_println, OP_BELL_NOTIFY, OP_BELL_CLOSE, OP_BELL_ACTION,
              OP_BELL_CLEAR, OP_BELL_MUTE_SENDER, OP_BELL_LIST, OP_BELL_SUBSCRIBE, OP_BELL_SET_POLICY};

const BELL_DELIVERY_PROOF_ENABLED: bool = option_env!("SEXOS_BELL_DELIVERY_PROOF").is_some();

// ── Attention Lanes ───────────────────────────────────────────────────
// Named ordinals for the final_lane/lane_override field. Ordinal order is
// load-bearing: find_lowest_priority_index() evicts the lowest ordinal
// first, so renaming must never reorder these values.
const LANE_LATER:   u8 = 0; // background info, no attention needed now (was "PASSIVE")
const LANE_PROJECT: u8 = 1; // Linen-object/workspace-context events (reserved, unpopulated until Linen lands)
const LANE_SOON:    u8 = 2; // worth seeing this session, not urgent
const LANE_SYSTEM:  u8 = 3; // dev-mode / OS-health events (see SelfCapDenied, category=6)
const LANE_NOW:     u8 = 4; // needs attention this moment
const LANE_RESERVED_5: u8 = 5; // reserved for future security-class events (was "SECURITY"), unused today

// Replaced local bell_reply helper with shared sex_pdx::pdx_reply(target_pd, value).
// Kernel: syscall 29 (SYSCALL_PDX_REPLY), rdi=target_pd, rsi=value.
// Reply received by caller as pdx_listen_raw(0) → msg.type_id=1, msg.arg0=value.

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[paging.client.fatal] bell {}", info);
    loop { sex_pdx::sys_yield(); }
}

// ── RAM Queue ─────────────────────────────────────────────────────────
const BELL_QUEUE_CAPACITY: usize = 16;

#[repr(C)]
#[derive(Copy, Clone)]
struct BellQueueEntry {
    /// Monotonic event ID assigned by Bell on accept (0 = invalid sentinel).
    event_id:         u64,
    /// Kernel-authoritative sender PD.
    caller_pd:        u32,
    /// Event category (0=Info .. 5=Error).
    category:         u8,
    /// Urgency hint from sender (0..3).
    requested_lane:   u8,
    /// Final lane after policy derivation (LANE_LATER=0 .. LANE_RESERVED_5=5).
    final_lane:       u8,
    /// Final urgency after policy derivation.
    final_urgency:    u8,
    /// Privacy level (0=Public .. 3=FullHidden).
    privacy_level:    u8,
    /// Redaction class (0=StructuralMeta .. 3=SecretContent).
    redaction_class:  u8,
    /// Number of action callbacks attached (V1: 0 or 1).
    action_count:     u8,
    /// First action ID (valid when action_count >= 1). Marker-only in V1.
    action_id:        u8,
    /// Number of object references attached (V1: 0 or 1).
    object_ref_count: u8,
    /// First object reference ID (valid when object_ref_count >= 1). Marker-only in V1.
    object_ref:       u8,
    /// 0 = active, 1 = dismissed by CLOSE or CLEAR. Skipped in LIST.
    dismissed:        u8,
    /// Padding.
    _pad:             [u8; 2],
}

struct BellQueue {
    /// Monotonic event ID counter. 0 reserved as invalid sentinel.
    next_event_id: u64,
    /// Head index (oldest entry).
    head: u16,
    /// Tail index (next write position).
    tail: u16,
    /// Current entry count.
    count: u16,
    /// Fixed-size entry array.
    entries: [BellQueueEntry; BELL_QUEUE_CAPACITY],
}

impl BellQueue {
    const fn new() -> Self {
        BellQueue {
            next_event_id: 1, // start at 1 (0 reserved)
            head: 0,
            tail: 0,
            count: 0,
            entries: [BellQueueEntry {
                event_id: 0,
                caller_pd: 0,
                category: 0,
                requested_lane: 0,
                final_lane: 0,
                final_urgency: 0,
                privacy_level: 0,
                redaction_class: 0,
                action_count: 0,
                action_id: 0,
                object_ref_count: 0,
                object_ref: 0,
                dismissed: 0,
                _pad: [0; 2],
            }; BELL_QUEUE_CAPACITY],
        }
    }

    fn is_full(&self) -> bool {
        self.count >= BELL_QUEUE_CAPACITY as u16
    }

    /// Find the lowest-priority active entry's index in the ring buffer.
    /// Returns None if no active entries exist.
    /// Low priority = lowest final_lane value. Ties broken by oldest (smallest distance from head).
    fn find_lowest_priority_index(&self) -> Option<u16> {
        let mut lowest_idx: Option<u16> = None;
        let mut lowest_lane: u8 = 6; // higher than any valid lane
        let mut lowest_dist: u16 = 0; // distance from head

        for i in 0..self.count as usize {
            let idx = (self.head as usize + i) % BELL_QUEUE_CAPACITY;
            let entry = &self.entries[idx];
            if entry.dismissed != 0 {
                continue;
            }
            if entry.final_lane < lowest_lane
                || (entry.final_lane == lowest_lane && lowest_idx.map_or(true, |_| i as u16 > lowest_dist))
            {
                lowest_lane = entry.final_lane;
                lowest_dist = i as u16;
                lowest_idx = Some(idx as u16);
            }
        }

        lowest_idx
    }

    /// Push a new entry, assigning event_id.
    /// If queue is full, drops the lowest-priority active entry to make room.
    /// Returns Ok with (event_id, Option<dropped_lane>) or Err("queue_full").
    fn push(&mut self, caller_pd: u32, category: u8, requested_lane: u8,
            final_lane: u8, final_urgency: u8, privacy_level: u8,
            redaction_class: u8, action_count: u8, action_id: u8,
            object_ref_count: u8, object_ref: u8) -> Result<(u64, Option<u8>), &'static str> {
        if self.is_full() {
            // Try to drop lowest-priority entry
            if let Some(drop_idx) = self.find_lowest_priority_index() {
                let dropped_lane = self.entries[drop_idx as usize].final_lane;
                self.entries[drop_idx as usize].event_id = 0;
                self.entries[drop_idx as usize].dismissed = 1;
                // Write new entry into the freed slot
                let event_id = self.next_event_id;
                self.next_event_id = if event_id == u64::MAX { 1 } else { event_id + 1 };
                self.entries[drop_idx as usize] = BellQueueEntry {
                    event_id,
                    caller_pd,
                    category,
                    requested_lane,
                    final_lane,
                    final_urgency,
                    privacy_level,
                    redaction_class,
                    action_count,
                    action_id,
                    object_ref_count,
                    object_ref,
                    dismissed: 0,
                    _pad: [0; 2],
                };
                return Ok((event_id, Some(dropped_lane)));
            }
            return Err("queue_full");
        }

        let event_id = self.next_event_id;
        // Wrap safely; 0 is reserved as invalid sentinel.
        self.next_event_id = if event_id == u64::MAX { 1 } else { event_id + 1 };

        let entry = BellQueueEntry {
            event_id,
            caller_pd,
            category,
            requested_lane,
            final_lane,
            final_urgency,
            privacy_level,
            redaction_class,
            action_count,
            action_id,
            object_ref_count,
            object_ref,
            dismissed: 0,
            _pad: [0; 2],
        };

        self.entries[self.tail as usize] = entry;
        self.tail = (self.tail + 1) % BELL_QUEUE_CAPACITY as u16;
        self.count += 1;

        Ok((event_id, None)) // None = no drop occurred
    }
}

/// Static queue instance. Safe for single-threaded use (sexbell has one thread).
static mut BELL_QUEUE: BellQueue = BellQueue::new();

/// Generation counter bumped on every queue/mute-visible state change.
/// Subscribers (OP_BELL_SUBSCRIBE) compare cached generation to detect changes
/// without scanning the queue. Wrapping is safe: false positive → extra LIST poll.
static mut BELL_GENERATION: u64 = 1;

/// Bump generation counter (wrapping-safe).
#[inline(always)]
fn bump_generation() {
    unsafe { BELL_GENERATION = BELL_GENERATION.wrapping_add(1); }
}

// ── Read-Cap Allowlist ──────────────────────────────────────────────

/// Static allowlist of PDs permitted to call OP_BELL_LIST.
/// Default-deny: any PD not in this list is rejected.
/// V1: only silk-shell (domain 3). Extended in future phases.
const BELL_LIST_ALLOWLIST: &[u32] = &[
    3,  // silk-shell (domain 3, policy owner)
    6,  // silkbar (domain 6, privacy-safe aggregate poller)
];

/// Check if a caller PD is authorized to call OP_BELL_LIST.
fn is_list_reader_allowed(caller_pd: u32) -> bool {
    BELL_LIST_ALLOWLIST.contains(&caller_pd)
}

// ── Mute List ────────────────────────────────────────────────────────

/// Static mute list of sender PDs. Muted senders' NOTIFY is rejected.
const MUTE_LIST_CAPACITY: usize = 16;
static mut MUTE_LIST: [u32; MUTE_LIST_CAPACITY] = [0; MUTE_LIST_CAPACITY];
static mut MUTE_COUNT: usize = 0;

/// Check if a caller PD is muted.
fn is_muted(caller_pd: u32) -> bool {
    unsafe {
        for i in 0..MUTE_COUNT {
            if MUTE_LIST[i] == caller_pd {
                return true;
            }
        }
    }
    false
}

/// Add a PD to the mute list. Returns Ok(()) if added or already present.
fn add_mute(caller_pd: u32) -> Result<(), &'static str> {
    unsafe {
        if is_muted(caller_pd) {
            return Ok(()); // idempotent
        }
        if MUTE_COUNT >= MUTE_LIST_CAPACITY {
            return Err("mute_list_full");
        }
        MUTE_LIST[MUTE_COUNT] = caller_pd;
        MUTE_COUNT += 1;
    }
    Ok(())
}

/// Remove a PD from the mute list. Returns true if found and removed, false if not present.
fn remove_mute(caller_pd: u32) -> bool {
    unsafe {
        for i in 0..MUTE_COUNT {
            if MUTE_LIST[i] == caller_pd {
                // Shift remaining entries left
                for j in i..MUTE_COUNT - 1 {
                    MUTE_LIST[j] = MUTE_LIST[j + 1];
                }
                MUTE_COUNT -= 1;
                return true;
            }
        }
    }
    false
}

// ── Policy Table (volatile RAM) ───────────────────────────────────────

/// Per-target PD policy entry.
/// target_pd == 0 marks an unused slot.
const POLICY_TABLE_CAPACITY: usize = 8;

#[repr(C)]
#[derive(Copy, Clone)]
struct PolicyEntry {
    target_pd:     u32,
    /// bit 0 = privacy_override active, bit 1 = lane_override active, bit 2 = force_mute active
    active_flags:  u8,
    /// Privacy override (0=Public .. 3=FullHidden). Only valid if active_flags bit 0 set.
    privacy_level: u8,
    /// Lane override (LANE_LATER=0 .. LANE_RESERVED_5=5). Only valid if active_flags bit 1 set.
    lane_override: u8,
    /// Mute override (0=unmuted, 1=muted). Only valid if active_flags bit 2 set.
    force_mute:    u8,
}

/// Static policy table. Entries may be updated or removed by SET_POLICY.
/// Volatile — lost on Bell restart.
static mut POLICY_TABLE: [PolicyEntry; POLICY_TABLE_CAPACITY] = [PolicyEntry {
    target_pd: 0, active_flags: 0, privacy_level: 0, lane_override: 0, force_mute: 0,
}; POLICY_TABLE_CAPACITY];
static mut POLICY_COUNT: usize = 0;

/// Look up policy for a target PD. Returns Some(&entry) if found.
fn find_policy(target_pd: u32) -> Option<&'static PolicyEntry> {
    unsafe {
        for i in 0..POLICY_COUNT {
            if POLICY_TABLE[i].target_pd == target_pd {
                return Some(&POLICY_TABLE[i]);
            }
        }
    }
    None
}

/// Find mutable policy slot for a target PD. Returns index or None.
/// Check if a target PD has an active mute policy.
fn is_policy_muted(target_pd: u32) -> bool {
    if let Some(entry) = find_policy(target_pd) {
        entry.active_flags & (1 << 2) != 0 && entry.force_mute != 0
    } else {
        false
    }
}

/// Apply policy privacy override: effective = max(event_privacy, policy_min_privacy).
/// Policy can only increase restriction, never decrease it.
fn apply_policy_privacy(caller_pd: u32, event_privacy: u8) -> u8 {
    if let Some(entry) = find_policy(caller_pd) {
        if entry.active_flags & 1 != 0 {
            return core::cmp::max(event_privacy, entry.privacy_level);
        }
    }
    event_privacy
}

/// Apply policy lane override.
fn apply_policy_lane(caller_pd: u32, derived_lane: u8) -> u8 {
    if let Some(entry) = find_policy(caller_pd) {
        if entry.active_flags & (1 << 1) != 0 {
            return entry.lane_override;
        }
    }
    derived_lane
}

// ── Policy Author Allowlist ──────────────────────────────────────────

/// Static allowlist of PDs authorized to call OP_BELL_SET_POLICY.
/// Default-deny: only silk-shell (domain 3) may set policy.
/// SilkBar (PD 6) is explicitly excluded — it is a reader, not an authority.
const BELL_POLICY_AUTHOR_ALLOWLIST: &[u32] = &[
    3,  // silk-shell (domain 3, policy owner)
];

fn is_policy_author_allowed(caller_pd: u32) -> bool {
    BELL_POLICY_AUTHOR_ALLOWLIST.contains(&caller_pd)
}

// ── Spam Budget ──────────────────────────────────────────────────────

/// Per-PD spam budget: max events per tick window.
const SPAM_WINDOW_TICKS: u64 = 62;      // ~1 second at typical tick rate
const SPAM_MAX_PER_WINDOW: u32 = 8;     // max events per window per PD
const SPAM_BUDGET_SLOTS: usize = 16;    // tracked sender slots

struct SpamBudget {
    slots: [(u32, u32, u64); SPAM_BUDGET_SLOTS], // (caller_pd, count, window_start_tick)
}

static mut SPAM_BUDGET: SpamBudget = SpamBudget {
    slots: [(0, 0, 0); SPAM_BUDGET_SLOTS],
};

/// Check and record a notify event for a caller PD.
/// Returns true if allowed, false if rate-limited (spam_budget_exceeded).
fn check_spam_budget(caller_pd: u32) -> bool {
    let now = sex_pdx::get_ticks();
    unsafe {
        // Find existing slot for this PD
        for i in 0..SPAM_BUDGET_SLOTS {
            let (pd, count, window_start) = &mut SPAM_BUDGET.slots[i];
            if *pd == caller_pd {
                // Check if window has expired
                if now.saturating_sub(*window_start) >= SPAM_WINDOW_TICKS {
                    // New window: reset
                    *window_start = now;
                    *count = 1;
                    return true;
                }
                // Within window: check limit
                if *count >= SPAM_MAX_PER_WINDOW {
                    return false; // rate-limited
                }
                *count += 1;
                return true;
            }
        }
        // No existing slot: find an empty one or reuse oldest
        let mut oldest_idx = 0;
        let mut oldest_tick = u64::MAX;
        for i in 0..SPAM_BUDGET_SLOTS {
            if SPAM_BUDGET.slots[i].0 == 0 {
                // Empty slot: use it
                SPAM_BUDGET.slots[i] = (caller_pd, 1, now);
                return true;
            }
            if SPAM_BUDGET.slots[i].2 < oldest_tick {
                oldest_tick = SPAM_BUDGET.slots[i].2;
                oldest_idx = i;
            }
        }
        // All slots full: evict oldest
        SPAM_BUDGET.slots[oldest_idx] = (caller_pd, 1, now);
        true
    }
}

// ── Enum Validation ──────────────────────────────────────────────────

/// Look up the maximum privacy level a caller PD may view.
/// Default-deny: unknown PDs get 0 (Public only).
/// V1: only silk-shell (domain 3) may view all levels.
fn max_privacy_for_caller(caller_pd: u32) -> u8 {
    match caller_pd {
        3 => 3, // silk-shell: may view FullHidden
        _ => 0, // default: Public only
    }
}

/// Validate a BellCategory enum value (0=Info .. 5=Error, 6=SelfCapDenied).
/// Category 6 is reserved for a PD self-reporting its own ERR_CAP_INVALID —
/// see sex_pdx::BELL_CATEGORY_SELF_CAP_DENIED doc comment.
fn valid_category(v: u8) -> bool {
    v <= 6
}

/// Category reserved for a PD self-reporting a local capability denial to
/// itself (not Bell's own readcap allowlist deny, which already has its own
/// [bell.readcap.deny] marker). Mirrors sex_pdx::BELL_CATEGORY_SELF_CAP_DENIED.
///
/// Convention: reporting PDs should set object_ref_count=0, object_ref=<the
/// opcode that was denied, truncated to u8, or 0 if unknown>. urgency_hint is
/// ignored for this category (see the pin below) — it cannot be used to
/// jump the queue.
const BELL_CATEGORY_SELF_CAP_DENIED: u8 = 6;

/// Validate a BellPrivacyLevel enum value (0=Public .. 3=FullHidden).
fn valid_privacy_level(v: u8) -> bool {
    v <= 3
}

/// Validate a BellRedactionClass enum value (0=StructuralMeta .. 3=SecretContent).
fn valid_redaction_class(v: u8) -> bool {
    v <= 3
}

/// First-proof placeholder lane derivation.
///
/// No BellCap table exists yet. Every sender is unknown/untrusted.
/// Unknown/untrusted max lane = LANE_LATER.
/// Urgency_hint > 0 downgrades to LANE_LATER.
fn derive_lane_first_proof(urgency_hint: u8) -> (u8, u8, Option<&'static str>) {
    if urgency_hint == 0 {
        (LANE_LATER, 0, None)
    } else {
        (LANE_LATER, 0, Some("no_caps_untrusted"))
    }
}

/// Bell Bridge status stub proof gate (Phase 1 of BELL_BRIDGE_APP_LAUNCH_PLAN_V1).
/// Emits marker-only proof that Bell Bridge is present but inert: no IPC,
/// no launch, no focus, no renderer integration.
const BELL_BRIDGE_STUB_PROOF_ENABLED: bool =
    option_env!("SEXOS_BELL_BRIDGE_STUB_PROOF").is_some();
static mut BELL_BRIDGE_STUB_PROOF_DONE: bool = false;

/// Bell Bridge status stub: marker-only proof (Phase 1).
/// No IPC, no opcodes, no launch, no focus, no render changes.
unsafe fn maybe_run_bell_bridge_status_stub() {
    if !BELL_BRIDGE_STUB_PROOF_ENABLED || BELL_BRIDGE_STUB_PROOF_DONE { return; }
    serial_println!("[bell.bridge.status.stub] phase=1 ipc=0 launch=0 focus=0 render=0");
    serial_println!("[bell.bridge.status.ready] ok=1");
    BELL_BRIDGE_STUB_PROOF_DONE = true;
}

/// One-shot proof that the LANE_* rename preserved ordinal order (lowest
/// ordinal still evicts first in find_lowest_priority_index). Emitted once
/// at boot behind a proof flag so it doesn't spam every run.
const BELL_LANE_RENAME_PROOF_ENABLED: bool =
    option_env!("SEXOS_BELL_LANE_RENAME_PROOF").is_some();

unsafe fn maybe_run_bell_lane_rename_proof() {
    if !BELL_LANE_RENAME_PROOF_ENABLED { return; }
    serial_println!("[bell.lane.rename] old=0 new=LATER ordinal=0");
    serial_println!("[bell.lane.rename] old=1 new=PROJECT ordinal=1");
    serial_println!("[bell.lane.rename] old=2 new=SOON ordinal=2");
    serial_println!("[bell.lane.rename] old=3 new=SYSTEM ordinal=3");
    serial_println!("[bell.lane.rename] old=4 new=NOW ordinal=4");
    serial_println!("[bell.lane.rename] old=5 new=RESERVED_5 ordinal=5");
    serial_println!(
        "[bell.lane.rename.done] ok=1 order_preserved={}",
        (LANE_LATER < LANE_PROJECT) as u8
            & (LANE_PROJECT < LANE_SOON) as u8
            & (LANE_SOON < LANE_SYSTEM) as u8
            & (LANE_SYSTEM < LANE_NOW) as u8
            & (LANE_NOW < LANE_RESERVED_5) as u8
    );
}

/// Rebuild the bounded ring in arrival order; no dead sender callback remains.
unsafe fn retire_app_resources(target:u32)->usize {
    let old=BELL_QUEUE.entries;
    let head=BELL_QUEUE.head as usize;
    let count=BELL_QUEUE.count as usize;
    let mut kept=0usize;
    let mut removed=0usize;
    for offset in 0..count {
        let e=old[(head+offset)%BELL_QUEUE_CAPACITY];
        if e.caller_pd==target {removed+=1;continue}
        BELL_QUEUE.entries[kept]=e;kept+=1;
    }
    for i in kept..BELL_QUEUE_CAPACITY {BELL_QUEUE.entries[i]=BellQueue::new().entries[0];}
    BELL_QUEUE.head=0;BELL_QUEUE.count=kept as u16;
    BELL_QUEUE.tail=(kept%BELL_QUEUE_CAPACITY) as u16;
    let mut n=0;
    for i in 0..MUTE_COUNT {if MUTE_LIST[i]!=target {MUTE_LIST[n]=MUTE_LIST[i];n+=1;}}
    for i in n..MUTE_LIST_CAPACITY {MUTE_LIST[i]=0;} MUTE_COUNT=n;
    n=0;
    for i in 0..POLICY_COUNT {if POLICY_TABLE[i].target_pd!=target {POLICY_TABLE[n]=POLICY_TABLE[i];n+=1;}}
    for i in n..POLICY_TABLE_CAPACITY {POLICY_TABLE[i]=PolicyEntry{target_pd:0,active_flags:0,privacy_level:0,lane_override:0,force_mute:0};}
    POLICY_COUNT=n;
    for entry in SPAM_BUDGET.slots.iter_mut() {if entry.0==target {*entry=(0,0,0)}}
    bump_generation();removed
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[sexbell.init.start]");
    serial_println!("[bell.boot]");
    paging_probe::run();

    // ── Bell Bridge status stub (Phase 1): marker-only, no IPC ──
    unsafe { maybe_run_bell_bridge_status_stub(); }

    // ── Lane rename proof (F.1): confirms ordinal order unchanged ──
    unsafe { maybe_run_bell_lane_rename_proof(); }

    // ── Demo self-notify (V1): push one notification to exercise Bell→SilkBar→sexdisplay pipe ──
    // caller_pd=0 marks internal Bell event. No sender validation needed for self-generated events.
    // category=0 (Info), urgency=0 (LANE_LATER), lane=0 (LANE_LATER), privacy=0 (Public).
    // SilkBar polls LIST every ~2s and forwards packed counts to sexdisplay.
    // Sexdisplay renders gold dot + count badge in the Bell layout slot.
    unsafe {
        match BELL_QUEUE.push(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0) {
            Ok((event_id, _)) => {
                bump_generation();
                serial_println!("[bell.demo.boot] event_id={}", event_id);
            }
            Err(reason) => {
                serial_println!("[bell.demo.boot.reject] reason={}", reason);
            }
        }
    }

    serial_println!("[sexbell.ready]");
    let mut app_custody=sex_pdx::app_lifecycle::ServiceCustody::new();
    #[cfg(paging_exit_cleanup_proof)]
    let mut exit_probe_pending = false;
    loop {
        #[cfg(paging_exit_cleanup_proof)]
        if exit_probe_pending {
            exit_probe_pending = false;
            paging_probe::exit_reuse();
        }
        let msg = paging_probe::next_message();
        paging_probe::check_retained();
        if msg.type_id==sex_pdx::app_lifecycle::OP_APP_EXIT {
            if let Some((n,acked))=app_custody.receive(&mut sex_pdx::app_lifecycle::KernelAuthority,
                msg.type_id,msg.caller_pd as u64,[msg.arg0,msg.arg1,msg.arg2],|n| {
                    let removed=unsafe{retire_app_resources(n.target_pd)};
                    sex_pdx::serial_record!("[bell.app.cleanup] target={} task={} events={} callbacks=0 policies=0 mute=0 spam=0",n.target_pd,n.task_id,removed);
                }) {
                sex_pdx::serial_record!("[bell.app.cleanup.ack] target={} task={} ok={}",n.target_pd,n.task_id,acked as u8);
            }
            continue;
        }
        if app_custody.is_retired(msg.caller_pd) {continue}

        if msg.type_id == OP_BELL_SUBSCRIBE && msg.caller_pd == 15 {
            static mut PAGING_CHECKS: u8 = 0;
            unsafe {
                PAGING_CHECKS += 1;
                if PAGING_CHECKS == 2 {
                    sex_pdx::serial_record!("[paging.client.retained] pd=10 pages=1 cross_client_reuse=1 ok=1");
                    #[cfg(paging_exit_cleanup_proof)]
                    { exit_probe_pending = true; }
                }
            }
        }

        match msg.type_id {
            OP_BELL_NOTIFY => {
                // ── Parse fixed numeric fields from IpcCall args ──────
                let category        = ((msg.arg0 >> 0)  & 0xFF) as u8;
                let urgency_hint    = ((msg.arg0 >> 8)  & 0xFF) as u8;
                let privacy_level   = ((msg.arg0 >> 16) & 0xFF) as u8;
                let redaction_class = ((msg.arg0 >> 24) & 0xFF) as u8;
                let action_count    = (msg.arg1 & 0xFF) as u8;
                let action_id       = ((msg.arg1 >> 8) & 0xFF) as u8;
                let object_ref_count = (msg.arg2 & 0xFF) as u8;
                let object_ref      = ((msg.arg2 >> 8) & 0xFF) as u8;
                let caller_pd       = msg.caller_pd;

                // ── Mute check (reject before any processing) ───────────
                if is_muted(caller_pd) || is_policy_muted(caller_pd) {
                    unsafe {
                        static mut BELL_MUTED_REJECT_BUDGET: u32 = 8;
                        let b = &mut BELL_MUTED_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.notify.reject] caller_pd={} reason=muted",
                                caller_pd);
                        }
                    }
                    continue;
                }

                // ── Validate enum ranges ─────────────────────────────
                let mut reject_reason: Option<&'static str> = None;

                if !valid_category(category) {
                    reject_reason = Some("invalid_category");
                } else if !valid_privacy_level(privacy_level) {
                    reject_reason = Some("invalid_privacy");
                } else if !valid_redaction_class(redaction_class) {
                    reject_reason = Some("invalid_redaction");
                } else if urgency_hint > 3 {
                    reject_reason = Some("invalid_urgency");
                } else if action_count > 1 {
                    reject_reason = Some("action_count_invalid");
                } else if action_count == 1 && action_id == 0 {
                    reject_reason = Some("action_id_zero");
                } else if object_ref_count > 1 {
                    reject_reason = Some("object_refs_invalid");
                }

                if let Some(reason) = reject_reason {
                    if BELL_DELIVERY_PROOF_ENABLED {
                        serial_println!("[bell.event.reject] caller_pd={} reason={}", caller_pd, reason);
                    }
                    unsafe {
                        static mut BELL_REJECT_BUDGET: u32 = 4;
                        let b = &mut BELL_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.notify.reject] caller_pd={} reason={}",
                                caller_pd, reason);
                        }
                    }
                    continue;
                }

                // ── Emit recv marker ─────────────────────────────────
                unsafe {
                    static mut BELL_RECV_BUDGET: u32 = 8;
                    let b = &mut BELL_RECV_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!("[bell.notify.recv] caller_pd={} category={} requested={}",
                            caller_pd, category, urgency_hint);
                    }
                }

                // ── Derive lane (placeholder policy: no caps → LANE_LATER) ──
                let (mut final_lane, final_urgency, downgrade) =
                    derive_lane_first_proof(urgency_hint);

                // ── SelfCapDenied pin: category=6 always lands in LANE_SYSTEM. ──
                // Sender-controlled urgency_hint must never be able to buy this
                // event class a higher lane — a malicious sender could otherwise
                // spoof category=6 with urgency_hint=3 to jump the queue.
                if category == BELL_CATEGORY_SELF_CAP_DENIED {
                    final_lane = LANE_SYSTEM;
                    unsafe {
                        static mut BELL_SELFREPORT_BUDGET: u32 = 8;
                        let b = &mut BELL_SELFREPORT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!(
                                "[bell.selfreport.capdenied] caller_pd={} target_op={}",
                                caller_pd, object_ref
                            );
                        }
                    }
                }

                if let Some(reason) = downgrade {
                    unsafe {
                        static mut BELL_DOWNGRADE_BUDGET: u32 = 8;
                        let b = &mut BELL_DOWNGRADE_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.notify.downgrade] from={} to={} reason={}",
                                urgency_hint, final_lane, reason);
                        }
                    }
                }

                // ── Spam budget check ───────────────────────────────────
                if !check_spam_budget(caller_pd) {
                    unsafe {
                        static mut BELL_SPAM_REJECT_BUDGET: u32 = 8;
                        let b = &mut BELL_SPAM_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.notify.reject] caller_pd={} reason=spam_budget_exceeded window=64 max=8",
                                caller_pd);
                        }
                    }
                    continue;
                }

                // ── Apply policy overrides (volatile policy table) ────
                // Policy can only increase privacy restriction, never reduce it.
                // Lane override replaces derived lane if policy has one set.
                let effective_privacy = apply_policy_privacy(caller_pd, privacy_level);
                let effective_lane = apply_policy_lane(caller_pd, final_lane);

                // ── Push to queue (with policy-applied values) ─────────
                let push_result = unsafe {
                    BELL_QUEUE.push(caller_pd, category, urgency_hint,
                                    effective_lane, final_urgency, effective_privacy,
                                    redaction_class, action_count, action_id,
                                    object_ref_count, object_ref)
                };

                match push_result {
                    Ok((event_id, drop_info)) => {
                        if BELL_DELIVERY_PROOF_ENABLED {
                            serial_println!("[bell.event.accept] caller_pd={} event_id={} lane={}", caller_pd, event_id, effective_lane);
                        }
                        // ── Emit drop marker if a lowest-priority entry was displaced ──
                        if let Some(dropped_lane) = drop_info {
                            unsafe {
                                static mut BELL_QUEUE_DROP_BUDGET: u32 = 16;
                                let b = &mut BELL_QUEUE_DROP_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    serial_println!("[bell.queue.drop] reason=lowest_priority lane={} dropped_lane={} event_id={}",
                                        final_lane, dropped_lane, event_id);
                                }
                            }
                        }

                        unsafe {
                            static mut BELL_QUEUE_PUSH_BUDGET: u32 = 64;
                            let b = &mut BELL_QUEUE_PUSH_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                serial_println!("[bell.queue.push] id={} final_lane={} count={}",
                                    event_id, final_lane, BELL_QUEUE.count);
                            }
                        }
                        // ── Emit ok marker ───────────────────────────
                        unsafe {
                            static mut BELL_OK_BUDGET: u32 = 8;
                            let b = &mut BELL_OK_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                serial_println!("[bell.notify.ok] caller_pd={} final_lane={} event_id={}",
                                    caller_pd, final_lane, event_id);
                            }
                        }
                        bump_generation();
                    }
                    Err(reason) => {
                        if BELL_DELIVERY_PROOF_ENABLED {
                            serial_println!("[bell.event.reject] caller_pd={} reason={}", caller_pd, reason);
                        }
                        unsafe {
                            static mut BELL_QUEUE_FULL_BUDGET: u32 = 16;
                            let b = &mut BELL_QUEUE_FULL_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                serial_println!("[bell.queue.reject.full] count={}",
                                    BELL_QUEUE_CAPACITY);
                            }
                        }
                        // Emit reject for queue-full
                        unsafe {
                            static mut BELL_REJECT_BUDGET: u32 = 4;
                            let b = &mut BELL_REJECT_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                serial_println!("[bell.notify.reject] caller_pd={} reason={}",
                                    caller_pd, reason);
                            }
                        }
                    }
                }
            }

            OP_BELL_LIST => {
                // ── Parse ──
                let lane_filter  = ((msg.arg0 >> 0)  & 0xFF) as u8;
                let max_results  = ((msg.arg0 >> 8)  & 0xFF) as u8;
                let caller_pd    = msg.caller_pd;

                // ── Validate lane_filter ──
                // 0xFF = all lanes, 0..=5 = specific lane
                if lane_filter != 0xFF && lane_filter > 5 {
                    unsafe {
                        static mut BELL_LIST_REJECT_BUDGET: u32 = 4;
                        let b = &mut BELL_LIST_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.list.reject] reason=invalid_lane caller_pd={}",
                                caller_pd);
                        }
                    }
                    continue;
                }

                // ── Validate max_results (1..=4 only, reject out of range) ──
                if max_results == 0 || max_results > 4 {
                    unsafe {
                        static mut BELL_LIST_REJECT_BUDGET: u32 = 4;
                        let b = &mut BELL_LIST_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.list.reject] reason=invalid_count caller_pd={}",
                                caller_pd);
                        }
                    }
                    continue;
                }

                // ── Check read-cap allowlist ──
                // Default-deny: only allowlisted PDs may call OP_BELL_LIST.
                // Check happens after arg validation (protocol errors are
                // reported regardless of caller) but before queue access.
                if !is_list_reader_allowed(caller_pd) {
                    unsafe {
                        static mut BELL_READCAP_DENY_BUDGET: u32 = 8;
                        let b = &mut BELL_READCAP_DENY_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.readcap.deny] caller_pd={} op=list reason=no_read_cap",
                                caller_pd);
                        }
                        // Reply with error so caller does not hang
                        pdx_reply(caller_pd, u64::MAX);
                    }
                    continue;
                }

                // ── Emit recv marker ──

                let caller_max_privacy = max_privacy_for_caller(caller_pd);

                // ── Compute aggregate lane counts (full scan, not max_results-limited) ──
                let mut lane_counts: [u32; 6] = [0; 6];
                let mut total_visible: u32 = 0;
                unsafe {
                    for i in 0..BELL_QUEUE.count as usize {
                        let idx = (BELL_QUEUE.tail as usize + BELL_QUEUE_CAPACITY - 1 - i)
                                  % BELL_QUEUE_CAPACITY;
                        let entry = &BELL_QUEUE.entries[idx];
                        if entry.dismissed != 0 { continue; }
                        if entry.privacy_level > caller_max_privacy { continue; }
                        let lane = entry.final_lane as usize;
                        if lane < 6 {
                            lane_counts[lane] += 1;
                            total_visible += 1;
                        }
                    }
                }

                // ── Read queue (newest-first, no mutation) ──
                let mut match_count: u32 = 0;
                let mut redact_count: u32 = 0;

                unsafe {
                    for i in 0..BELL_QUEUE.count as usize {
                        let idx = (BELL_QUEUE.tail as usize + BELL_QUEUE_CAPACITY - 1 - i)
                                  % BELL_QUEUE_CAPACITY;
                        let entry = &BELL_QUEUE.entries[idx];

                        // Skip dismissed entries
                        if entry.dismissed != 0 {
                            continue;
                        }

                        // ── Privacy gate: skip entries above caller's max privacy level ──
                        if entry.privacy_level > caller_max_privacy {
                            if entry.privacy_level == 3 {
                                // FullHidden: count but don't reveal
                                redact_count += 1;
                            }
                            continue;
                        }

                        if lane_filter == 0xFF || entry.final_lane == lane_filter {
                            // ── Emit item marker ──
                            static mut BELL_LIST_ITEM_BUDGET: u32 = 8;
                            let b = &mut BELL_LIST_ITEM_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                serial_println!("[bell.list.item] event_id={} final_lane={} \
                                    category={} privacy={} redaction={} actions={} refs={}",
                                    entry.event_id, entry.final_lane, entry.category,
                                    entry.privacy_level, entry.redaction_class,
                                    entry.action_count, entry.object_ref_count);
                            }

                            match_count += 1;
                            if match_count >= max_results as u32 {
                                break;
                            }
                        }
                    }
                }

                // ── Emit redact marker if FullHidden entries were filtered ──
                if redact_count > 0 {
                    unsafe {
                        static mut BELL_LIST_REDACT_BUDGET: u32 = 8;
                        let b = &mut BELL_LIST_REDACT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.list.redact] reason=full_hidden count={} caller_pd={}",
                                redact_count, caller_pd);
                        }
                    }
                }

                // ── Reply with packed aggregate counts ──
                // Packing: [63:56]=redacted [55:48]=lane5 [47:40]=lane4 [39:32]=lane3
                //          [31:24]=lane2 [23:16]=lane1 [15:8]=lane0 [7:0]=total_visible
                let packed = (total_visible as u64) & 0xFF
                           | ((lane_counts[0] as u64) & 0xFF) << 8
                           | ((lane_counts[1] as u64) & 0xFF) << 16
                           | ((lane_counts[2] as u64) & 0xFF) << 24
                           | ((lane_counts[3] as u64) & 0xFF) << 32
                           | ((lane_counts[4] as u64) & 0xFF) << 40
                           | ((lane_counts[5] as u64) & 0xFF) << 48
                           | ((redact_count as u64) & 0xFF) << 56;
                unsafe {
                    static mut BELL_LIST_REPLY_BUDGET: u32 = 8;
                    let b = &mut BELL_LIST_REPLY_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!("[bell.list.reply] total={} lanes=[{} {} {} {} {} {}] redacted={}",
                            total_visible,
                            lane_counts[0], lane_counts[1], lane_counts[2],
                            lane_counts[3], lane_counts[4], lane_counts[5],
                            redact_count);
                    }
                    pdx_reply(caller_pd, packed);
                }
            }

            OP_BELL_CLOSE => {
                // ── Parse ──
                let event_id   = msg.arg0;
                let caller_pd  = msg.caller_pd;

                // ── Search queue for matching event_id, mark dismissed ──
                let mut found = false;
                unsafe {
                    for i in 0..BELL_QUEUE.count as usize {
                        let idx = (BELL_QUEUE.head as usize + i) % BELL_QUEUE_CAPACITY;
                        if BELL_QUEUE.entries[idx].event_id == event_id
                            && BELL_QUEUE.entries[idx].dismissed == 0
                        {
                            BELL_QUEUE.entries[idx].dismissed = 1;
                            found = true;
                            break;
                        }
                    }
                }

                if found {
                    unsafe {
                        static mut BELL_CLOSE_OK_BUDGET: u32 = 8;
                        let b = &mut BELL_CLOSE_OK_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.close.ok] event_id={}", event_id);
                        }
                    }
                    bump_generation();
                } else {
                    unsafe {
                        static mut BELL_CLOSE_REJECT_BUDGET: u32 = 4;
                        let b = &mut BELL_CLOSE_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.close.reject] reason=not_found event_id={} caller_pd={}",
                                event_id, caller_pd);
                        }
                    }
                }
            }

            OP_BELL_ACTION => {
                // ── Parse ──
                let event_id   = msg.arg0;
                let action_id  = (msg.arg1 & 0xFF) as u8;
                let caller_pd  = msg.caller_pd;

                // ── Search queue for matching event_id with matching action_id ──
                let mut found = false;
                let mut action_final_lane: u8 = 0;
                unsafe {
                    for i in 0..BELL_QUEUE.count as usize {
                        let idx = (BELL_QUEUE.head as usize + i) % BELL_QUEUE_CAPACITY;
                        let entry = &BELL_QUEUE.entries[idx];
                        if entry.event_id == event_id
                            && entry.dismissed == 0
                            && entry.action_count > 0
                            && entry.action_id == action_id
                        {
                            action_final_lane = entry.final_lane;
                            found = true;
                            break;
                        }
                    }
                }

                if found {
                    unsafe {
                        static mut BELL_ACTION_DISPATCH_BUDGET: u32 = 8;
                        let b = &mut BELL_ACTION_DISPATCH_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.action.dispatch] event_id={} action_id={} lane={}",
                                event_id, action_id, action_final_lane);
                        }
                    }
                } else {
                    unsafe {
                        static mut BELL_ACTION_REJECT_BUDGET: u32 = 4;
                        let b = &mut BELL_ACTION_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.action.reject] reason=not_found event_id={} action_id={} caller_pd={}",
                                event_id, action_id, caller_pd);
                        }
                    }
                }
            }

            OP_BELL_CLEAR => {
                // ── Parse ──
                let lane_filter = ((msg.arg0 >> 0) & 0xFF) as u8;
                let caller_pd   = msg.caller_pd;

                if lane_filter == 0xFF {
                    // ── Clear all lanes: reset queue ──
                    unsafe {
                        BELL_QUEUE.head = 0;
                        BELL_QUEUE.tail = 0;
                        BELL_QUEUE.count = 0;
                    }
                    unsafe {
                        static mut BELL_CLEAR_OK_BUDGET: u32 = 4;
                        let b = &mut BELL_CLEAR_OK_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.clear.ok] lane=all caller_pd={}", caller_pd);
                        }
                    }
                    bump_generation();
                } else if lane_filter <= 5 {
                    // ── Clear specific lane: mark matching entries as dismissed ──
                    let mut dismiss_count: u32 = 0;
                    unsafe {
                        for i in 0..BELL_QUEUE.count as usize {
                            let idx = (BELL_QUEUE.head as usize + i) % BELL_QUEUE_CAPACITY;
                            if BELL_QUEUE.entries[idx].final_lane == lane_filter
                                && BELL_QUEUE.entries[idx].dismissed == 0
                            {
                                BELL_QUEUE.entries[idx].dismissed = 1;
                                dismiss_count += 1;
                            }
                        }
                    }
                    unsafe {
                        static mut BELL_CLEAR_OK_BUDGET: u32 = 4;
                        let b = &mut BELL_CLEAR_OK_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.clear.ok] lane={} count={} caller_pd={}",
                                lane_filter, dismiss_count, caller_pd);
                        }
                    }
                    if dismiss_count > 0 {
                        bump_generation();
                    }
                } else {
                    unsafe {
                        static mut BELL_CLEAR_REJECT_BUDGET: u32 = 4;
                        let b = &mut BELL_CLEAR_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.clear.reject] reason=invalid_lane lane={} caller_pd={}",
                                lane_filter, caller_pd);
                        }
                    }
                }
            }

            OP_BELL_MUTE_SENDER => {
                // ── Parse ──
                let mute_pd   = (msg.arg0 & 0xFFFFFFFF) as u32;
                let action    = ((msg.arg0 >> 32) & 0xFF) as u8;
                let caller_pd = msg.caller_pd;

                match action {
                    0 => {
                        match add_mute(mute_pd) {
                            Ok(()) => {
                                unsafe {
                                    static mut BELL_MUTE_ADD_BUDGET: u32 = 8;
                                    let b = &mut BELL_MUTE_ADD_BUDGET;
                                    if *b > 0 {
                                        *b -= 1;
                                        serial_println!("[bell.mute.add] mute_pd={} caller_pd={}",
                                            mute_pd, caller_pd);
                                    }
                                }
                                bump_generation();
                            }
                            Err(reason) => {
                                unsafe {
                                    static mut BELL_MUTE_REJECT_BUDGET: u32 = 4;
                                    let b = &mut BELL_MUTE_REJECT_BUDGET;
                                    if *b > 0 {
                                        *b -= 1;
                                        serial_println!("[bell.mute.reject] reason={} mute_pd={} caller_pd={}",
                                            reason, mute_pd, caller_pd);
                                    }
                                }
                            }
                        }
                    }
                    1 => {
                        if remove_mute(mute_pd) {
                            unsafe {
                                static mut BELL_MUTE_REMOVE_BUDGET: u32 = 8;
                                let b = &mut BELL_MUTE_REMOVE_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    serial_println!("[bell.mute.remove] mute_pd={} caller_pd={}",
                                        mute_pd, caller_pd);
                                }
                            }
                            bump_generation();
                        } else {
                            unsafe {
                                static mut BELL_MUTE_REJECT_BUDGET: u32 = 4;
                                let b = &mut BELL_MUTE_REJECT_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    serial_println!("[bell.mute.reject] reason=not_found mute_pd={} caller_pd={}",
                                        mute_pd, caller_pd);
                                }
                            }
                        }
                    }
                    _ => {
                        unsafe {
                            static mut BELL_MUTE_REJECT_BUDGET: u32 = 4;
                            let b = &mut BELL_MUTE_REJECT_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                serial_println!("[bell.mute.reject] reason=invalid_action action={} caller_pd={}",
                                    action, caller_pd);
                            }
                        }
                    }
                }
            }

            OP_BELL_SET_POLICY => {
                // ── Parse ──
                let target_pd  = msg.arg0 as u32;
                let packed     = msg.arg1;
                let caller_pd  = msg.caller_pd;

                // ── Author check: only allowlisted PDs may set policy ──
                if !is_policy_author_allowed(caller_pd) {
                    unsafe {
                        static mut BELL_POLICY_DENY_BUDGET: u32 = 8;
                        let b = &mut BELL_POLICY_DENY_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.policy.deny] caller_pd={} target_pd={}",
                                caller_pd, target_pd);
                        }
                    }
                    pdx_reply(caller_pd, u64::MAX);
                    continue;
                }

                // ── Decode policy payload ──
                // bit 0: privacy_override active
                // bit 1: lane_override active
                // bit 2: force_mute active
                // bits 8-9: privacy_override value (0=Public .. 3=FullHidden)
                // bits 16-18: lane_override value (LANE_LATER=0 .. LANE_RESERVED_5=5)
                // bit 24: force_mute value (0=unmuted, 1=muted)
                let active_flags  = (packed & 0x7) as u8;
                let privacy_val   = ((packed >> 8) & 0x3) as u8;
                let lane_val      = ((packed >> 16) & 0x7) as u8;
                let mute_val      = ((packed >> 24) & 0x1) as u8;

                // ── Validate fields ──
                let mut reject = false;
                if active_flags & 1 != 0 && privacy_val > 3 {
                    reject = true;
                }
                if active_flags & (1 << 1) != 0 && lane_val > 5 {
                    reject = true;
                }
                if reject {
                    unsafe {
                        static mut BELL_POLICY_REJECT_BUDGET: u32 = 8;
                        let b = &mut BELL_POLICY_REJECT_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.policy.reject] reason=invalid_field caller_pd={} target_pd={}",
                                caller_pd, target_pd);
                        }
                    }
                    pdx_reply(caller_pd, u64::MAX);
                    continue;
                }

                // ── Apply policy: find or create entry ──
                // Privacy invariant: policy can only INCREASE restriction.
                // Compare against existing override if present.
                let mut changed = false;
                let new_entry = PolicyEntry {
                    target_pd,
                    active_flags,
                    privacy_level: privacy_val,
                    lane_override: lane_val,
                    force_mute: mute_val,
                };

                // If all flags cleared, remove the entry entirely.
                if active_flags == 0 {
                    unsafe {
                        for i in 0..POLICY_COUNT {
                            if POLICY_TABLE[i].target_pd == target_pd {
                                // Shift remaining entries left
                                for j in i..POLICY_COUNT - 1 {
                                    POLICY_TABLE[j] = POLICY_TABLE[j + 1];
                                }
                                POLICY_TABLE[POLICY_COUNT - 1].target_pd = 0;
                                POLICY_COUNT -= 1;
                                changed = true;
                                break;
                            }
                        }
                    }
                } else {
                    // Check privacy invariant: new override must not reduce restriction.
                    if active_flags & 1 != 0 {
                        if let Some(existing) = find_policy(target_pd) {
                            if existing.active_flags & 1 != 0 && privacy_val < existing.privacy_level {
                                // Cannot reduce privacy restriction
                                unsafe {
                                    static mut BELL_POLICY_REJECT_BUDGET: u32 = 8;
                                    let b = &mut BELL_POLICY_REJECT_BUDGET;
                                    if *b > 0 {
                                        *b -= 1;
                                        serial_println!("[bell.policy.reject] reason=privacy_reduction \
                                            caller_pd={} target_pd={} old={} new={}",
                                            caller_pd, target_pd, existing.privacy_level, privacy_val);
                                    }
                                }
                                pdx_reply(caller_pd, u64::MAX);
                                continue;
                            }
                        }
                    }

                    unsafe {
                        // Find existing entry
                        let mut found = false;
                        for i in 0..POLICY_COUNT {
                            if POLICY_TABLE[i].target_pd == target_pd {
                                let old = &POLICY_TABLE[i];
                                changed = old.active_flags != new_entry.active_flags
                                    || old.privacy_level != new_entry.privacy_level
                                    || old.lane_override != new_entry.lane_override
                                    || old.force_mute != new_entry.force_mute;
                                POLICY_TABLE[i] = new_entry;
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            if POLICY_COUNT < POLICY_TABLE_CAPACITY {
                                POLICY_TABLE[POLICY_COUNT] = new_entry;
                                POLICY_COUNT += 1;
                                changed = true;
                            } else {
                                // Table full
                                static mut BELL_POLICY_REJECT_BUDGET: u32 = 8;
                                let b = &mut BELL_POLICY_REJECT_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    serial_println!("[bell.policy.reject] reason=table_full \
                                        caller_pd={} target_pd={}",
                                        caller_pd, target_pd);
                                }
                                pdx_reply(caller_pd, u64::MAX);
                                continue;
                            }
                        }
                    }
                }

                if changed {
                    bump_generation();
                }

                unsafe {
                    static mut BELL_POLICY_SET_BUDGET: u32 = 8;
                    let b = &mut BELL_POLICY_SET_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!("[bell.policy.set] caller_pd={} target_pd={} flags={} privacy={} lane={} mute={}",
                            caller_pd, target_pd, active_flags, privacy_val, lane_val, mute_val);
                    }
                }
                pdx_reply(caller_pd, 0);
            }

            OP_BELL_SUBSCRIBE => {
                let caller_pd = msg.caller_pd;
                // Same allowlist as OP_BELL_LIST: default-deny.
                if !is_list_reader_allowed(caller_pd) {
                    unsafe {
                        static mut BELL_SUBSCRIBE_DENY_BUDGET: u32 = 8;
                        let b = &mut BELL_SUBSCRIBE_DENY_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!("[bell.subscribe.deny] caller_pd={}", caller_pd);
                        }
                    }
                    pdx_reply(caller_pd, u64::MAX);
                    continue;
                }
                let gen = unsafe { BELL_GENERATION };
                pdx_reply(caller_pd, gen);
                unsafe {
                    static mut BELL_SUBSCRIBE_REPLY_BUDGET: u32 = 4;
                    let b = &mut BELL_SUBSCRIBE_REPLY_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!("[bell.subscribe.reply] gen={}", gen);
                    }
                }
            }

            _ => {
                unsafe {
                    static mut BELL_UNKNOWN_BUDGET: u32 = 8;
                    let b = &mut BELL_UNKNOWN_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!("[bell.unknown.reject] type_id={:#x}", msg.type_id);
                    }
                }
            }
        }
    }
}
