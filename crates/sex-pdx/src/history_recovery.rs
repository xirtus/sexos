//! Immutable BOOT-sanctioned data recovery relationship, not handle transfer.
//! Catalog targets describe the expected roster; only the held query supplies
//! authority and the registered main-task identities. These are not per-request
//! authenticated task identities.
pub const SLOT: usize = 53;
pub const SYS_QUERY: u64 = 67;
pub const VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Participant { pub target: u32, pub task: u32 }
impl Participant { pub const EMPTY: Self = Self { target: 0, task: 0 }; }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryGrant<const N: usize> {
    pub holder: u32,
    pub logical: u32,
    pub participants: [Participant; N],
    pub base: u64,
    pub sectors: u64,
}
pub type Grant = RecoveryGrant<{ crate::app_catalog::SPINDLE_COUNT }>;

impl<const N: usize> RecoveryGrant<N> {
    pub fn valid(&self) -> bool {
        if N == 0 || N > 64 || self.holder == 0 || self.logical == 0
            || self.sectors != 18 || self.base.checked_add(self.sectors).is_none() {
            return false;
        }
        for (i, p) in self.participants.iter().enumerate() {
            if p.target == 0 || p.task == 0 || self.participants[..i].iter()
                .any(|old| old.target == p.target || old.task == p.task) { return false; }
        }
        true
    }
    pub fn participant(&self, target: u32) -> Option<Participant> {
        if !self.valid() { return None; }
        self.participants.iter().find(|p| p.target == target).copied()
    }
    pub fn permits(&self, holder: u32, target: u32, task: u32) -> bool {
        self.holder == holder && self.participant(target) == Some(Participant { target, task })
    }
    pub fn query(&self, holder: u32, kind: u64) -> [u64; 5] {
        if !self.valid() || self.holder != holder { return [3, 0, 0, 0, 0]; }
        match kind {
            0 => [0, self.logical as u64, self.holder as u64, N as u64, VERSION],
            1 => [0, self.base, self.sectors, 0, 0],
            k if k >= 2 && k - 2 < N as u64 => {
                let i = (k - 2) as usize;
                let p = self.participants[i];
                [0, p.target as u64, p.task as u64, i as u64, 0]
            }
            _ => [1, 0, 0, 0, 0],
        }
    }
}

/// Reconstruct metadata only from a held capability query, rejecting an
/// incomplete or differently ordered roster before publishing any result.
pub fn decode_for<const N: usize>(
    expected_holder: u32, expected_logical: u32, expected_targets: &[u32; N],
    mut query: impl FnMut(u64) -> Result<[u64; 4], u64>,
) -> Result<RecoveryGrant<N>, u64> {
    if N == 0 || N > 64 || expected_holder == 0 || expected_logical == 0 { return Err(1); }
    let header = query(0)?;
    if header != [expected_logical as u64, expected_holder as u64, N as u64, VERSION] {
        return Err(1);
    }
    let extent = query(1)?;
    if extent[2..] != [0, 0] { return Err(1); }
    let mut grant = RecoveryGrant {
        holder: expected_holder, logical: expected_logical,
        participants: [Participant::EMPTY; N], base: extent[0], sectors: extent[1],
    };
    for (i, expected) in expected_targets.iter().enumerate() {
        let raw = query(2 + i as u64)?;
        if raw[0] != *expected as u64 || raw[2] != i as u64 || raw[3] != 0 { return Err(1); }
        grant.participants[i] = Participant {
            target: u32::try_from(raw[0]).map_err(|_| 1u64)?,
            task: u32::try_from(raw[1]).map_err(|_| 1u64)?,
        };
    }
    if !grant.valid() { return Err(3); }
    Ok(grant)
}

#[cfg(target_arch = "x86_64")]
pub fn get(slot: usize) -> Result<Grant, u64> {
    fn query(slot: usize, kind: u64) -> Result<[u64; 4], u64> {
        let (status, a, b, c, d): (u64, u64, u64, u64, u64);
        unsafe {
            core::arch::asm!("syscall", inlateout("rax") SYS_QUERY => status,
                in("rdi") slot as u64, inlateout("rsi") kind => a,
                lateout("rdx") b, lateout("r10") c, lateout("r8") d,
                out("rcx") _, out("r11") _, options(nostack));
        }
        if status != 0 {
            return Err(if [a, b, c, d] == [0; 4] { status } else { 1 });
        }
        Ok([a, b, c, d])
    }
    decode_for(11, crate::app_catalog::SPINDLE_TARGETS[0],
        &crate::app_catalog::SPINDLE_TARGETS, |kind| query(slot, kind))
}
