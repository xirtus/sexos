//! Exact transaction-bound image-copy receipts. Wrapper obtains the attested
//! lease only from Files' compiled helper pin and actual staging region.
use crate::restricted_image::Lease;
use sex_pdx::restricted_helper::Binding;
pub struct Load {
    pub ticket: u32,
    pub scope: Binding,
    pub lease: Lease,
    length: u32,
    until: u64,
    first: bool,
    serial: bool,
    serial_mode: u8,
    serial_part: u8,
    copied: bool,
    copied_ack: bool,
    installed: bool,
    pending: Option<u8>,
}
impl Load {
    pub fn new(ticket: u32, scope: Binding, lease: Lease, length: u32, now: u64) -> Option<Self> {
        if ticket == 0
            || ticket > 0xff_ffff
            || length == 0
            || length > 32 * 1024 * 1024
            || scope.ticket != 0
            || scope.child.pd != 0
            || scope.child.task != 0
            || lease.serial == 0
            || lease.files_epoch != scope.provider_epoch
            || lease.shell_epoch != scope.coordinator_epoch
            || lease.parent != scope.parent.pd
            || lease.parent_task != scope.parent.task
            || lease.cookie != scope.cookie
            || lease.nonce != core::array::from_fn(|i| scope.nonce[i / 8].to_le_bytes()[i % 8])
        {
            return None;
        }
        Some(Self {
            ticket,
            scope,
            lease,
            length,
            until: now.checked_add(5000)?,
            first: false,
            serial: false,
            serial_mode: 0,
            serial_part: 0,
            copied: false,
            copied_ack: false,
            installed: false,
            pending: Some(0),
        })
    }
    pub fn valid(&self, epoch: u64, now: u64) -> bool {
        epoch == self.lease.files_epoch && now < self.until
    }
    pub fn repeat(&mut self, request: [u64; 3]) -> bool {
        if request
            != [
                self.ticket as u64,
                self.scope.generation,
                self.scope.previous.word(),
            ]
            || self.serial
            || self.serial_mode != 0
            || self.copied
            || self.pending.is_some_and(|p| p != 0)
        {
            return false;
        }
        self.pending = Some(0);
        true
    }
    pub fn serial(&mut self, request: [u64; 3]) -> bool {
        if self.serial_mode == 2
            || !self.first
            || request != [self.ticket as u64, 22, self.lease.files_epoch]
            || self.copied
            || self.pending.is_some_and(|p| p != 1)
        {
            return false;
        }
        self.pending = Some(1);
        true
    }
    pub fn serial_tagged(&mut self, request: [u64; 3]) -> bool {
        if self.serial_mode == 1
            || !self.first
            || self.copied
            || request[0] != self.ticket as u64
            || request[2] != self.lease.files_epoch
            || request[1] & !0x1ff != 0
            || request[1] & 255 != 22
        {
            return false;
        }
        let part = ((request[1] >> 8) & 1) as u8;
        let pending = 3 + part;
        if self.pending.is_some_and(|p| p != pending) {
            return false;
        }
        if (part == 0 && self.serial_part > 1) || (part == 1 && self.serial_part == 0) {
            return false;
        }
        self.pending = Some(pending);
        true
    }
    /// Only trusted Shell may attest that its synchronous kind3 returned after
    /// copying this exact lease. Generation is full-width, never a short hash.
    pub fn copied(&mut self, request: [u64; 3], mut release: impl FnMut(Lease) -> bool) -> bool {
        if !self.serial
            || request != [self.ticket as u64, self.scope.generation, self.lease.serial]
            || self.pending.is_some_and(|p| p != 2)
        {
            return false;
        }
        if !self.copied {
            if !release(self.lease) {
                return false;
            }
            self.copied = true;
        }
        self.pending = Some(2);
        true
    }
    pub fn reply(&self) -> Option<u64> {
        Some(match self.pending? {
            0 => 0xd2 << 56 | (self.ticket as u64) << 32 | self.length as u64,
            1 => self.lease.serial,
            2 => 0xd3 << 56 | (self.ticket as u64) << 32,
            part @ (3 | 4) => {
                return sex_pdx::restricted_helper::chunk(
                    self.ticket as u64,
                    0,
                    22,
                    part - 3,
                    (self.lease.serial >> ((part - 3) as u32 * 32)) as u32,
                )
            }
            _ => return None,
        })
    }
    pub fn delivered(&mut self) {
        match self.pending.take() {
            Some(0) => self.first = true,
            Some(1) => {
                self.serial = true;
                self.serial_mode = 1;
            }
            Some(3) => {
                self.serial_mode = 2;
                self.serial_part = 1;
            }
            Some(4) => {
                self.serial_mode = 2;
                self.serial_part = 2;
                self.serial = true;
            }
            Some(2) => self.copied_ack = true,
            _ => {}
        }
    }
    pub fn consumed(&self) -> bool {
        self.installed && self.copied_ack && self.pending.is_none()
    }
    pub fn mark_installed(&mut self, b: Binding) -> bool {
        if self.installed || !self.install(b) {
            return false;
        }
        self.installed = true;
        true
    }
    pub fn install(&self, b: Binding) -> bool {
        self.copied_ack
            && self.pending.is_none()
            && b.ticket == self.ticket as u64
            && b.generation == self.scope.generation
            && b.coordinator_epoch == self.scope.coordinator_epoch
            && b.provider_epoch == self.scope.provider_epoch
            && b.previous == self.scope.previous
            && b.child.pd == self.scope.previous.pd
            && b.child.task != 0
            && b.parent == self.scope.parent
            && b.cookie == self.scope.cookie
            && b.nonce == self.scope.nonce
            && b.plugin_hash == self.scope.plugin_hash
            && b.input_hash == self.scope.input_hash
    }
}

/// Actual Files image snapshot, never a client- or Shell-supplied digest. Every
/// writer uses restricted_image's shared Writer fence; attestation then retains
/// that fence until exact COPIED. On uncertain failure it is NOT timed out open.
pub fn image(scope: Binding) -> Option<(Lease, u32)> {
    use sex_pdx::{
        memory_region::{QUERY_VIEW, READ, WRITE},
        program_store as store,
    };
    let word = crate::program_store::load_unbound(store::pack_name(b"moanplug")?);
    if word >> 56 != store::REPLY_TAG || (word >> 48) & 0xff != 1 {
        return None;
    }
    let length = word as u32;
    if length == 0 || length as usize > store::IMAGE_BYTES {
        return None;
    }
    let info = sex_pdx::memory_region_info(store::IMAGE_SLOT_FILES).ok()?;
    if info.region_id != store::IMAGE_REGION_ID
        || info.peer_pd != None
        || info.length != store::IMAGE_BYTES as u64
        || info.rights & (QUERY_VIEW | READ | WRITE) != (QUERY_VIEW | READ | WRITE)
    {
        return None;
    }
    let view = sex_pdx::memory_region_view(
        store::IMAGE_SLOT_FILES,
        0,
        info.length,
        QUERY_VIEW | READ | WRITE,
    )
    .ok()?;
    if view.region_id != info.region_id
        || view.length != info.length
        || view.virtual_address == 0
        || view.virtual_address % 4096 != 0
    {
        return None;
    }
    let candidate =
        unsafe { core::slice::from_raw_parts(view.virtual_address as *const u8, length as usize) };
    let lease = crate::restricted_image::attest_and_lease(
        Lease {
            serial: 0,
            shell_epoch: scope.coordinator_epoch,
            files_epoch: scope.provider_epoch,
            parent: scope.parent.pd,
            parent_task: scope.parent.task,
            cookie: scope.cookie,
            nonce: core::array::from_fn(|i| scope.nonce[i / 8].to_le_bytes()[i % 8]),
        },
        candidate,
    )?;
    Some((lease, length))
}
