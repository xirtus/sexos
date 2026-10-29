use crate::backends::FsBackend;
use crate::messages;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::RwLock;

/// RamFs: Bounded RAM-backed flat filesystem.
///
/// CONTRACT (RAMFS_CONTRACT_LOCK_V1 + NAMESPACE_CAPS_V1):
/// - Max 64 files
/// - File names ≤ 24 bytes
/// - File content ≤ 4096 bytes
/// - Flat namespace (no directories)
/// - All handles validated before use
/// - Deterministic errors for all failure modes
/// - Close releases handle but file data persists (reopen by name)
/// - No POSIX semantics claim
/// - Each file has an owner PD; read/write/close/stat require matching caller_pd
/// - Server-internal (caller_pd == 0) bypasses owner checks
pub struct RamFs {
    files: RwLock<Vec<FileEntry>>,
    caps: RwLock<Vec<CapRecord>>,
    next_object_id: AtomicU64,
    next_handle: AtomicU64,
}

/// A slot in the RamFS. `active: false` means handle is released
/// but name+data persists for reopen-by-name.
struct FileEntry {
    handle: u64,
    object_id: u64,
    active: bool,
    owner_pd: u32,
    cap_generation: u64,
    name: [u8; messages::RAMFS_MAX_NAME],
    name_len: u8,
    data: Vec<u8>,
}

#[derive(Clone, Copy)]
struct CapRecord {
    object_id: u64,
    subject_pd: u32,
    rights: u8,
    generation: u64,
    valid: bool,
}

pub const CAP_RIGHT_READ: u8 = 1 << 0;
pub const CAP_RIGHT_WRITE: u8 = 1 << 1;
pub const CAP_RIGHT_APPEND: u8 = 1 << 2;
pub const CAP_RIGHT_LIST: u8 = 1 << 3;
pub const CAP_RIGHT_DELETE: u8 = 1 << 4;
pub const CAP_RIGHT_GRANT: u8 = 1 << 5;
const CAP_RIGHT_OPEN_EXISTING: u8 =
    CAP_RIGHT_READ | CAP_RIGHT_WRITE | CAP_RIGHT_APPEND | CAP_RIGHT_DELETE | CAP_RIGHT_GRANT;
const CAP_MAX_RECORDS: usize = 256;

impl RamFs {
    /// Publish a complete owner-private snapshot in one service transition.
    /// Failed validation/allocation before the swap cannot expose a partial file.
    pub fn replace_owned(&self,handle:u64,data:&[u8],caller:u32)->Result<(),i64>{
        if data.len()>messages::RAMFS_MAX_FILE_SIZE{return Err(messages::ERR_OVERFLOW)}
        let mut files=self.files.write();
        let entry=Self::find_active_by_handle_mut(&mut files,handle).ok_or(messages::ERR_INVALID_HANDLE)?;
        if caller==0 || entry.owner_pd!=caller{return Err(messages::ERR_PERM_DENIED)}
        let mut next=Vec::new();
        next.try_reserve_exact(data.len()).map_err(|_|messages::ERR_FULL)?;
        next.extend_from_slice(data);entry.data=next;Ok(())
    }
    pub const fn new() -> Self {
        Self {
            files: RwLock::new(Vec::new()),
            caps: RwLock::new(Vec::new()),
            next_object_id: AtomicU64::new(1),
            next_handle: AtomicU64::new(1),
        }
    }

    /// Release a terminal subject's volatile leases, preserving saved bytes and
    /// valid leases held by other live subjects. Lock order matches open/write.
    pub fn retire_subject(&self, subject: u32) -> (usize,usize) {
        let mut files=self.files.write();
        let mut caps=self.caps.write();
        let mut revoked=0;
        for cap in caps.iter_mut() {
            if cap.valid && cap.subject_pd==subject { cap.valid=false; revoked+=1; }
        }
        let mut closed=0;
        for file in files.iter_mut() {
            if file.active && file.owner_pd==subject && !caps.iter().any(|c|
                c.valid && c.object_id==file.object_id && c.generation==file.cap_generation
                && c.subject_pd!=subject && c.rights & CAP_RIGHT_OPEN_EXISTING!=0) {
                file.active=false; closed+=1;
            }
        }
        (closed,revoked)
    }

    /// Find an active entry by handle.
    fn find_active_by_handle<'s>(files: &'s [FileEntry], handle: u64) -> Option<&'s FileEntry> {
        files.iter().find(|e| e.active && e.handle == handle)
    }

    /// Find an active entry by handle (mutable).
    fn find_active_by_handle_mut<'s>(files: &'s mut [FileEntry], handle: u64) -> Option<&'s mut FileEntry> {
        files.iter_mut().find(|e| e.active && e.handle == handle)
    }

    /// Find any entry (active or inactive) by name.
    #[allow(dead_code)]
    fn find_any_by_name<'s>(files: &'s [FileEntry], name: &[u8]) -> Option<&'s FileEntry> {
        files.iter().find(|e| {
            e.name_len as usize == name.len() && &e.name[..name.len()] == name
        })
    }

    /// Find any entry by name (mutable).
    fn find_any_by_name_mut<'s>(files: &'s mut [FileEntry], name: &[u8]) -> Option<&'s mut FileEntry> {
        files.iter_mut().find(|e| {
            e.name_len as usize == name.len() && &e.name[..name.len()] == name
        })
    }

    /// Allocate a new file entry with the given name (initially active).
    fn allocate(&self, files: &mut Vec<FileEntry>, name: &[u8], owner_pd: u32) -> Result<u64, i64> {
        let count = files.iter().filter(|e| e.active).count();
        if count >= messages::RAMFS_MAX_FILES {
            return Err(messages::ERR_FULL);
        }

        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        let mut name_buf = [0u8; messages::RAMFS_MAX_NAME];
        name_buf[..name.len()].copy_from_slice(name);

        files.push(FileEntry {
            handle,
            object_id: self.next_object_id.fetch_add(1, Ordering::SeqCst),
            active: true,
            owner_pd,
            cap_generation: 1,
            name: name_buf,
            name_len: name.len() as u8,
            data: Vec::new(),
        });
        Ok(handle)
    }

    /// Check whether `caller_pd` is allowed to access an entry for all bits in `required`.
    /// caller_pd == 0 (server-internal) and owner are both fast-path allows.
    fn check_access(
        caps: &[CapRecord],
        caller_pd: u32,
        entry: &FileEntry,
        required: u8,
    ) -> Result<(), i64> {
        if caller_pd == 0 || caller_pd == entry.owner_pd {
            Ok(())
        } else {
            let mut i = 0usize;
            while i < caps.len() {
                let cap = caps[i];
                if cap.valid
                    && cap.object_id == entry.object_id
                    && cap.subject_pd == caller_pd
                    && cap.generation == entry.cap_generation
                    && (cap.rights & required) == required
                {
                    return Ok(());
                }
                i += 1;
            }
            Err(messages::ERR_PERM_DENIED)
        }
    }

    /// Check whether `caller_pd` has at least one right from `required_any`.
    fn check_access_any(
        caps: &[CapRecord],
        caller_pd: u32,
        entry: &FileEntry,
        required_any: u8,
    ) -> Result<(), i64> {
        if caller_pd == 0 || caller_pd == entry.owner_pd {
            return Ok(());
        }
        let mut i = 0usize;
        while i < caps.len() {
            let cap = caps[i];
            if cap.valid
                && cap.object_id == entry.object_id
                && cap.subject_pd == caller_pd
                && cap.generation == entry.cap_generation
                && (cap.rights & required_any) != 0
            {
                return Ok(());
            }
            i += 1;
        }
        Err(messages::ERR_PERM_DENIED)
    }

    fn grant_cap_internal(
        caps: &mut Vec<CapRecord>,
        entry: &FileEntry,
        subject_pd: u32,
        rights: u8,
    ) -> Result<(), i64> {
        if rights == 0 {
            return Err(messages::ERR_PERM_DENIED);
        }
        let mut i = 0usize;
        while i < caps.len() {
            let rec = &mut caps[i];
            if rec.object_id == entry.object_id && rec.subject_pd == subject_pd {
                rec.rights = rights;
                rec.generation = entry.cap_generation;
                rec.valid = true;
                return Ok(());
            }
            i += 1;
        }
        if caps.len() >= CAP_MAX_RECORDS {
            return Err(messages::ERR_FULL);
        }
        caps.push(CapRecord {
            object_id: entry.object_id,
            subject_pd,
            rights,
            generation: entry.cap_generation,
            valid: true,
        });
        Ok(())
    }

    fn revoke_caps_internal(caps: &mut Vec<CapRecord>, object_id: u64) {
        let mut i = 0usize;
        while i < caps.len() {
            if caps[i].object_id == object_id {
                caps[i].valid = false;
            }
            i += 1;
        }
    }

    /// Proof helper: grant capabilities on an object by name.
    pub fn proof_grant_caps_by_name(
        &self,
        owner_pd: u32,
        name: &[u8],
        subject_pd: u32,
        rights: u8,
    ) -> Result<(), i64> {
        let files = self.files.read();
        let entry = Self::find_any_by_name(&files, name).ok_or(messages::ERR_NOT_FOUND)?;
        if owner_pd != 0 && owner_pd != entry.owner_pd {
            return Err(messages::ERR_PERM_DENIED);
        }
        let mut caps = self.caps.write();
        Self::grant_cap_internal(&mut caps, entry, subject_pd, rights)
    }

    /// Proof helper: revoke all caps for an object by name by bumping generation.
    pub fn proof_revoke_caps_by_name(&self, owner_pd: u32, name: &[u8]) -> Result<(), i64> {
        let mut files = self.files.write();
        let entry = Self::find_any_by_name_mut(&mut files, name).ok_or(messages::ERR_NOT_FOUND)?;
        if owner_pd != 0 && owner_pd != entry.owner_pd {
            return Err(messages::ERR_PERM_DENIED);
        }
        entry.cap_generation = entry.cap_generation.saturating_add(1);
        let object_id = entry.object_id;
        drop(files);
        let mut caps = self.caps.write();
        Self::revoke_caps_internal(&mut caps, object_id);
        Ok(())
    }

    /// Proof helper: stale generation cap insertion.
    pub fn proof_inject_stale_generation_cap(
        &self,
        name: &[u8],
        subject_pd: u32,
        rights: u8,
        stale_generation: u64,
    ) -> Result<(), i64> {
        let files = self.files.read();
        let entry = Self::find_any_by_name(&files, name).ok_or(messages::ERR_NOT_FOUND)?;
        let mut caps = self.caps.write();
        if caps.len() >= CAP_MAX_RECORDS {
            return Err(messages::ERR_FULL);
        }
        caps.push(CapRecord {
            object_id: entry.object_id,
            subject_pd,
            rights,
            generation: stale_generation,
            valid: true,
        });
        Ok(())
    }

    /// Return the global RamFS object_id for an open handle.
    ///
    /// Implements OP_RAMFS_OBJECT_ID (0x37). Caller must own the file or
    /// use caller_pd=0 (server-internal). Closes the OQ5 namespace gap:
    /// returns a SexFiles-assigned ID, not a client-local ID.
    pub fn object_id_for_handle(&self, handle: u64, caller_pd: u32) -> Result<u64, i64> {
        let files = self.files.read();
        let entry = Self::find_active_by_handle(&files, handle)
            .ok_or(messages::ERR_INVALID_HANDLE)?;
        if caller_pd != 0 && caller_pd != entry.owner_pd {
            return Err(messages::ERR_PERM_DENIED);
        }
        Ok(entry.object_id)
    }
}

impl FsBackend for RamFs {
    fn open(&self, name: &[u8], flags: u32, _mode: u32, caller_pd: u32) -> Result<u64, i64> {
        if name.len() > messages::RAMFS_MAX_NAME {
            return Err(messages::ERR_NAME_TOO_LONG);
        }
        if name.is_empty() {
            return Err(messages::ERR_NOT_FOUND);
        }

        let mut files = self.files.write();
        let caps = self.caps.read();

        // Check if file exists by name (any state)
        if let Some(existing) = Self::find_any_by_name_mut(&mut files, name) {
            if flags & messages::RAMFS_O_EXCL != 0 {
                return Err(messages::ERR_NOT_FOUND);
            }
            // Existing object open: owner fast-path or capability record allow.
            Self::check_access_any(&caps, caller_pd, existing, CAP_RIGHT_OPEN_EXISTING)?;
            // Reactivate if inactive
            if !existing.active {
                existing.active = true;
                existing.handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
            }
            return Ok(existing.handle);
        }

        // File doesn't exist: create if O_CREATE
        if flags & messages::RAMFS_O_CREATE != 0 {
            self.allocate(&mut files, name, caller_pd)
        } else {
            Err(messages::ERR_NOT_FOUND)
        }
    }

    fn read(&self, handle: u64, offset: u64, buf: &mut [u8], caller_pd: u32) -> Result<u64, i64> {
        let files = self.files.read();
        let caps = self.caps.read();
        let entry = Self::find_active_by_handle(&files, handle)
            .ok_or(messages::ERR_INVALID_HANDLE)?;
        Self::check_access(&caps, caller_pd, entry, CAP_RIGHT_READ)?;

        if offset as usize >= entry.data.len() {
            return Ok(0);
        }

        let start = offset as usize;
        let available = entry.data.len() - start;
        let to_read = buf.len().min(available).min(messages::RAMFS_MAX_FILE_SIZE);
        buf[..to_read].copy_from_slice(&entry.data[start..start + to_read]);
        Ok(to_read as u64)
    }

    fn write(&self, handle: u64, offset: u64, data: &[u8], caller_pd: u32) -> Result<u64, i64> {
        let mut files = self.files.write();
        let caps = self.caps.read();
        let entry = Self::find_active_by_handle_mut(&mut files, handle)
            .ok_or(messages::ERR_INVALID_HANDLE)?;
        let required = if offset as usize >= entry.data.len() {
            CAP_RIGHT_APPEND
        } else {
            CAP_RIGHT_WRITE
        };
        Self::check_access(&caps, caller_pd, entry, required)?;

        let start = offset as usize;
        let end = start + data.len();

        if end > messages::RAMFS_MAX_FILE_SIZE {
            return Err(messages::ERR_OVERFLOW);
        }

        if end > entry.data.len() {
            entry.data.resize(end, 0);
        }

        entry.data[start..end].copy_from_slice(data);
        Ok(data.len() as u64)
    }

    fn close(&self, handle: u64, caller_pd: u32) -> Result<(), i64> {
        let mut files = self.files.write();
        let caps = self.caps.read();
        let entry = Self::find_active_by_handle_mut(&mut files, handle)
            .ok_or(messages::ERR_INVALID_HANDLE)?;
        // Close requires at least read access for non-owner.
        Self::check_access(&caps, caller_pd, entry, CAP_RIGHT_READ)?;
        entry.active = false; // Release handle, keep name+data
        Ok(())
    }

    fn stat(&self, handle: u64, caller_pd: u32) -> Result<(u64, u32), i64> {
        let files = self.files.read();
        let caps = self.caps.read();
        let entry = Self::find_active_by_handle(&files, handle)
            .ok_or(messages::ERR_INVALID_HANDLE)?;
        Self::check_access(&caps, caller_pd, entry, CAP_RIGHT_READ)?;
        Ok((entry.data.len() as u64, entry.name_len as u32))
    }

    fn list_at(&self, index: usize, caller_pd: u32) -> Option<(u64, u32)> {
        let files = self.files.read();
        let caps = self.caps.read();
        let mut count = 0;
        for entry in files.iter() {
            if entry.active {
                let allow = caller_pd == 0
                    || caller_pd == entry.owner_pd
                    || Self::check_access(&caps, caller_pd, entry, CAP_RIGHT_LIST).is_ok();
                if allow {
                    if count == index {
                        return Some((entry.handle, entry.name_len as u32));
                    }
                    count += 1;
                }
            }
        }
        None
    }

    fn len(&self, caller_pd: u32) -> usize {
        let files = self.files.read();
        let caps = self.caps.read();
        if caller_pd == 0 {
            files.iter().filter(|e| e.active).count()
        } else {
            files
                .iter()
                .filter(|e| {
                    e.active
                        && (e.owner_pd == caller_pd
                            || Self::check_access(&caps, caller_pd, e, CAP_RIGHT_LIST).is_ok())
                })
                .count()
        }
    }

    /// Create a file with an explicit owner PD.
    /// `caller_pd` must be 0 (server-internal) or the same as owner_pd.
    /// The file is created with `owner_pd` as the recorded owner.
    /// Always creates (O_CREATE implicit). Fails if name already exists.
    fn create_with_owner(
        &self,
        name: &[u8],
        owner_pd: u32,
        caller_pd: u32,
    ) -> Result<u64, i64> {
        // Gate: caller must be server-internal (0) or the owner themselves.
        if caller_pd != 0 && caller_pd != owner_pd {
            return Err(messages::ERR_PERM_DENIED);
        }
        if name.len() > messages::RAMFS_MAX_NAME {
            return Err(messages::ERR_NAME_TOO_LONG);
        }
        if name.is_empty() {
            return Err(messages::ERR_NOT_FOUND);
        }

        let mut files = self.files.write();

        // Reject if name already exists.
        if Self::find_any_by_name(&files, name).is_some() {
            return Err(messages::ERR_NOT_FOUND);
        }

        // Allocate with explicit owner.
        let mut name_buf = [0u8; messages::RAMFS_MAX_NAME];
        name_buf[..name.len()].copy_from_slice(name);

        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        files.push(FileEntry {
            handle,
            object_id: self.next_object_id.fetch_add(1, Ordering::SeqCst),
            active: true,
            owner_pd,
            cap_generation: 1,
            name: name_buf,
            name_len: name.len() as u8,
            data: Vec::new(),
        });
        Ok(handle)
    }
}
