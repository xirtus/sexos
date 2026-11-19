// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Recording transport data. These values never establish sender/lifecycle authority.
use crate::desktop_recording::{RoleSequence, SessionSequence, DISPLAY, SHELL, TARGET};
#[path = "recording_frame_bank.rs"]
mod bank;
pub use bank::{Bank, Header, Snapshot};

pub const SLOT: u64 = 95;
pub const REGION_ID: u32 = 0x6101;
pub const BYTES: usize = 1024 * 1024;
pub const HEADER_BYTES: usize = 4096;
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 360;
pub const PIXELS: usize = WIDTH * HEIGHT;
pub const CHUNK_PIXELS: usize = 4096;
pub const INTERVAL_MS: u64 = 200;
pub const DURATION_MS: u64 = 60_000;
pub const OP_CONTROL: u64 = 0x2e4;
pub const OP_FRAME: u64 = 0x2e5;
pub const OP_EVENT: u64 = 0x2e7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub session: SessionSequence,
    pub shell_epoch: u64,
    pub display_epoch: u64,
    pub record_task: u32,
    pub spindle_task: u32,
}
impl Scope {
    pub fn valid(self) -> bool {
        self.session.matches(self.shell_epoch) && self.shell_epoch <= u32::MAX as u64
            && self.display_epoch != 0 && self.display_epoch <= u32::MAX as u64
            && self.record_task != 0 && self.spindle_task != 0
    }
    fn args(self, kind: u8, sequence: RoleSequence) -> Option<[u64; 3]> {
        if !self.valid() || !sequence.matches(self.shell_epoch) { return None; }
        Some([
            ((kind as u64) << 56) | ((sequence.word() as u64) << 32) | self.session.word() as u64,
            (u32::try_from(self.display_epoch).ok()? as u64) << 32 | self.record_task as u64,
            (u32::try_from(self.shell_epoch).ok()? as u64) << 32 | self.spindle_task as u64,
        ])
    }
    fn decode(args: [u64; 3]) -> Option<(Self, RoleSequence)> {
        let scope = Self { session: SessionSequence::decode(args[0] & 0xffff_ffff)?,
            shell_epoch: args[2] >> 32, display_epoch: args[1] >> 32,
            record_task: args[1] as u32, spindle_task: args[2] as u32 };
        let sequence = RoleSequence::decode((args[0] >> 32) & 0xff_ffff)?;
        (scope.valid() && sequence.matches(scope.shell_epoch)).then_some((scope, sequence))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ControlKind { Arm = 1, Stop = 2, Acquire = 4 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlRequest { pub kind: ControlKind, pub sequence: RoleSequence, pub scope: Scope }
impl ControlRequest {
    pub fn args(self) -> Option<[u64; 3]> { self.scope.args(self.kind as u8, self.sequence) }
    pub fn decode(caller: u32, args: [u64; 3]) -> Option<Self> {
        let kind = match args[0] >> 56 {
            1 if caller == SHELL => ControlKind::Arm,
            2 if caller == SHELL => ControlKind::Stop,
            4 if caller == TARGET => ControlKind::Acquire,
            _ => return None,
        };
        let (scope, sequence) = Scope::decode(args)?;
        Some(Self { kind, sequence, scope })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AckKind { Frame = 1, Release = 2 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameAck {
    pub kind: AckKind, pub sequence: RoleSequence, pub session: SessionSequence,
    pub generation: u64, pub display_epoch: u64,
}
impl FrameAck {
    fn valid(self) -> bool {
        self.display_epoch != 0 && self.display_epoch <= u32::MAX as u64
            && self.sequence.matches((self.session.word() >> 24) as u64)
            && match self.kind { AckKind::Frame => self.generation != 0, AckKind::Release => self.generation == 0 }
    }
    pub fn args(self) -> Option<[u64; 3]> {
        self.valid().then_some([
            ((self.kind as u64) << 56) | ((self.sequence.word() as u64) << 32) | self.session.word() as u64,
            self.generation, self.display_epoch,
        ])
    }
    pub fn decode(caller: u32, args: [u64; 3]) -> Option<Self> {
        if caller != TARGET { return None; }
        let value = Self {
            kind: match args[0] >> 56 { 1 => AckKind::Frame, 2 => AckKind::Release, _ => return None },
            sequence: RoleSequence::decode((args[0] >> 32) & 0xff_ffff)?,
            session: SessionSequence::decode(args[0] & 0xffff_ffff)?,
            generation: args[1], display_epoch: args[2],
        };
        value.valid().then_some(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CloseReason { Released = 1, Expired = 2, Cancelled = 3 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClosedEvent { pub reason: CloseReason, pub sequence: RoleSequence, pub scope: Scope }
impl ClosedEvent {
    pub fn args(self) -> Option<[u64; 3]> { self.scope.args(self.reason as u8, self.sequence) }
    pub fn decode(caller: u32, args: [u64; 3]) -> Option<Self> {
        if caller != DISPLAY { return None; }
        let reason = match args[0] >> 56 { 1 => CloseReason::Released, 2 => CloseReason::Expired,
            3 => CloseReason::Cancelled, _ => return None };
        let (scope, sequence) = Scope::decode(args)?;
        Some(Self { reason, sequence, scope })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ControlStatus { Armed = 0, Stopped = 1, Denied = 2, Busy = 3, Unavailable = 4, Expired = 5, Cancelled = 6 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameStatus { Ready = 0, Acked = 1, Released = 2, Denied = 3, Busy = 4, Cancelled = 5, Expired = 6 }

/// Denied may echo a malformed 24-bit nonce; it cannot match a valid positive.
/// Every other status requires a canonical epoch-prefixed nonce and nonzero task.
macro_rules! receipt {
    ($name:ident,$status:ident,$tag:expr,$denied:ident,{$($n:literal=>$variant:ident),+}) => {
        #[derive(Clone,Copy,Debug,PartialEq,Eq)]
        pub struct $name { pub sequence: u32, pub task: u32, pub status: $status }
        impl $name {
            pub fn new(sequence: RoleSequence, task: u32, status: $status) -> Option<Self> {
                (task != 0).then_some(Self { sequence: sequence.word(), task, status })
            }
            pub fn denied(raw_sequence: u64, task: u32) -> Option<Self> {
                (raw_sequence <= 0xff_ffff).then_some(Self { sequence: raw_sequence as u32, task, status: $status::$denied })
            }
            pub fn word(self) -> Option<u64> {
                if self.sequence > 0xff_ffff || (self.status != $status::$denied
                    && (self.task == 0 || RoleSequence::decode(self.sequence as u64).is_none())) { return None; }
                Some((($tag | self.status as u64) << 56) | ((self.task as u64) << 24) | self.sequence as u64)
            }
            pub fn decode(caller: u32, word: u64) -> Option<Self> {
                if caller != DISPLAY || word >> 60 != $tag >> 4 { return None; }
                let status = match (word >> 56) & 15 { $($n => $status::$variant,)+ _ => return None };
                let value = Self { sequence: (word & 0xff_ffff) as u32, task: ((word >> 24) & 0xffff_ffff) as u32, status };
                value.word().filter(|encoded| *encoded == word).map(|_| value)
            }
            pub fn matches(self, sequence: RoleSequence, task: u32) -> bool {
                task != 0 && self.task == task && self.sequence == sequence.word()
            }
        }
    }
}
receipt!(ControlReply,ControlStatus,0xc0,Denied,{0=>Armed,1=>Stopped,2=>Denied,3=>Busy,4=>Unavailable,5=>Expired,6=>Cancelled});
receipt!(FrameReply,FrameStatus,0xd0,Denied,{0=>Ready,1=>Acked,2=>Released,3=>Denied,4=>Busy,5=>Cancelled,6=>Expired});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentRect { pub x: usize, pub y: usize, pub width: usize, pub height: usize }
impl ContentRect {
    pub fn fit(native_width: usize, native_height: usize) -> Option<Self> {
        if native_width == 0 || native_height == 0 || native_width > 8192 || native_height > 4320 { return None; }
        let (width,height) = if native_width as u64 * HEIGHT as u64 >= native_height as u64 * WIDTH as u64 {
            (WIDTH, (native_height as u64 * WIDTH as u64 / native_width as u64).max(1) as usize)
        } else {
            ((native_width as u64 * HEIGHT as u64 / native_height as u64).max(1) as usize, HEIGHT)
        };
        Some(Self { x:(WIDTH-width)/2, y:(HEIGHT-height)/2, width, height })
    }
    pub fn source(self, x: usize, y: usize, nw: usize, nh: usize) -> Option<usize> {
        if Self::fit(nw,nh) != Some(self) || x < self.x || y < self.y
            || x-self.x >= self.width || y-self.y >= self.height { return None; }
        let sx = (((2*(x-self.x)+1) as u64 * nw as u64)/(2*self.width) as u64).min(nw as u64-1) as usize;
        let sy = (((2*(y-self.y)+1) as u64 * nh as u64)/(2*self.height) as u64).min(nh as u64-1) as usize;
        sy.checked_mul(nw)?.checked_add(sx)
    }
}
