// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Omanta-style Collar FileChooser Portal over PDX.
//!
//! Programs without ambient filesystem authority ask Shell (`OP_FILE_CHOOSER`) to
//! show Loom's file picker for a single file, multiple files, a save target, or a folder.
//! When the person confirms, Shell mints explicit `@/canonical/path` Collar grant token
//! metadata inside [`ChooserReply`] (`OP_FILE_CHOOSER_REPLY`), scoped to the target PD
//! and current Shell epoch.

/// App -> Shell: open a Collar FileChooser portal dialog.
pub const OP_FILE_CHOOSER: u64 = 0x2F3;
/// Shell -> App: FileChooser completion carrying status and Shell-minted `@/` grant metadata.
pub const OP_FILE_CHOOSER_REPLY: u64 = 0x2F4;
/// App -> Shell: cancel a pending FileChooser request by sequence.
pub const OP_FILE_CHOOSER_CANCEL: u64 = 0x2F5;

pub const MAX_TITLE_BYTES: usize = 64;
pub const MAX_SUGGESTED_NAME_BYTES: usize = 128;
pub const MAX_FILTERS: usize = 8;
pub const MAX_FILTER_LABEL_BYTES: usize = 32;
pub const MAX_FILTER_EXTS: usize = 8;
pub const MAX_EXT_BYTES: usize = 12;
pub const MAX_GRANT_TOKEN_BYTES: usize = 256;
pub const MAX_REPLY_GRANTS: usize = 16;

const WIRE_MAGIC: [u8; 4] = *b"FCV1";

/// Portal selection mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ChooserMode {
    OpenFile = 1,
    OpenMultiple = 2,
    SaveFile = 3,
    PickFolder = 4,
}

impl ChooserMode {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::OpenFile),
            2 => Some(Self::OpenMultiple),
            3 => Some(Self::SaveFile),
            4 => Some(Self::PickFolder),
            _ => None,
        }
    }

    pub const fn allows_multiple(self) -> bool {
        matches!(self, Self::OpenMultiple)
    }

    pub const fn is_save(self) -> bool {
        matches!(self, Self::SaveFile)
    }

    pub const fn is_folder(self) -> bool {
        matches!(self, Self::PickFolder)
    }
}

/// Extension filter entry displayed in the portal UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileFilter {
    pub label: [u8; MAX_FILTER_LABEL_BYTES],
    pub label_len: u8,
    pub extensions: [[u8; MAX_EXT_BYTES]; MAX_FILTER_EXTS],
    pub ext_lens: [u8; MAX_FILTER_EXTS],
    pub ext_count: u8,
}

impl FileFilter {
    pub const EMPTY: Self = Self {
        label: [0; MAX_FILTER_LABEL_BYTES],
        label_len: 0,
        extensions: [[0; MAX_EXT_BYTES]; MAX_FILTER_EXTS],
        ext_lens: [0; MAX_FILTER_EXTS],
        ext_count: 0,
    };

    /// Creates a filter with human label and lowercase extension suffixes (leading `.` stripped).
    /// An empty `exts` slice matches any file.
    pub fn new(label: &[u8], exts: &[&[u8]]) -> Option<Self> {
        if label.is_empty()
            || label.len() > MAX_FILTER_LABEL_BYTES
            || exts.len() > MAX_FILTER_EXTS
            || label.iter().any(|b| *b == 0 || b.is_ascii_control())
        {
            return None;
        }
        let mut filter = Self::EMPTY;
        filter.label[..label.len()].copy_from_slice(label);
        filter.label_len = label.len() as u8;

        for (idx, raw_ext) in exts.iter().enumerate() {
            let ext = raw_ext.strip_prefix(b".").unwrap_or(raw_ext);
            if ext.is_empty()
                || ext.len() > MAX_EXT_BYTES
                || !ext
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
            {
                return None;
            }
            for (dst, src) in filter.extensions[idx][..ext.len()].iter_mut().zip(ext) {
                *dst = src.to_ascii_lowercase();
            }
            filter.ext_lens[idx] = ext.len() as u8;
        }
        filter.ext_count = exts.len() as u8;
        Some(filter)
    }

    pub fn label(&self) -> &[u8] {
        &self.label[..self.label_len as usize]
    }

    pub fn extension(&self, index: usize) -> Option<&[u8]> {
        if index >= self.ext_count as usize {
            return None;
        }
        Some(&self.extensions[index][..self.ext_lens[index] as usize])
    }

    /// True if `filename` matches any extension in this filter (case-insensitive),
    /// or always true when `ext_count == 0`.
    pub fn matches_name(&self, filename: &[u8]) -> bool {
        if self.ext_count == 0 {
            return true;
        }
        let base = filename.rsplit(|b| *b == b'/').next().unwrap_or(filename);
        let Some(dot) = base.iter().rposition(|b| *b == b'.') else {
            return false;
        };
        let ext = &base[dot + 1..];
        if ext.is_empty() || ext.len() > MAX_EXT_BYTES {
            return false;
        }
        let mut low = [0u8; MAX_EXT_BYTES];
        for (dst, src) in low[..ext.len()].iter_mut().zip(ext) {
            *dst = src.to_ascii_lowercase();
        }
        let candidate = &low[..ext.len()];
        (0..self.ext_count as usize).any(|i| self.extension(i) == Some(candidate))
    }
}

/// Request sent by a confined program to Shell to open the FileChooser portal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChooserRequest {
    pub sequence: u32,
    pub caller_pd: u32,
    pub mode: ChooserMode,
    pub title: [u8; MAX_TITLE_BYTES],
    pub title_len: u8,
    pub suggested_name: [u8; MAX_SUGGESTED_NAME_BYTES],
    pub suggested_name_len: u8,
    pub filters: [FileFilter; MAX_FILTERS],
    pub filter_count: u8,
    pub read_only_grant: bool,
}

impl ChooserRequest {
    pub const fn new(sequence: u32, caller_pd: u32, mode: ChooserMode) -> Option<Self> {
        if sequence == 0 || caller_pd == 0 {
            return None;
        }
        Some(Self {
            sequence,
            caller_pd,
            mode,
            title: [0; MAX_TITLE_BYTES],
            title_len: 0,
            suggested_name: [0; MAX_SUGGESTED_NAME_BYTES],
            suggested_name_len: 0,
            filters: [FileFilter::EMPTY; MAX_FILTERS],
            filter_count: 0,
            read_only_grant: matches!(mode, ChooserMode::OpenFile | ChooserMode::OpenMultiple),
        })
    }

    pub fn with_title(mut self, title: &[u8]) -> Option<Self> {
        if title.len() > MAX_TITLE_BYTES || title.iter().any(|b| *b == 0 || b.is_ascii_control()) {
            return None;
        }
        self.title.fill(0);
        self.title[..title.len()].copy_from_slice(title);
        self.title_len = title.len() as u8;
        Some(self)
    }

    /// Sets a suggested basename (no `/`, no `..`, no `@` prefix, no control chars).
    pub fn with_suggested_name(mut self, name: &[u8]) -> Option<Self> {
        if name.is_empty()
            || name.len() > MAX_SUGGESTED_NAME_BYTES
            || name == b"."
            || name == b".."
            || name.first() == Some(&b'@')
            || name.iter().any(|b| *b == b'/' || *b == 0 || b.is_ascii_control())
        {
            return None;
        }
        self.suggested_name.fill(0);
        self.suggested_name[..name.len()].copy_from_slice(name);
        self.suggested_name_len = name.len() as u8;
        Some(self)
    }

    pub fn with_filter(mut self, filter: FileFilter) -> Option<Self> {
        if filter.label_len == 0 || self.filter_count as usize >= MAX_FILTERS {
            return None;
        }
        self.filters[self.filter_count as usize] = filter;
        self.filter_count += 1;
        Some(self)
    }

    pub const fn with_read_only(mut self, read_only: bool) -> Option<Self> {
        if self.mode.is_save() && read_only {
            return None;
        }
        self.read_only_grant = read_only;
        Some(self)
    }

    pub fn title(&self) -> &[u8] {
        &self.title[..self.title_len as usize]
    }

    pub fn suggested_name(&self) -> &[u8] {
        &self.suggested_name[..self.suggested_name_len as usize]
    }

    pub fn filters(&self) -> &[FileFilter] {
        &self.filters[..self.filter_count as usize]
    }

    /// Packs the PDX call header words for `OP_FILE_CHOOSER`.
    pub const fn wire_open(&self) -> [u64; 3] {
        let flags = (self.mode as u64)
            | ((self.read_only_grant as u64) << 8)
            | ((self.filter_count as u64) << 16)
            | ((self.suggested_name_len as u64) << 24);
        [self.sequence as u64, flags, self.caller_pd as u64]
    }

    pub fn decode_open(args: [u64; 3]) -> Option<(u32, ChooserMode, bool, u8, u8, u32)> {
        let sequence = u32::try_from(args[0]).ok()?;
        let caller_pd = u32::try_from(args[2]).ok()?;
        if sequence == 0 || caller_pd == 0 || (args[1] >> 32) != 0 {
            return None;
        }
        let mode = ChooserMode::from_u8((args[1] & 0xFF) as u8)?;
        let read_only = ((args[1] >> 8) & 1) != 0;
        if ((args[1] >> 9) & 0x7F) != 0 {
            return None;
        }
        if mode.is_save() && read_only {
            return None;
        }
        let filter_count = ((args[1] >> 16) & 0xFF) as u8;
        let suggested_len = ((args[1] >> 24) & 0xFF) as u8;
        if filter_count as usize > MAX_FILTERS || suggested_len as usize > MAX_SUGGESTED_NAME_BYTES {
            return None;
        }
        Some((sequence, mode, read_only, filter_count, suggested_len, caller_pd))
    }

    /// Encodes the complete request into a shared-memory byte buffer.
    pub fn encode_bytes(&self, out: &mut [u8]) -> Option<usize> {
        let mut pos = 0usize;
        let put = |buf: &mut [u8], pos: &mut usize, bytes: &[u8]| -> Option<()> {
            let end = pos.checked_add(bytes.len())?;
            buf.get_mut(*pos..end)?.copy_from_slice(bytes);
            *pos = end;
            Some(())
        };
        put(out, &mut pos, &WIRE_MAGIC)?;
        put(out, &mut pos, &self.sequence.to_le_bytes())?;
        put(out, &mut pos, &self.caller_pd.to_le_bytes())?;
        put(
            out,
            &mut pos,
            &[
                self.mode as u8,
                self.read_only_grant as u8,
                self.title_len,
                self.suggested_name_len,
                self.filter_count,
            ],
        )?;
        put(out, &mut pos, self.title())?;
        put(out, &mut pos, self.suggested_name())?;
        for f in self.filters() {
            put(out, &mut pos, &[f.label_len, f.ext_count])?;
            put(out, &mut pos, f.label())?;
            for i in 0..f.ext_count as usize {
                let ext = f.extension(i)?;
                put(out, &mut pos, &[ext.len() as u8])?;
                put(out, &mut pos, ext)?;
            }
        }
        Some(pos)
    }

    /// Decodes a complete request from a shared-memory byte buffer.
    pub fn decode_bytes(input: &[u8]) -> Option<Self> {
        let mut pos = 0usize;
        let take = |pos: &mut usize, len: usize| -> Option<&[u8]> {
            let end = pos.checked_add(len)?;
            let slice = input.get(*pos..end)?;
            *pos = end;
            Some(slice)
        };
        if take(&mut pos, 4)? != WIRE_MAGIC {
            return None;
        }
        let sequence = u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?);
        let caller_pd = u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?);
        let hdr = take(&mut pos, 5)?;
        let mode = ChooserMode::from_u8(hdr[0])?;
        if hdr[1] > 1 {
            return None;
        }
        let read_only = hdr[1] != 0;
        let title_len = hdr[2] as usize;
        let suggested_len = hdr[3] as usize;
        let filter_count = hdr[4] as usize;
        if title_len > MAX_TITLE_BYTES
            || suggested_len > MAX_SUGGESTED_NAME_BYTES
            || filter_count > MAX_FILTERS
        {
            return None;
        }
        let mut req = Self::new(sequence, caller_pd, mode)?.with_read_only(read_only)?;
        let title = take(&mut pos, title_len)?;
        if title_len > 0 {
            req = req.with_title(title)?;
        }
        let suggested = take(&mut pos, suggested_len)?;
        if suggested_len > 0 {
            req = req.with_suggested_name(suggested)?;
        }
        for _ in 0..filter_count {
            let fh = take(&mut pos, 2)?;
            let l_len = fh[0] as usize;
            let e_count = fh[1] as usize;
            if l_len == 0 || l_len > MAX_FILTER_LABEL_BYTES || e_count > MAX_FILTER_EXTS {
                return None;
            }
            let label = take(&mut pos, l_len)?;
            let mut exts_buf: [&[u8]; MAX_FILTER_EXTS] = [&[]; MAX_FILTER_EXTS];
            for slot in exts_buf[..e_count].iter_mut() {
                let elen = take(&mut pos, 1)?[0] as usize;
                if elen == 0 || elen > MAX_EXT_BYTES {
                    return None;
                }
                *slot = take(&mut pos, elen)?;
            }
            let filter = FileFilter::new(label, &exts_buf[..e_count])?;
            req = req.with_filter(filter)?;
        }
        if pos != input.len() {
            return None;
        }
        Some(req)
    }
}

/// Completion status returned in [`ChooserReply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ChooserStatus {
    Granted = 0,
    Cancelled = 1,
    Denied = 2,
    Busy = 3,
    Invalid = 4,
}

impl ChooserStatus {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Granted),
            1 => Some(Self::Cancelled),
            2 => Some(Self::Denied),
            3 => Some(Self::Busy),
            4 => Some(Self::Invalid),
            _ => None,
        }
    }
}

/// Shell-minted Collar grant token metadata (`@/canonical/path`) returned to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollarGrantToken {
    pub token: [u8; MAX_GRANT_TOKEN_BYTES],
    pub len: u16,
    pub grant_id: u64,
    pub writable: bool,
    pub is_dir: bool,
}

impl CollarGrantToken {
    pub const EMPTY: Self = Self {
        token: [0; MAX_GRANT_TOKEN_BYTES],
        len: 0,
        grant_id: 0,
        writable: false,
        is_dir: false,
    };

    /// Constructs a validated Shell-minted `@/path` grant token.
    ///
    /// Rejects non-`@/path` prefixes, root `@/` (ambient filesystem authority),
    /// `.` / `..` segments, `//`, whitespace, NUL, and control characters.
    pub fn new(token: &[u8], grant_id: u64, writable: bool, is_dir: bool) -> Option<Self> {
        if grant_id == 0
            || token.len() <= 2
            || token.len() > MAX_GRANT_TOKEN_BYTES
            || !token.starts_with(b"@/")
        {
            return None;
        }
        let path = &token[1..];
        if path.iter().any(|b| *b == 0 || *b == b' ' || b.is_ascii_control() || *b == b'@') {
            return None;
        }
        if path.ends_with(b"/") {
            return None;
        }
        for seg in path[1..].split(|b| *b == b'/') {
            if seg.is_empty() || seg == b"." || seg == b".." {
                return None;
            }
        }
        let mut out = Self::EMPTY;
        out.token[..token.len()].copy_from_slice(token);
        out.len = token.len() as u16;
        out.grant_id = grant_id;
        out.writable = writable;
        out.is_dir = is_dir;
        Some(out)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.token[..self.len as usize]
    }

    pub fn path_bytes(&self) -> &[u8] {
        if self.len > 1 {
            &self.token[1..self.len as usize]
        } else {
            &[]
        }
    }
}

/// Shell's reply to a [`ChooserRequest`], carrying minted Collar `@/` grant tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChooserReply {
    pub sequence: u32,
    pub target_pd: u32,
    pub mode: ChooserMode,
    pub status: ChooserStatus,
    pub shell_epoch: u64,
    pub grants: [CollarGrantToken; MAX_REPLY_GRANTS],
    pub grant_count: u8,
}

impl ChooserReply {
    pub const fn non_grant(
        sequence: u32,
        target_pd: u32,
        mode: ChooserMode,
        status: ChooserStatus,
        shell_epoch: u64,
    ) -> Option<Self> {
        if sequence == 0
            || target_pd == 0
            || shell_epoch == 0
            || matches!(status, ChooserStatus::Granted)
        {
            return None;
        }
        Some(Self {
            sequence,
            target_pd,
            mode,
            status,
            shell_epoch,
            grants: [CollarGrantToken::EMPTY; MAX_REPLY_GRANTS],
            grant_count: 0,
        })
    }

    pub fn cancelled(request: &ChooserRequest, shell_epoch: u64) -> Option<Self> {
        Self::non_grant(
            request.sequence,
            request.caller_pd,
            request.mode,
            ChooserStatus::Cancelled,
            shell_epoch,
        )
    }

    pub fn denied(request: &ChooserRequest, shell_epoch: u64) -> Option<Self> {
        Self::non_grant(
            request.sequence,
            request.caller_pd,
            request.mode,
            ChooserStatus::Denied,
            shell_epoch,
        )
    }

    /// Constructs a `Granted` reply enforcing mode arity, folder/file kind, and read-only rules.
    pub fn granted(
        request: &ChooserRequest,
        shell_epoch: u64,
        grants: &[CollarGrantToken],
    ) -> Option<Self> {
        if shell_epoch == 0 || grants.is_empty() || grants.len() > MAX_REPLY_GRANTS {
            return None;
        }
        if !request.mode.allows_multiple() && grants.len() != 1 {
            return None;
        }
        let mut out_grants = [CollarGrantToken::EMPTY; MAX_REPLY_GRANTS];
        for (idx, g) in grants.iter().enumerate() {
            // Re-validate token invariants so forged structs cannot slip through.
            let checked = CollarGrantToken::new(g.as_bytes(), g.grant_id, g.writable, g.is_dir)?;
            match request.mode {
                ChooserMode::PickFolder => {
                    if !checked.is_dir {
                        return None;
                    }
                }
                ChooserMode::SaveFile => {
                    if checked.is_dir || !checked.writable {
                        return None;
                    }
                }
                ChooserMode::OpenFile | ChooserMode::OpenMultiple => {
                    if checked.is_dir || (request.read_only_grant && checked.writable) {
                        return None;
                    }
                }
            }
            out_grants[idx] = checked;
        }
        Some(Self {
            sequence: request.sequence,
            target_pd: request.caller_pd,
            mode: request.mode,
            status: ChooserStatus::Granted,
            shell_epoch,
            grants: out_grants,
            grant_count: grants.len() as u8,
        })
    }

    pub fn grants(&self) -> &[CollarGrantToken] {
        &self.grants[..self.grant_count as usize]
    }

    /// Packs summary fields for `OP_FILE_CHOOSER_REPLY`.
    pub const fn wire_summary(&self) -> [u64; 3] {
        let arg0 = (self.sequence as u64)
            | ((self.mode as u64) << 32)
            | ((self.status as u64) << 40)
            | ((self.grant_count as u64) << 48);
        [arg0, self.target_pd as u64, self.shell_epoch]
    }

    pub fn decode_summary(args: [u64; 3]) -> Option<(u32, ChooserMode, ChooserStatus, u8, u32, u64)> {
        if (args[0] >> 56) != 0 {
            return None;
        }
        let sequence = (args[0] & 0xffff_ffff) as u32;
        let mode = ChooserMode::from_u8(((args[0] >> 32) & 0xff) as u8)?;
        let status = ChooserStatus::from_u8(((args[0] >> 40) & 0xff) as u8)?;
        let grant_count = ((args[0] >> 48) & 0xff) as u8;
        let target_pd = u32::try_from(args[1]).ok()?;
        let shell_epoch = args[2];
        if sequence == 0 || target_pd == 0 || shell_epoch == 0 {
            return None;
        }
        match status {
            ChooserStatus::Granted => {
                if grant_count == 0
                    || grant_count as usize > MAX_REPLY_GRANTS
                    || (!mode.allows_multiple() && grant_count != 1)
                {
                    return None;
                }
            }
            _ => {
                if grant_count != 0 {
                    return None;
                }
            }
        }
        Some((sequence, mode, status, grant_count, target_pd, shell_epoch))
    }

    /// Encodes the complete reply (including minted `@/` grant tokens) into a byte buffer.
    pub fn encode_bytes(&self, out: &mut [u8]) -> Option<usize> {
        let mut pos = 0usize;
        let put = |buf: &mut [u8], pos: &mut usize, bytes: &[u8]| -> Option<()> {
            let end = pos.checked_add(bytes.len())?;
            buf.get_mut(*pos..end)?.copy_from_slice(bytes);
            *pos = end;
            Some(())
        };
        put(out, &mut pos, &WIRE_MAGIC)?;
        put(out, &mut pos, &self.sequence.to_le_bytes())?;
        put(out, &mut pos, &self.target_pd.to_le_bytes())?;
        put(out, &mut pos, &self.shell_epoch.to_le_bytes())?;
        put(
            out,
            &mut pos,
            &[self.mode as u8, self.status as u8, self.grant_count],
        )?;
        for g in self.grants() {
            put(out, &mut pos, &g.grant_id.to_le_bytes())?;
            put(
                out,
                &mut pos,
                &g.len.to_le_bytes(),
            )?;
            put(out, &mut pos, &[g.writable as u8, g.is_dir as u8])?;
            put(out, &mut pos, g.as_bytes())?;
        }
        Some(pos)
    }

    /// Decodes a complete reply from a byte buffer, validating every Collar grant token.
    pub fn decode_bytes(input: &[u8]) -> Option<Self> {
        let mut pos = 0usize;
        let take = |pos: &mut usize, len: usize| -> Option<&[u8]> {
            let end = pos.checked_add(len)?;
            let slice = input.get(*pos..end)?;
            *pos = end;
            Some(slice)
        };
        if take(&mut pos, 4)? != WIRE_MAGIC {
            return None;
        }
        let sequence = u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?);
        let target_pd = u32::from_le_bytes(take(&mut pos, 4)?.try_into().ok()?);
        let shell_epoch = u64::from_le_bytes(take(&mut pos, 8)?.try_into().ok()?);
        let hdr = take(&mut pos, 3)?;
        let mode = ChooserMode::from_u8(hdr[0])?;
        let status = ChooserStatus::from_u8(hdr[1])?;
        let grant_count = hdr[2] as usize;
        if sequence == 0 || target_pd == 0 || shell_epoch == 0 {
            return None;
        }
        if status != ChooserStatus::Granted {
            if grant_count != 0 || pos != input.len() {
                return None;
            }
            return Self::non_grant(sequence, target_pd, mode, status, shell_epoch);
        }
        if grant_count == 0
            || grant_count > MAX_REPLY_GRANTS
            || (!mode.allows_multiple() && grant_count != 1)
        {
            return None;
        }
        let mut grants = [CollarGrantToken::EMPTY; MAX_REPLY_GRANTS];
        for slot in grants[..grant_count].iter_mut() {
            let grant_id = u64::from_le_bytes(take(&mut pos, 8)?.try_into().ok()?);
            let len = u16::from_le_bytes(take(&mut pos, 2)?.try_into().ok()?) as usize;
            let flags = take(&mut pos, 2)?;
            if flags[0] > 1 || flags[1] > 1 || len > MAX_GRANT_TOKEN_BYTES {
                return None;
            }
            let writable = flags[0] != 0;
            let is_dir = flags[1] != 0;
            let tok_bytes = take(&mut pos, len)?;
            let checked = CollarGrantToken::new(tok_bytes, grant_id, writable, is_dir)?;
            match mode {
                ChooserMode::PickFolder if !checked.is_dir => return None,
                ChooserMode::SaveFile if checked.is_dir || !checked.writable => return None,
                ChooserMode::OpenFile | ChooserMode::OpenMultiple if checked.is_dir => return None,
                _ => {}
            }
            *slot = checked;
        }
        if pos != input.len() {
            return None;
        }
        Some(Self {
            sequence,
            target_pd,
            mode,
            status,
            shell_epoch,
            grants,
            grant_count: grant_count as u8,
        })
    }
}
