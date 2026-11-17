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

// ── Broadcast Studio Capture v2 (Cabaret / SilkBar / SexDisplay) ──────────────

pub const OP_STUDIO_CAPTURE: u64 = 0x2e7;
pub const OP_STUDIO_WINDOW_FLAGS: u64 = 0x2e8;
pub const MAX_STUDIO_WIDTH: u32 = 3840;
pub const MAX_STUDIO_HEIGHT: u32 = 2160;
pub const MAX_STUDIO_FPS: u32 = 120;

/// What Display composites into a Broadcast Studio Capture v2 frame ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSourceTarget {
    FullDisplay(u8),
    SilkWindow { frame_id: u32 },
    Region { x: u16, y: u16, w: u16, h: u16 },
    FollowFocusedWindow,
}

impl CaptureSourceTarget {
    const KIND_FULL_DISPLAY: u8 = 1;
    const KIND_SILK_WINDOW: u8 = 2;
    const KIND_REGION: u8 = 3;
    const KIND_FOLLOW_FOCUSED: u8 = 4;

    pub const fn is_valid(self) -> bool {
        match self {
            Self::FullDisplay(_) => true,
            Self::SilkWindow { frame_id } => frame_id != 0,
            Self::Region { w, h, .. } => {
                w > 0
                    && h > 0
                    && w as u32 <= MAX_STUDIO_WIDTH
                    && h as u32 <= MAX_STUDIO_HEIGHT
            }
            Self::FollowFocusedWindow => true,
        }
    }

    pub const fn encode_pair(self) -> Option<(u8, u64)> {
        if !self.is_valid() {
            return None;
        }
        match self {
            Self::FullDisplay(idx) => Some((Self::KIND_FULL_DISPLAY, idx as u64)),
            Self::SilkWindow { frame_id } => Some((Self::KIND_SILK_WINDOW, frame_id as u64)),
            Self::Region { x, y, w, h } => Some((
                Self::KIND_REGION,
                (x as u64)
                    | ((y as u64) << 16)
                    | ((w as u64) << 32)
                    | ((h as u64) << 48),
            )),
            Self::FollowFocusedWindow => Some((Self::KIND_FOLLOW_FOCUSED, 0)),
        }
    }

    pub fn decode_pair(kind: u8, payload: u64) -> Option<Self> {
        let target = match kind {
            Self::KIND_FULL_DISPLAY => {
                if payload > u8::MAX as u64 {
                    return None;
                }
                Self::FullDisplay(payload as u8)
            }
            Self::KIND_SILK_WINDOW => {
                let frame_id = u32::try_from(payload).ok()?;
                Self::SilkWindow { frame_id }
            }
            Self::KIND_REGION => Self::Region {
                x: payload as u16,
                y: (payload >> 16) as u16,
                w: (payload >> 32) as u16,
                h: (payload >> 48) as u16,
            },
            Self::KIND_FOLLOW_FOCUSED => {
                if payload != 0 {
                    return None;
                }
                Self::FollowFocusedWindow
            }
            _ => return None,
        };
        target.is_valid().then_some(target)
    }
}

/// Frame ring pixel layout for Broadcast Studio Capture v2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CapturePixelFmt {
    Xrgb8888 = 1,
    Nv12 = 2,
    P010 = 3,
}

impl CapturePixelFmt {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Xrgb8888),
            2 => Some(Self::Nv12),
            3 => Some(Self::P010),
            _ => None,
        }
    }

    pub const fn is_subsampled_420(self) -> bool {
        matches!(self, Self::Nv12 | Self::P010)
    }

    /// Bytes required for a single frame of `(width, height)`, or `None` if invalid.
    pub const fn frame_bytes(self, width: u32, height: u32) -> Option<usize> {
        if width == 0
            || height == 0
            || width > MAX_STUDIO_WIDTH
            || height > MAX_STUDIO_HEIGHT
        {
            return None;
        }
        if self.is_subsampled_420() && (width % 2 != 0 || height % 2 != 0) {
            return None;
        }
        let pixels = (width as usize) * (height as usize);
        match self {
            Self::Xrgb8888 => pixels.checked_mul(4),
            Self::Nv12 => match pixels.checked_mul(3) {
                Some(n) => Some(n / 2),
                None => None,
            },
            Self::P010 => pixels.checked_mul(3),
        }
    }
}

/// Bitflags controlling how Display includes or omits a window during capture.
///
/// `HIDE_FROM_CAPTURE` ensures Cabaret UI, Cabaret Teleprompter, and Chastity password
/// windows are automatically omitted from composited screen capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WindowCapturePolicy(u32);

impl WindowCapturePolicy {
    pub const NONE: Self = Self(0);
    pub const HIDE_FROM_CAPTURE: Self = Self(1 << 0);
    pub const OMIT_DECORATIONS: Self = Self(1 << 1);
    pub const REDACT_WHEN_UNFOCUSED: Self = Self(1 << 2);
    const ALL_BITS: u32 = Self::HIDE_FROM_CAPTURE.0 | Self::OMIT_DECORATIONS.0 | Self::REDACT_WHEN_UNFOCUSED.0;

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::ALL_BITS != 0 {
            None
        } else {
            Some(Self(bits))
        }
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn is_hidden_from_capture(self) -> bool {
        self.contains(Self::HIDE_FROM_CAPTURE)
    }

    /// Default policy for sensitive or studio-overlay windows (Cabaret UI,
    /// Cabaret Teleprompter, and Chastity secret/password dialogs).
    pub const fn sensitive_overlay() -> Self {
        Self::HIDE_FROM_CAPTURE
    }
}

impl core::ops::BitOr for WindowCapturePolicy {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitAnd for WindowCapturePolicy {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// Wire request for `OP_STUDIO_WINDOW_FLAGS` (`arg0 = frame_id`, `arg1 = policy bits`, `arg2 = 0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowCaptureFlagsReq {
    pub frame_id: u32,
    pub policy: WindowCapturePolicy,
}

impl WindowCaptureFlagsReq {
    pub const fn new(frame_id: u32, policy: WindowCapturePolicy) -> Option<Self> {
        if frame_id == 0 {
            None
        } else {
            Some(Self { frame_id, policy })
        }
    }

    pub const fn args(self) -> [u64; 3] {
        [self.frame_id as u64, self.policy.bits() as u64, 0]
    }

    pub fn decode(args: [u64; 3]) -> Option<Self> {
        if args[2] != 0 {
            return None;
        }
        let frame_id = u32::try_from(args[0]).ok()?;
        let bits = u32::try_from(args[1]).ok()?;
        let policy = WindowCapturePolicy::from_bits(bits)?;
        Self::new(frame_id, policy)
    }
}

/// Errors returned when validating or arming a [`StudioSessionSpec`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioSpecError {
    InvalidDimensions,
    InvalidFps,
    InvalidTarget,
    UnalignedSubsampledDimensions,
    TallyInactive,
}

/// Validated Broadcast Studio Capture v2 session specification ready to arm frame rings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmedStudioSession {
    spec: StudioSessionSpec,
}

impl ArmedStudioSession {
    pub const fn spec(&self) -> StudioSessionSpec {
        self.spec
    }

    pub fn args(self) -> [u64; 3] {
        self.spec.raw_args()
    }
}

/// Broadcast Studio Capture v2 session specification.
///
/// Enforces that the `SilkBar` red tally indicator (`tally_active`) is lit before
/// Display arms the frame capture rings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StudioSessionSpec {
    pub sequence: SessionSequence,
    pub target: CaptureSourceTarget,
    pub width: u16,
    pub height: u16,
    pub fps: u8,
    pub pixel_fmt: CapturePixelFmt,
    pub tally_active: bool,
    pub hide_cursor: bool,
}

impl StudioSessionSpec {
    pub const fn new(
        sequence: SessionSequence,
        target: CaptureSourceTarget,
        width: u16,
        height: u16,
        fps: u8,
        pixel_fmt: CapturePixelFmt,
        tally_active: bool,
        hide_cursor: bool,
    ) -> Self {
        Self {
            sequence,
            target,
            width,
            height,
            fps,
            pixel_fmt,
            tally_active,
            hide_cursor,
        }
    }

    pub const fn validate(&self) -> Result<(), StudioSpecError> {
        let w = self.width as u32;
        let h = self.height as u32;
        if w == 0 || h == 0 || w > MAX_STUDIO_WIDTH || h > MAX_STUDIO_HEIGHT {
            return Err(StudioSpecError::InvalidDimensions);
        }
        if self.pixel_fmt.is_subsampled_420() && (w % 2 != 0 || h % 2 != 0) {
            return Err(StudioSpecError::UnalignedSubsampledDimensions);
        }
        if self.fps == 0 || (self.fps as u32) > MAX_STUDIO_FPS {
            return Err(StudioSpecError::InvalidFps);
        }
        if !self.target.is_valid() {
            return Err(StudioSpecError::InvalidTarget);
        }
        Ok(())
    }

    /// True only when geometry/FPS/target are valid AND SilkBar's red tally indicator is active.
    pub const fn can_arm_rings(&self) -> bool {
        self.validate().is_ok() && self.tally_active
    }

    /// Enforces that `SilkBar` red tally is active before frame rings can arm.
    pub const fn arm(self) -> Result<ArmedStudioSession, StudioSpecError> {
        if let Err(e) = self.validate() {
            return Err(e);
        }
        if !self.tally_active {
            return Err(StudioSpecError::TallyInactive);
        }
        Ok(ArmedStudioSession { spec: self })
    }

    const fn raw_args(self) -> [u64; 3] {
        let (target_kind, target_payload) = match self.target.encode_pair() {
            Some(pair) => pair,
            None => (0, 0),
        };
        let arg0 = (self.sequence.word() as u64)
            | ((self.width as u64) << 32)
            | ((self.height as u64) << 44)
            | ((self.pixel_fmt as u64) << 56)
            | ((target_kind as u64) << 60);
        let arg2 = (self.fps as u64)
            | ((self.tally_active as u64) << 8)
            | ((self.hide_cursor as u64) << 9);
        [arg0, target_payload, arg2]
    }

    /// Decodes an `OP_STUDIO_CAPTURE` invocation, refusing any request without an active tally.
    pub fn decode_armed(args: [u64; 3]) -> Option<ArmedStudioSession> {
        if (args[0] >> 63) != 0 || (args[2] >> 10) != 0 {
            return None;
        }
        let sequence = SessionSequence::decode(args[0] & 0xffff_ffff)?;
        let width = ((args[0] >> 32) & 0xfff) as u16;
        let height = ((args[0] >> 44) & 0xfff) as u16;
        let pixel_fmt = CapturePixelFmt::from_u8(((args[0] >> 56) & 0xf) as u8)?;
        let target_kind = ((args[0] >> 60) & 0x7) as u8;
        let target = CaptureSourceTarget::decode_pair(target_kind, args[1])?;
        let fps = (args[2] & 0xff) as u8;
        let tally_active = ((args[2] >> 8) & 1) != 0;
        let hide_cursor = ((args[2] >> 9) & 1) != 0;

        let spec = Self::new(
            sequence,
            target,
            width,
            height,
            fps,
            pixel_fmt,
            tally_active,
            hide_cursor,
        );
        spec.arm().ok()
    }
}
