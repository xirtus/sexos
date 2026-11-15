//! Immutable offline job buffers. No filesystem access or authority inferred from
//! filenames: admission supplies resolved objects and already checked parent scope.
extern crate alloc;
use alloc::vec::Vec;
use sex_pdx::restricted_helper::{Binding, Subject};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};
pub const MAX_PLUGIN: usize = 16 * 1024 * 1024;
pub const MAX_FRAMES: usize = 48_000 * 60;
pub const HEADER: usize = 40;
pub const MAX_REQUEST: usize = HEADER + MAX_FRAMES * 8 + 64 * 16;
pub const MAX_TRANSFER: usize = 248 * 1024;
pub const MAX_OPERATIONS: u32 = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Bounds,
    Memory,
    Identity,
    Alias,
    Header,
    Audio,
    State,
    ReadOnly,
    Budget,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Object {
    pub volume: u64,
    pub id: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blob {
    Plugin,
    Request,
    Output,
}
pub struct Source {
    object: Object,
    bytes: Zeroizing<Vec<u8>>,
    hash: [u8; 32],
}
impl Source {
    /// Size is checked BEFORE allocation. Reader belongs to the sole Files turn;
    /// the final one-byte probe refuses an inconsistent/growing source.
    pub fn capture(
        object: Object,
        size: u64,
        limit: usize,
        mut read: impl FnMut(u64, &mut [u8]) -> Result<usize, Error>,
    ) -> Result<Self, Error> {
        if object.id == 0 || size == 0 || size > limit as u64 {
            return Err(Error::Bounds);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        bytes
            .try_reserve_exact(size as usize)
            .map_err(|_| Error::Memory)?;
        bytes.resize(size as usize, 0);
        let mut at = 0;
        while at < bytes.len() {
            let end = bytes.len().min(at + MAX_TRANSFER);
            let n = read(at as u64, &mut bytes[at..end])?;
            if n == 0 || n > end - at {
                return Err(Error::Bounds);
            }
            at += n;
        }
        let mut probe = [0];
        if read(size, &mut probe)? != 0 {
            return Err(Error::Bounds);
        }
        let hash = Sha256::digest(bytes.as_slice()).into();
        Ok(Self {
            object,
            bytes,
            hash,
        })
    }
}
pub struct Job {
    id: u32,
    parent: Subject,
    cookie: u32,
    nonce: [u8; 16],
    frames: usize,
    plugin: Source,
    request: Source,
    output: Zeroizing<Vec<u8>>,
    binding: Option<Binding>,
    bound_once: bool,
    created: bool,
    written: usize,
    sealed: Option<[u8; 32]>,
    published: bool,
    operations: u32,
    read_bytes: [usize; 2],
}
fn words(hash: [u8; 32]) -> [u64; 4] {
    core::array::from_fn(|i| u64::from_le_bytes(hash[i * 8..i * 8 + 8].try_into().unwrap()))
}
fn nonce_words(nonce: [u8; 16]) -> [u64; 2] {
    core::array::from_fn(|i| u64::from_le_bytes(nonce[i * 8..i * 8 + 8].try_into().unwrap()))
}
fn samples(bytes: &[u8]) -> bool {
    bytes.len() % 4 == 0
        && bytes.chunks_exact(4).all(|sample| {
            u32::from_le_bytes(sample.try_into().unwrap()) & 0x7fff_ffff <= 0x4180_0000
        })
}
fn request_frames(bytes: &[u8], nonce: [u8; 16]) -> Result<usize, Error> {
    if bytes.len() < HEADER
        || bytes.len() > MAX_REQUEST
        || &bytes[..8] != b"MOANREQ1"
        || bytes[8..24] != nonce
        || bytes[24..28] != 48_000u32.to_le_bytes()
        || bytes[32..36] != 2u32.to_le_bytes()
    {
        return Err(Error::Header);
    }
    let frames = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
    let count = u32::from_le_bytes(bytes[36..40].try_into().unwrap()) as usize;
    if frames == 0
        || frames > MAX_FRAMES
        || count > 64
        || bytes.len() != HEADER + frames * 8 + count * 16
    {
        return Err(Error::Bounds);
    }
    let end = HEADER + frames * 8;
    if !samples(&bytes[HEADER..end]) {
        return Err(Error::Audio);
    }
    let mut ids = [0u32; 64];
    for (index, param) in bytes[end..].chunks_exact(16).enumerate() {
        let id = u32::from_le_bytes(param[..4].try_into().unwrap());
        let value = u64::from_le_bytes(param[8..].try_into().unwrap()) & 0x7fff_ffff_ffff_ffff;
        if id == u32::MAX
            || param[4..8] != [0; 4]
            || ids[..index].contains(&id)
            || value > 1_000_000_000f64.to_bits()
        {
            return Err(Error::Header);
        }
        ids[index] = id;
    }
    Ok(frames)
}
impl Job {
    pub fn new(
        id: u32,
        parent: Subject,
        cookie: u32,
        nonce: [u8; 16],
        plugin: Source,
        request: Source,
    ) -> Result<Self, Error> {
        if id == 0
            || id > 0xff_ffff
            || parent.pd == 0
            || parent.task == 0
            || cookie == 0
            || nonce == [0; 16]
        {
            return Err(Error::Identity);
        }
        if plugin.object == request.object {
            return Err(Error::Alias);
        }
        if plugin.bytes.len() > MAX_PLUGIN {
            return Err(Error::Bounds);
        }
        let frames = request_frames(&request.bytes, nonce)?;
        let mut output = Zeroizing::new(Vec::new());
        output
            .try_reserve_exact(HEADER + frames * 8)
            .map_err(|_| Error::Memory)?;
        output.resize(HEADER + frames * 8, 0);
        Ok(Self {
            id,
            parent,
            cookie,
            nonce,
            frames,
            plugin,
            request,
            output,
            binding: None,
            bound_once: false,
            created: false,
            written: 0,
            sealed: None,
            published: false,
            operations: 0,
            read_bytes: [0; 2],
        })
    }
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn parent(&self) -> Subject {
        self.parent
    }
    pub fn nonce(&self) -> [u8; 16] {
        self.nonce
    }
    pub fn plugin_hash(&self) -> [u8; 32] {
        self.plugin.hash
    }
    pub fn input_hash(&self) -> [u8; 32] {
        self.request.hash
    }
    pub fn binding(&self) -> Option<Binding> {
        self.binding
    }
    /// Provisional association only. Data methods additionally require the
    /// provider's exact committed unexpired lease, supplied by the runtime hook.
    pub fn bind(&mut self, binding: Binding) -> Result<(), Error> {
        if self.bound_once
            || binding.parent != self.parent
            || binding.cookie != self.cookie
            || binding.nonce != nonce_words(self.nonce)
            || binding.plugin_hash != words(self.plugin.hash)
            || binding.input_hash != words(self.request.hash)
            || binding.ticket != self.id as u64
            || binding.child.pd == self.parent.pd
            || binding.child.pd == 0
            || binding.child.task == 0
        {
            return Err(Error::Identity);
        }
        self.binding = Some(binding);
        self.bound_once = true;
        Ok(())
    }
    fn authorize(
        &mut self,
        actor: Subject,
        binding: Binding,
        committed_live: bool,
    ) -> Result<(), Error> {
        if !committed_live || self.binding != Some(binding) || binding.child != actor {
            return Err(Error::Identity);
        }
        self.operations = self
            .operations
            .checked_add(1)
            .filter(|n| *n <= MAX_OPERATIONS)
            .ok_or(Error::Budget)?;
        Ok(())
    }
    pub fn size(
        &mut self,
        actor: Subject,
        binding: Binding,
        live: bool,
        blob: Blob,
    ) -> Result<usize, Error> {
        self.authorize(actor, binding, live)?;
        match blob {
            Blob::Plugin => Ok(self.plugin.bytes.len()),
            Blob::Request => Ok(self.request.bytes.len()),
            Blob::Output => {
                if self.created {
                    Ok(self.written)
                } else {
                    Err(Error::State)
                }
            }
        }
    }
    pub fn read(
        &mut self,
        actor: Subject,
        binding: Binding,
        live: bool,
        blob: Blob,
        offset: u64,
        length: usize,
    ) -> Result<&[u8], Error> {
        self.authorize(actor, binding, live)?;
        if length > MAX_TRANSFER {
            return Err(Error::Bounds);
        }
        let (data, index) = match blob {
            Blob::Plugin => (self.plugin.bytes.as_slice(), Some(0)),
            Blob::Request => (self.request.bytes.as_slice(), Some(1)),
            Blob::Output => {
                if !self.created {
                    return Err(Error::State);
                }
                (&self.output[..self.written], None)
            }
        };
        if offset > data.len() as u64 {
            return Err(Error::Bounds);
        }
        let at = offset as usize;
        let n = length.min(data.len() - at);
        if let Some(index) = index {
            let total = self.read_bytes[index]
                .checked_add(n)
                .filter(|n| *n <= data.len() * 2)
                .ok_or(Error::Budget)?;
            self.read_bytes[index] = total;
        }
        Ok(&data[at..at + n])
    }
    pub fn create(
        &mut self,
        actor: Subject,
        binding: Binding,
        live: bool,
        blob: Blob,
    ) -> Result<(), Error> {
        self.authorize(actor, binding, live)?;
        if blob != Blob::Output {
            return Err(Error::ReadOnly);
        }
        if self.created {
            return Err(Error::State);
        }
        self.created = true;
        Ok(())
    }
    pub fn truncate(
        &mut self,
        actor: Subject,
        binding: Binding,
        live: bool,
        blob: Blob,
        length: u64,
    ) -> Result<(), Error> {
        self.authorize(actor, binding, live)?;
        if blob != Blob::Output {
            return Err(Error::ReadOnly);
        }
        if !self.created || self.sealed.is_some() || length != 0 || self.written != 0 {
            return Err(Error::State);
        }
        Ok(())
    }
    /// Trusted wrapper supplies volatile reads from the child's own grant, after
    /// validating extent. It never lends an exclusive Rust slice to hostile code.
    pub fn write(
        &mut self,
        actor: Subject,
        binding: Binding,
        live: bool,
        blob: Blob,
        offset: u64,
        length: usize,
        mut byte: impl FnMut(usize) -> u8,
    ) -> Result<usize, Error> {
        self.authorize(actor, binding, live)?;
        if blob != Blob::Output {
            return Err(Error::ReadOnly);
        }
        if !self.created || self.sealed.is_some() {
            return Err(Error::State);
        }
        if length > MAX_TRANSFER
            || offset != self.written as u64
            || length > self.output.len() - self.written
        {
            return Err(Error::Bounds);
        }
        for i in 0..length {
            self.output[self.written + i] = byte(i);
        }
        self.written += length;
        Ok(length)
    }
    pub fn seal(
        &mut self,
        actor: Subject,
        binding: Binding,
        live: bool,
    ) -> Result<[u8; 32], Error> {
        self.authorize(actor, binding, live)?;
        if !self.created || self.written != self.output.len() {
            return Err(Error::State);
        }
        if let Some(hash) = self.sealed {
            return Ok(hash);
        }
        let bytes = self.output.as_slice();
        if &bytes[..8] != b"MOANPCM1"
            || bytes[8..24] != self.nonce
            || bytes[24..28] != 48_000u32.to_le_bytes()
            || bytes[28..32] != (self.frames as u32).to_le_bytes()
            || bytes[32..36] != 2u32.to_le_bytes()
            || bytes[36..40] != [0; 4]
            || !samples(&bytes[HEADER..])
        {
            return Err(Error::Header);
        }
        let hash = Sha256::digest(bytes).into();
        self.sealed = Some(hash);
        Ok(hash)
    }
    pub fn publication_bytes(&self) -> Result<&[u8], Error> {
        if self.sealed.is_none() {
            return Err(Error::State);
        }
        Ok(&self.output)
    }
    pub fn published(&mut self) {
        if self.sealed.is_some() {
            self.published = true;
        }
    }
    pub fn result(
        &self,
        parent: Subject,
        id: u32,
        nonce: [u8; 16],
    ) -> Result<(&[u8], [u8; 32]), Error> {
        if parent != self.parent || id != self.id || nonce != self.nonce {
            return Err(Error::Identity);
        }
        if !self.published {
            return Err(Error::State);
        }
        Ok((&self.output, self.sealed.ok_or(Error::State)?))
    }
    pub fn retire_child(&mut self) {
        self.binding = None;
        self.plugin.bytes.zeroize();
        self.request.bytes.zeroize();
    }
}
