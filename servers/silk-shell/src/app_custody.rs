//! Shell policy over immutable kernel catalog identities. Terminal entries never
//! become live again; the finite visual tombstone ring is not authority.
/// Close kinds and decline reason of the app lifecycle wire; main.rs checks
/// at compile time that they equal sex_pdx::app_lifecycle.
pub const CLOSE_ASK: u64 = 0;
pub const CLOSE_CONFIRMED: u64 = 1;
pub const DECLINE_UNSAVED: u64 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub slot: usize, pub target: u32, pub task: u32, pub surface: u64,
    pub pending: bool, pub dead: bool, pub retired: bool, pub acked: bool,
    pub close_pending: bool, pub close_attempt: Option<u64>,
    /// The app declined an ordinary close (unsaved work); the next user
    /// Close is sent as confirmed.
    pub close_declined: bool,
    pub registration_reply: bool,
    pub bounds: Option<(i32, i32, u32, u32)>,
}
impl Entry {
    pub const fn new(slot: usize, target: u32, task: u32, surface: u64) -> Self {
        Self { slot, target, task, surface, pending:false, dead:false,
            retired:false, acked:false, close_pending:false, close_attempt:None, close_declined:false, registration_reply:false, bounds:None }
    }
    /// Fixed BOOT catalog association, not a caller-supplied ownership bind.
    pub fn shell_surface(&self) -> Option<u64> {
        crate::app_catalog::by_surface(self.target, self.surface).and_then(|i| i.shell_surface)
    }
    /// Successor frames acquire geometry through app registration. Original
    /// Shell frames retain their existing pre-registration creation path.
    pub fn is_successor_frame(&self,sid:u64)->bool {
        crate::app_catalog::by_surface(self.target,self.surface).is_some_and(|row|
            row.app==crate::app_catalog::App::Spindle&&row.ordinal!=0&&row.shell_surface==Some(sid))
    }
    pub fn owns_surface(&self, sid: u64) -> bool { self.surface == sid || self.shell_surface() == Some(sid) }
    pub fn close_slot(&self) -> Option<u64> {
        crate::app_catalog::by_surface(self.target, self.surface).map(|i| i.close_slot)
    }
    pub fn request_close(&mut self) -> bool {
        if self.dead || self.close_slot().is_none() {return false}
        self.close_pending=true; true
    }
    pub fn close_due(&self,tick:u64)->bool {
        self.close_pending && !self.dead && self.close_attempt.map_or(true,|last|tick.wrapping_sub(last)>=16)
    }
    pub fn attempted_close(&mut self,tick:u64) { self.close_attempt=Some(tick); }
    pub fn close_kind(&self) -> u64 {
        if self.close_declined {CLOSE_CONFIRMED} else {CLOSE_ASK}
    }
    /// Only this live incarnation, only while a close is pending, and only
    /// for unsaved work. A decline ends the pending close. An app may decline
    /// a confirmed close too, when it has new unsaved work since its warning.
    pub fn decline_close(&mut self, caller: u32, args: [u64;3]) -> bool {
        if self.dead || !self.close_pending || caller != self.target
            || args != [self.target as u64, self.task as u64, DECLINE_UNSAVED] { return false; }
        self.close_pending = false; self.close_attempt = None; self.close_declined = true; true
    }
    pub fn hint(&mut self, caller: u32, target: u64, task: u64, reserved: u64) -> bool {
        if caller != 0 || target != self.target as u64 || task != self.task as u64 || reserved != 0 { return false; }
        if !self.acked { self.pending = true; }
        true
    }
    pub fn attest(&mut self, target: u32, task: u32, required: u64) -> bool {
        if !self.pending || target != self.target || task != self.task || required & 1 == 0 { return false; }
        self.dead = true;
        self.close_pending = false;
        self.registration_reply = false;
        true
    }
    /// A Domain endpoint grant does not confer every shell sub-operation.
    pub fn allows_operation(&self, opcode: u64, args: [u64;3]) -> bool {
        if self.dead || self.close_pending { return false; }
        match opcode {
            0xFA => args[0] == self.surface && args[1] != 0,
            // Document controls and read-only invalidation are exact, no-payload operations on
            // the existing BOOT orchestration edge, not ambient shell access.
            0x275 => crate::app_catalog::by_surface(self.target,self.surface).is_some_and(|i|i.app!=crate::app_catalog::App::Spindle) && args == [self.target as u64,self.task as u64,0],
            0x274 | 0x279 => crate::app_catalog::by_surface(self.target,self.surface).is_some_and(|i|i.app!=crate::app_catalog::App::Spindle) && (!crate::app_catalog::is_browser(self.target)||self.task==self.target) && args == [self.target as u64,self.task as u64,0],
            // Existing Spindle launcher route selects a fixed shell action.
            0x15 => crate::app_catalog::by_surface(self.target,self.surface).is_some_and(|i|i.app==crate::app_catalog::App::Spindle) && args[0] <= 7 && args[1] == 0 && args[2] == 0,
            0 => crate::app_catalog::by_surface(self.target,self.surface).is_some_and(|i|i.app==crate::app_catalog::App::Spindle) && args == [0;3], // no-effect endpoint probe
            _ => false,
        }
    }
    pub fn allows(&self, caller: u32, surface: u64, app_id: u16) -> bool {
        !self.dead && !self.close_pending && caller == self.target && surface == self.surface && app_id as u32 == self.target
    }
    pub fn finish_ack(&mut self) -> bool {
        if !self.dead || !self.retired { return false; }
        self.acked = true; self.pending = false; true
    }
}

/// Existing BOOT_DAG drive owns the paired frame handed to shell.
pub fn buffer_sender(caller: u32) -> bool { caller == 2 }
