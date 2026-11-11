// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Recording protocol data, never executable identity or authority. BOOT pins the
//! reserved image; servers separately validate held lifecycles and peer snapshots.
pub const ENABLED:bool=match option_env!("SEXOS_RECORDER"){Some(value)=>{let bytes=value.as_bytes();bytes.len()==1&&bytes[0]==b'1'},None=>false};
pub const TARGET: u32 = 42;
pub const SPINDLE: u32 = 12;
pub const SHELL: u32 = 3;
pub const FILES: u32 = 11;
pub const DISPLAY: u32 = 1;
pub const ROOT: &[u8] = b"/recordings";
pub const HEAP_LIMIT: usize = 64 * 1024 * 1024;
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 360;
pub const FPS: u32 = 5;
pub const MAX_DURATION_MS: u64 = 60_000;
pub const NAME_BYTES: usize = 200;
pub const OP_FILES_BIND: u64 = 0x2e1;
pub const OP_SPINDLE: u64 = 0x2e2;
pub const OP_RECORDER: u64 = 0x2e3;
pub const OP_DISPLAY: u64 = 0x2e4;
pub const OP_FRAME_ACK: u64 = 0x2e5;
pub const OP_FILES_IMAGE: u64 = 0x2e6;
pub const MAX_IMAGE_BYTES: u32 = 32 * 1024 * 1024;

/// Epoch prefixes prevent a fresh same-task Shell from reusing an old ACK nonce.
/// Epoch/counter overflow refuses the operation; it must never be truncated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleSequence(u32);
impl RoleSequence {
    pub fn new(shell_epoch: u64, counter: u32) -> Option<Self> {
        if !(1..16).contains(&shell_epoch) || !(1..1 << 20).contains(&counter) {
            return None;
        }
        Some(Self((shell_epoch as u32) << 20 | counter))
    }
    pub fn decode(raw: u64) -> Option<Self> {
        if raw > 0xff_ffff {
            return None;
        }
        let value = raw as u32;
        Self::new((value >> 20) as u64, value & 0xfffff)
    }
    pub const fn word(self) -> u32 {
        self.0
    }
    pub fn matches(self, shell_epoch: u64) -> bool {
        shell_epoch != 0 && shell_epoch < 16 && (self.0 >> 20) as u64 == shell_epoch
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSequence(u32);
impl SessionSequence {
    pub fn new(shell_epoch: u64, counter: u32) -> Option<Self> {
        if !(1..256).contains(&shell_epoch) || !(1..1 << 24).contains(&counter) {
            return None;
        }
        Some(Self((shell_epoch as u32) << 24 | counter))
    }
    pub fn decode(raw: u64) -> Option<Self> {
        let value = u32::try_from(raw).ok()?;
        Self::new((value >> 24) as u64, value & 0xff_ffff)
    }
    pub const fn word(self) -> u32 {
        self.0
    }
    pub fn matches(self, shell_epoch: u64) -> bool {
        shell_epoch != 0 && shell_epoch < 256 && (self.0 >> 24) as u64 == shell_epoch
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindRequest {
    pub sequence: RoleSequence,
    pub task: u32,
    pub shell_epoch: u64,
    pub files_epoch: u32,
}
impl BindRequest {
    pub fn new(
        sequence: RoleSequence,
        task: u32,
        shell_epoch: u64,
        files_epoch: u32,
    ) -> Option<Self> {
        (sequence.matches(shell_epoch) && task != 0 && files_epoch != 0).then_some(Self {
            sequence,
            task,
            shell_epoch,
            files_epoch,
        })
    }
    pub const fn args(self) -> [u64; 3] {
        [
            self.sequence.word() as u64,
            self.task as u64 | ((self.files_epoch as u64) << 32),
            self.shell_epoch,
        ]
    }
    pub fn decode(caller: u32, args: [u64; 3]) -> Option<Self> {
        if caller != SHELL {
            return None;
        }
        Self::new(
            RoleSequence::decode(args[0])?,
            args[1] as u32,
            args[2],
            (args[1] >> 32) as u32,
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BindStatus {
    Ok = 0,
    Denied = 1,
    Unavailable = 2,
    Io = 3,
    Busy = 4,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindReply {
    pub sequence: RoleSequence,
    pub task: u32,
    pub status: BindStatus,
}
impl BindReply {
    pub fn new(request: BindRequest, status: BindStatus) -> Self {
        Self {
            sequence: request.sequence,
            task: request.task,
            status,
        }
    }
    pub const fn word(self) -> u64 {
        ((0xb0 | self.status as u8) as u64) << 56
            | (self.task as u64) << 24
            | self.sequence.word() as u64
    }
    pub fn decode(caller: u32, word: u64) -> Option<Self> {
        if caller != FILES || word >> 60 != 0xb {
            return None;
        }
        let status = match (word >> 56) & 15 {
            0 => BindStatus::Ok,
            1 => BindStatus::Denied,
            2 => BindStatus::Unavailable,
            3 => BindStatus::Io,
            4 => BindStatus::Busy,
            _ => return None,
        };
        let task = ((word >> 24) & 0xffff_ffff) as u32;
        if task == 0 {
            return None;
        }
        Some(Self {
            sequence: RoleSequence::decode(word & 0xff_ffff)?,
            task,
            status,
        })
    }
}

/// One immutable basename assembled through ordered bounded PDX chunks. The
/// receiver supplies the actual stamped owner task and expected transaction.
pub struct Name {
    bytes: [u8; NAME_BYTES],
    length: usize,
    received: usize,
    owner: u32,
    sequence: u32,
}
impl Name {
    pub const fn new() -> Self {
        Self {
            bytes: [0; NAME_BYTES],
            length: 0,
            received: 0,
            owner: 0,
            sequence: 0,
        }
    }
    pub fn clear(&mut self) {
        self.bytes.fill(0);
        self.length = 0;
        self.received = 0;
        self.owner = 0;
        self.sequence = 0;
    }
    pub fn begin(&mut self, owner: u32, sequence: u32, length: usize) -> bool {
        if owner == 0 || sequence == 0 || length == 0 || length > NAME_BYTES {
            return false;
        }
        self.clear();
        self.owner = owner;
        self.sequence = sequence;
        self.length = length;
        true
    }
    pub fn chunk(&mut self, owner: u32, sequence: u32, offset: usize, words: [u64; 2]) -> bool {
        if owner != self.owner
            || sequence != self.sequence
            || offset != self.received
            || offset >= self.length
        {
            return false;
        }
        let n = (self.length - offset).min(16);
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&words[0].to_le_bytes());
        bytes[8..].copy_from_slice(&words[1].to_le_bytes());
        if bytes[n..].iter().any(|b| *b != 0) {
            return false;
        }
        self.bytes[offset..offset + n].copy_from_slice(&bytes[..n]);
        self.received += n;
        true
    }
    pub fn complete(&self, owner: u32, sequence: u32) -> Option<&[u8]> {
        if owner != self.owner
            || sequence != self.sequence
            || self.length == 0
            || self.received != self.length
        {
            return None;
        }
        let name = &self.bytes[..self.length];
        if name.first() == Some(&b'@')
            || !name.ends_with(b".mkv")
            || name.iter().any(|b| *b == b'/' || b.is_ascii_control())
        {
            return None;
        }
        Some(name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ImageKind {
    Read = 1,
    Release = 2,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageRequest {
    pub kind: ImageKind,
    pub sequence: RoleSequence,
    pub shell_epoch: u64,
    pub files_epoch: u32,
}
impl ImageRequest {
    pub fn new(
        kind: ImageKind,
        sequence: RoleSequence,
        shell_epoch: u64,
        files_epoch: u32,
    ) -> Option<Self> {
        (sequence.matches(shell_epoch) && files_epoch != 0).then_some(Self {
            kind,
            sequence,
            shell_epoch,
            files_epoch,
        })
    }
    pub const fn args(self) -> [u64; 3] {
        [
            ((self.kind as u64) << 56) | self.sequence.word() as u64,
            self.files_epoch as u64,
            self.shell_epoch,
        ]
    }
    pub fn decode(caller: u32, args: [u64; 3]) -> Option<Self> {
        if caller != SHELL || args[0] & 0x00ff_ffff_ff00_0000 != 0 {
            return None;
        }
        let kind = match args[0] >> 56 {
            1 => ImageKind::Read,
            2 => ImageKind::Release,
            _ => return None,
        };
        Self::new(
            kind,
            RoleSequence::decode(args[0] & 0xff_ffff)?,
            args[2],
            u32::try_from(args[1]).ok()?,
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageReply {
    pub kind: ImageKind,
    pub sequence: RoleSequence,
    pub status: BindStatus,
    pub bytes: u32,
}
impl ImageReply {
    pub fn new(request: ImageRequest, status: BindStatus, bytes: u32) -> Option<Self> {
        (request.sequence.matches(request.shell_epoch)
            && request.files_epoch != 0
            && Self::valid(request.kind, status, bytes))
        .then_some(Self {
            kind: request.kind,
            sequence: request.sequence,
            status,
            bytes,
        })
    }
    pub const fn word(self) -> u64 {
        let tag = match self.kind {
            ImageKind::Read => 0xa0u64,
            ImageKind::Release => 0x90,
        };
        ((tag | self.status as u64) << 56)
            | ((self.bytes as u64) << 24)
            | self.sequence.word() as u64
    }
    pub fn decode(caller: u32, word: u64) -> Option<Self> {
        if caller != FILES {
            return None;
        }
        let kind = match word >> 60 {
            0xa => ImageKind::Read,
            9 => ImageKind::Release,
            _ => return None,
        };
        let status = match (word >> 56) & 15 {
            0 => BindStatus::Ok,
            1 => BindStatus::Denied,
            2 => BindStatus::Unavailable,
            3 => BindStatus::Io,
            4 => BindStatus::Busy,
            _ => return None,
        };
        let sequence = RoleSequence::decode(word & 0xff_ffff)?;
        let bytes = ((word >> 24) & 0xffff_ffff) as u32;
        Self::valid(kind, status, bytes).then_some(Self {
            kind,
            sequence,
            status,
            bytes,
        })
    }
    fn valid(kind: ImageKind, status: BindStatus, bytes: u32) -> bool {
        match (kind, status) {
            (ImageKind::Read, BindStatus::Ok) => bytes > 0 && bytes <= MAX_IMAGE_BYTES,
            _ => bytes == 0,
        }
    }
}

/// Person intent transport. Metadata alone is not authentication: Shell also
/// checks the kernel-stamped current Spindle task against its held lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Begin { sequence: u32, length: u16, shell_epoch:u64 },
    Chunk { sequence: u32, offset: u8, words: [u64; 2] },
    Commit { sequence: u32 },
    Stop { sequence: u32 },
}
impl Intent {
    pub fn sequence(self) -> u32 {
        match self { Self::Begin{sequence,..}|Self::Chunk{sequence,..}|Self::Commit{sequence}|Self::Stop{sequence}=>sequence }
    }
    pub fn args(self) -> [u64;3] {
        let sequence=self.sequence() as u64;
        match self {
            Self::Begin{length,shell_epoch,..}=>[1<<56|sequence,length as u64,shell_epoch],
            Self::Chunk{offset,words,..}=>[2<<56|(offset as u64)<<32|sequence,words[0],words[1]],
            Self::Commit{..}=>[3<<56|sequence,0,0],
            Self::Stop{..}=>[4<<56|sequence,0,0],
        }
    }
    pub fn decode(caller:u32,args:[u64;3])->Option<Self>{
        if caller!=SPINDLE || args[0]&0x00ff_ff00_ff00_0000!=0{return None}
        let sequence=(args[0]&0xff_ffff) as u32;
        if sequence==0{return None}
        let offset=((args[0]>>32)&255)as u8;
        match args[0]>>56 {
            1 if offset==0 && args[2]!=0 && (1..=NAME_BYTES as u64).contains(&args[1])=>Some(Self::Begin{sequence,length:args[1] as u16,shell_epoch:args[2]}),
            2 if (offset as usize)<NAME_BYTES && offset%16==0=>Some(Self::Chunk{sequence,offset,words:[args[1],args[2]]}),
            3 if offset==0 && args[1]==0 && args[2]==0=>Some(Self::Commit{sequence}),
            4 if offset==0 && args[1]==0 && args[2]==0=>Some(Self::Stop{sequence}),
            _=>None,
        }
    }
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
#[repr(u8)]
pub enum IntentStatus {Denied=1,Unavailable=2,Busy=3,Starting=4,Stopped=5,Saved=6,Failed=7,Active=8}
pub fn intent_reply(sequence:u32,status:IntentStatus)->Option<u64>{
    (sequence!=0 && sequence<=0xff_ffff).then_some(0x88u64<<56|(status as u64)<<48|sequence as u64)
}
pub fn decode_intent_reply(caller:u32,word:u64)->Option<(u32,IntentStatus)>{
    if caller!=SHELL || word>>56!=0x88 || word&0x0000_ffff_ff00_0000!=0{return None}
    let sequence=(word&0xff_ffff)as u32;if sequence==0{return None}
    let status=match (word>>48)&255 {1=>IntentStatus::Denied,2=>IntentStatus::Unavailable,3=>IntentStatus::Busy,4=>IntentStatus::Starting,5=>IntentStatus::Stopped,6=>IntentStatus::Saved,7=>IntentStatus::Failed,8=>IntentStatus::Active,_=>return None};
    Some((sequence,status))
}

/// Setup and progress over the reserved application's existing held Shell route.
/// Each receiver must additionally validate the kernel-stamped current main task,
/// both held service epochs, and its retained session before changing state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCommand {
    Prepare {
        session: SessionSequence,
        length: u16,
        shell_epoch: u32,
        task: u32,
        files_epoch: u32,
        spindle_task: u32,
    },
    Name {
        session: SessionSequence,
        offset: u8,
        words: [u64; 2],
    },
    Active {
        session: SessionSequence,
        shell_epoch: u64,
        display_epoch: u32,
        task: u32,
    },
    Stop {
        session: SessionSequence,
        shell_epoch: u64,
        display_epoch: u32,
        task: u32,
    },
}
impl AppCommand {
    pub fn args(self) -> [u64; 3] {
        match self {
            Self::Prepare {
                session,
                length,
                shell_epoch,
                task,
                files_epoch,
                spindle_task,
            } => [
                1 << 56 | (length as u64) << 32 | session.word() as u64,
                (shell_epoch as u64) << 32 | task as u64,
                (files_epoch as u64) << 32 | spindle_task as u64,
            ],
            Self::Name {
                session,
                offset,
                words,
            } => [
                2 << 56 | (offset as u64) << 32 | session.word() as u64,
                words[0],
                words[1],
            ],
            Self::Active {
                session,
                shell_epoch,
                display_epoch,
                task,
            } => [
                3 << 56 | session.word() as u64,
                shell_epoch,
                (display_epoch as u64) << 32 | task as u64,
            ],
            Self::Stop {
                session,
                shell_epoch,
                display_epoch,
                task,
            } => [
                8 << 56 | session.word() as u64,
                shell_epoch,
                (display_epoch as u64) << 32 | task as u64,
            ],
        }
    }
    pub fn decode(caller: u32, args: [u64; 3]) -> Option<Self> {
        if caller != SHELL {
            return None;
        }
        let session = SessionSequence::decode(args[0] & 0xffff_ffff)?;
        match args[0] >> 56 {
            1 if args[0] & 0x00ff_0000_0000_0000 == 0 => {
                let length = ((args[0] >> 32) & 0xffff) as u16;
                let files_epoch = (args[2] >> 32) as u32;
                let spindle_task = args[2] as u32;
                let shell_epoch = (args[1] >> 32) as u32;
                let task = args[1] as u32;
                (session.matches(shell_epoch as u64)
                    && length != 0
                    && (length as usize) <= NAME_BYTES
                    && task != 0
                    && files_epoch != 0
                    && spindle_task != 0)
                    .then_some(Self::Prepare {
                        session,
                        length,
                        shell_epoch,
                        task,
                        files_epoch,
                        spindle_task,
                    })
            }
            2 if args[0] & 0x00ff_ff00_0000_0000 == 0 => {
                let offset = ((args[0] >> 32) & 255) as u8;
                (offset % 16 == 0 && (offset as usize) < NAME_BYTES).then_some(Self::Name {
                    session,
                    offset,
                    words: [args[1], args[2]],
                })
            }
            kind @ (3 | 8) if args[0] & 0x00ff_ffff_0000_0000 == 0 => {
                let display_epoch = (args[2] >> 32) as u32;
                let task = args[2] as u32;
                if !session.matches(args[1]) || display_epoch == 0 || task == 0 {
                    return None;
                }
                Some(if kind == 3 {
                    Self::Active {
                        session,
                        shell_epoch: args[1],
                        display_epoch,
                        task,
                    }
                } else {
                    Self::Stop {
                        session,
                        shell_epoch: args[1],
                        display_epoch,
                        task,
                    }
                })
            }
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AppStatus {
    Prepared = 4,
    Saved = 5,
    Failed = 6,
    Stopped = 7,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppReport {
    pub session: SessionSequence,
    pub status: AppStatus,
    pub task: u32,
    pub files_epoch: u32,
    pub display_epoch: u32,
    pub spindle_task: u32,
}
impl AppReport {
    pub const fn args(self) -> [u64; 3] {
        [
            (self.status as u64) << 56 | self.session.word() as u64,
            (self.files_epoch as u64) << 32 | self.task as u64,
            (self.display_epoch as u64) << 32 | self.spindle_task as u64,
        ]
    }
    pub fn decode(caller: u32, args: [u64; 3]) -> Option<Self> {
        if caller != TARGET || args[0] & 0x00ff_ffff_0000_0000 != 0 {
            return None;
        }
        let session = SessionSequence::decode(args[0] & 0xffff_ffff)?;
        let status = match args[0] >> 56 {
            4 => AppStatus::Prepared,
            5 => AppStatus::Saved,
            6 => AppStatus::Failed,
            7 => AppStatus::Stopped,
            _ => return None,
        };
        let (task, files_epoch, display_epoch, spindle_task) = (
            args[1] as u32,
            (args[1] >> 32) as u32,
            (args[2] >> 32) as u32,
            args[2] as u32,
        );
        (task != 0 && files_epoch != 0 && display_epoch != 0 && spindle_task != 0).then_some(Self {
            session,
            status,
            task,
            files_epoch,
            display_epoch,
            spindle_task,
        })
    }
}
