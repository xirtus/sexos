//! System-volume snapshot/publication facade. Mounted/home backends are refused
//! until they provide equivalent stable identity; no weaker path fallback.
extern crate alloc;
use crate::restricted_job::{Error, Job, Object, Source, MAX_PLUGIN, MAX_REQUEST, MAX_TRANSFER};
use alloc::vec::Vec;
use sex_pdx::restricted_helper::Subject;
use sexfs::{Device, Fs, Kind};
pub struct Storage {
    pub tokens: [Vec<u8>; 3],
    output: Vec<u8>,
    parent_path: Vec<u8>,
    parent_id: u64,
    plugin_id: u64,
    input_id: u64,
}
fn copy(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(bytes.len())
        .map_err(|_| Error::Memory)?;
    result.extend_from_slice(bytes);
    Ok(result)
}
pub fn canonical(path: &[u8]) -> bool {
    path.len() > 1
        && path.len() <= 192
        && path[0] == b'/'
        && !path.iter().any(|b| *b <= 32 || *b == 127)
        && !path[1..]
            .split(|b| *b == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b"..")
}
fn under(path: &[u8], prefix: &[u8]) -> bool {
    path == prefix || path.starts_with(prefix) && path.get(prefix.len()) == Some(&b'/')
}
impl Storage {
    /// Absolute paths are already grant-confined by fs_service against the exact
    /// stamped parent task. This function must own one Files dispatcher turn.
    pub fn capture<D: Device>(
        fs: &mut Fs<'_, D>,
        tokens: [&[u8]; 3],
        absolute: [&[u8]; 3],
        id: u32,
        parent: Subject,
        cookie: u32,
        nonce: [u8; 16],
    ) -> Result<(Self, Job), Error> {
        if absolute.iter().any(|path| !canonical(path))
            || (0..3).any(|i| tokens[i].get(1..) != Some(absolute[i]))
            || tokens
                .iter()
                .any(|path| !path.starts_with(b"@/") || path.len() > 192)
        {
            return Err(Error::Bounds);
        }
        if absolute
            .iter()
            .any(|path| under(path, b"/media") || under(path, b"/home"))
        {
            return Err(Error::Unsupported);
        }
        if absolute[0] == absolute[1] || absolute[0] == absolute[2] || absolute[1] == absolute[2] {
            return Err(Error::Alias);
        }
        fs.commit().map_err(|_| Error::Io)?;
        let plugin = fs.open(absolute[0]).map_err(|_| Error::Io)?;
        let input = fs.open(absolute[1]).map_err(|_| Error::Io)?;
        if plugin.kind != Kind::File || input.kind != Kind::File {
            return Err(Error::Header);
        }
        if plugin.id == input.id {
            return Err(Error::Alias);
        }
        match fs.open(absolute[2]) {
            Err(sexfs::Error::NotFound) => {}
            Ok(_) => return Err(Error::Alias),
            Err(_) => return Err(Error::Io),
        }
        let split = absolute[2]
            .iter()
            .rposition(|b| *b == b'/')
            .ok_or(Error::Bounds)?;
        let parent_path = if split == 0 {
            b"/".as_slice()
        } else {
            &absolute[2][..split]
        };
        let directory = fs.open(parent_path).map_err(|_| Error::Io)?;
        if directory.kind != Kind::Dir {
            return Err(Error::Header);
        }
        let p = Source::capture(
            Object {
                volume: 1,
                id: plugin.id,
            },
            plugin.size,
            MAX_PLUGIN,
            |at, bytes| fs.read(plugin.id, at, bytes).map_err(|_| Error::Io),
        )?;
        let i = Source::capture(
            Object {
                volume: 1,
                id: input.id,
            },
            input.size,
            MAX_REQUEST,
            |at, bytes| fs.read(input.id, at, bytes).map_err(|_| Error::Io),
        )?;
        let job = Job::new(id, parent, cookie, nonce, p, i)?;
        let storage = Self {
            tokens: [copy(tokens[0])?, copy(tokens[1])?, copy(tokens[2])?],
            output: copy(absolute[2])?,
            parent_path: copy(parent_path)?,
            parent_id: directory.id,
            plugin_id: plugin.id,
            input_id: input.id,
        };
        Ok((storage, job))
    }
    /// Creates a new object only; never opens an existing destination for writes.
    /// Parent directory IDs are monotonic sexfs object IDs, rechecked in the same
    /// dispatcher turn. Rename/delete/replacement cannot redirect publication.
    /// Check current parent+child lifecycle and committed provider lease before
    /// every bounded write and before/after commit. In-flight accepted disk I/O
    /// may finish; revoked work never receives successful result publication.
    pub fn publish<D: Device>(
        &self,
        fs: &mut Fs<'_, D>,
        bytes: &[u8],
        now: u64,
        mut live: impl FnMut() -> bool,
    ) -> Result<(), Error> {
        if bytes.len() < crate::restricted_job::HEADER
            || bytes.len() > crate::restricted_job::HEADER + crate::restricted_job::MAX_FRAMES * 8
        {
            return Err(Error::Bounds);
        }
        if !live() {
            return Err(Error::Identity);
        }
        fs.commit().map_err(|_| Error::Io)?;
        let directory = fs.open(&self.parent_path).map_err(|_| Error::Io)?;
        if directory.id != self.parent_id || directory.kind != Kind::Dir {
            return Err(Error::Identity);
        }
        match fs.open(&self.output) {
            Err(sexfs::Error::NotFound) => {}
            Ok(_) => return Err(Error::Alias),
            Err(_) => return Err(Error::Io),
        }
        let outcome = (|| {
            if !live() {
                return Err(Error::Identity);
            }
            let output = fs.create(&self.output, now).map_err(|_| Error::Io)?;
            if output == self.plugin_id || output == self.input_id {
                return Err(Error::Alias);
            }
            let mut at = 0;
            while at < bytes.len() {
                if !live() {
                    return Err(Error::Identity);
                }
                let end = bytes.len().min(at + MAX_TRANSFER);
                let n = fs
                    .write(output, at as u64, &bytes[at..end], now)
                    .map_err(|_| Error::Io)?;
                if n == 0 || n > end - at {
                    return Err(Error::Io);
                }
                at += n;
            }
            if !live() {
                return Err(Error::Identity);
            }
            fs.commit().map_err(|_| Error::Io)?;
            if !live() {
                return Err(Error::Identity);
            }
            Ok(())
        })();
        if outcome.is_err() {
            let _ = fs.rollback();
        }
        outcome
    }
}
