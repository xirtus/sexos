// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! NVMe Autonomous Power State Transitions (APST): the table that lets an idle SSD drop
//! into its low-power states by itself, which on a laptop is most of the drive's
//! battery cost. Same policy as Linux's defaults: a non-operational state whose entry
//! plus exit latency is at most 15 ms is entered after 100 ms idle, one of at most
//! 100 ms after 2 s idle; states whose entry plus exit latency exceeds the 100 ms
//! ceiling are not used.

/// Identify Controller offsets.
pub const ID_NPSS: usize = 263;
pub const ID_APSTA: usize = 265;
pub const ID_PSD: usize = 2048;
pub const PSD_BYTES: usize = 32;
/// Set Features feature id for APST, and its 256-byte table.
pub const FID_APST: u32 = 0x0C;
pub const ENTRIES: usize = 32;

/// Linux's default_ps_max_latency_us, on entry plus exit.
pub const MAX_EXIT_LATENCY_US: u64 = 100_000;
const LATENCY_TOLERANCE_US: [u64; 2] = [15_000, 100_000];
const IDLE_TIMEOUT_MS: [u64; 2] = [100, 2_000];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerState { pub non_operational: bool, pub entry_us: u32, pub exit_us: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub table: [u64; ENTRIES],
    /// Deepest state a transition may reach.
    pub deepest: u8,
    /// Worst entry + exit latency among the states used, in microseconds.
    pub worst_latency_us: u64,
}

fn u32_at(id: &[u8], at: usize) -> u32 { u32::from_le_bytes([id[at], id[at + 1], id[at + 2], id[at + 3]]) }

/// Whether the controller supports APST, and its power state descriptors.
pub fn parse(id: &[u8]) -> Option<([PowerState; ENTRIES], u8)> {
    if id.len() < ID_PSD + ENTRIES * PSD_BYTES || id[ID_APSTA] & 1 == 0 { return None; }
    let npss = id[ID_NPSS].min((ENTRIES - 1) as u8);
    let mut states = [PowerState { non_operational: false, entry_us: 0, exit_us: 0 }; ENTRIES];
    for (i, s) in states.iter_mut().enumerate().take(npss as usize + 1) {
        let d = ID_PSD + i * PSD_BYTES;
        *s = PowerState { non_operational: id[d + 3] & 0x02 != 0, entry_us: u32_at(id, d + 4), exit_us: u32_at(id, d + 8) };
    }
    Some((states, npss))
}

/// The APST table, or None when no state is worth using. `skip_deepest` is for
/// drives known to misbehave in their deepest state.
pub fn plan(states: &[PowerState; ENTRIES], npss: u8, skip_deepest: bool) -> Option<Plan> {
    let mut table = [0u64; ENTRIES];
    let mut target = 0u64;
    let mut tier = 0usize;
    let mut deepest = None;
    let mut worst = 0u64;
    for state in (0..=npss.min((ENTRIES - 1) as u8) as usize).rev() {
        if target != 0 { table[state] = target; }
        if state == npss as usize && skip_deepest { continue; }
        let s = states[state];
        let total = s.entry_us as u64 + s.exit_us as u64;
        // Like Linux, a state over the ceiling is passed over before the tier search,
        // so it cannot use up the tiers and hide shallower states.
        if !s.non_operational || total > MAX_EXIT_LATENCY_US { continue; }
        while tier < LATENCY_TOLERANCE_US.len() && total > LATENCY_TOLERANCE_US[tier] { tier += 1; }
        if tier == LATENCY_TOLERANCE_US.len() { continue; }
        target = ((state as u64) << 3) | (IDLE_TIMEOUT_MS[tier] << 8);
        deepest.get_or_insert(state as u8);
        worst = worst.max(total);
    }
    deepest.map(|deepest| Plan { table, deepest, worst_latency_us: worst })
}

/// Drives whose deepest state is unreliable on some laptops (Linux's
/// NVME_QUIRK_NO_DEEPEST_PS list for laptop parts).
pub fn skip_deepest(vendor: u16, device: u16) -> bool {
    matches!((vendor, device), (0x144d, 0xa802) | (0x1c5c, 0x1504) | (0x1987, 0x5016))
}
