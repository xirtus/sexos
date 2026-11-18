use core::sync::atomic::{AtomicU32, AtomicPtr};
use core::ptr;
use alloc::collections::VecDeque;
use spin::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct InterruptCapData { pub irq: u8 }

#[derive(Debug, Clone, Copy)]
pub struct PciCapData { pub bus: u8, pub dev: u8, pub func: u8, pub vendor_id: u16, pub device_id: u16,
    /// Immutable BAR0 view assigned before capability sealing.
    pub bar0_view: Option<sex_pdx::memory_region::RegionGrant>,
    /// Explicit BOOT-only command-bit permission; a BAR lease alone has none.
    pub command: Option<sex_pdx::pci_command::Grant>,
    /// Register-block offsets inside `bar0_view` (which then holds the virtio BAR).
    pub virtio: Option<sex_pdx::virtio::Layout>,
    /// PCI class, subclass and programming interface (`class<<16|sub<<8|if`) of a
    /// leased controller whose register prefix is set by its class (sound, SATA);
    /// 0 for leases sized another way.
    pub class: u32,
    /// Offset and length of the device's registers inside `bar0_view`; (0, 0)
    /// means the whole view. Non-zero only for a BAR smaller than a page.
    pub window: (u32, u32),
}

pub struct DisplayHardwareLease {
    pub domain: u16,
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
}

impl core::fmt::Debug for DisplayHardwareLease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayHardwareLease")
            .field("bus", &self.bus)
            .field("dev", &self.dev)
            .field("vendor", &self.vendor_id)
            .finish()
    }
}

pub use crate::capability_table::{rights, ObjectCap};

#[derive(Debug, Clone, Copy)]
pub enum CapabilityData {
    /// IPC v2 rendezvous endpoint (kernel/src/ipc_v2/endpoint.rs).
    Endpoint(ObjectCap),
    /// IPC v2 notification word, OR-ed by signalers (ipc_v2/notification.rs).
    Notification(ObjectCap),
    /// One-shot reply to one call; `generation` is the caller's incarnation.
    Reply(ObjectCap),
    Interrupt(InterruptCapData), Domain(u32), Pci(PciCapData),
    Region(sex_pdx::memory_region::RegionGrant),
    FramePool(crate::paging_state::FramePoolGrant),
    PagerMapping(crate::paging_state::PagerMappingGrant),
    DemandRegion(crate::paging_state::DemandRegionGrant),
    AppLifecycle(crate::app_resource_state::LifecycleGrant),
    AppActivation(crate::app_resource_state::ActivationGrant),
    HistoryRecovery(sex_pdx::history_recovery::Grant),
    EntropySource(sex_pdx::entropy_source::Grant),
    ClockSource(sex_pdx::clock_source::Grant),
    TrustPolicy(sex_pdx::trust_source::Grant),
    /// Shutting down, restarting, flushing before either, reading power sensors.
    Power(sex_pdx::power::Grant),
    /// Drag: making virtual machines from the holder's own heap (syscall 110).
    Machine(sex_pdx::vm::Grant),
    /// Shell's right to start and stop program records. Held once, from BOOT.
    RecordPool,
    InputRing, MessageQueue,
}

#[derive(Debug, Clone, Copy)]
pub struct Capability {
    pub id: u32,
    pub data: CapabilityData,
}

impl CapabilityData {
    /// Slot tag of the one-shot reply kind (tags 0 and 1 are the table's).
    pub const REPLY_TAG: u8 = 4;

    /// The inline form of an object capability: its table tag and fields.
    /// Every other kind is a BOOT record and returns None.
    pub fn object_tag(&self) -> Option<(u8, ObjectCap)> {
        match *self {
            CapabilityData::Endpoint(cap) => Some((2, cap)),
            CapabilityData::Notification(cap) => Some((3, cap)),
            CapabilityData::Reply(cap) => Some((Self::REPLY_TAG, cap)),
            _ => None,
        }
    }

    pub fn from_object_tag(tag: u8, cap: ObjectCap) -> Option<Self> {
        match tag {
            2 => Some(CapabilityData::Endpoint(cap)),
            3 => Some(CapabilityData::Notification(cap)),
            Self::REPLY_TAG => Some(CapabilityData::Reply(cap)),
            _ => None,
        }
    }
}

pub use crate::capability_table::CapabilityTable;

use crate::ipc_ring::RingBuffer;
use crate::ipc::messages::MessageType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PkruValue {
    /// God Mode: All PKEYs accessible (Access and Write).
    GodMode = 0x0000_0000,
    /// Secure Default: All PKEYs blocked except PKEY 0 (Kernel).
    SecureDefault = 0xFFFF_FFFC,
}

impl PkruValue {
    /// Computes the standard PKRU mask for a Protection Domain.
    /// Opens PKEY 0 (Kernel) and the target pkey.
    /// Shared regions use separately mapped aliases tagged with each holder's key.
    pub fn for_domain(pkey: u8) -> u32 {
        let mut mask: u32 = 0xFFFF_FFFF;
        // Open PKEY 0 (Default/Kernel)
        mask &= !0b11;
        
        // Open target PKEY (Self) as RW
        if pkey < 16 {
            mask &= !(0b11 << (pkey * 2));
        }

        mask
    }
}

pub struct ProtectionDomain {
    pub id: u32,
    pub pku_key: u8,
    pub base_pkru_mask: u32,
    pub current_pkru_mask: AtomicU32,
    pub cap_table: *mut CapabilityTable,
    pub signal_ring: *mut RingBuffer<u8, 32>,
    pub message_ring: *mut RingBuffer<MessageType, 256>,
    pub main_task: AtomicPtr<crate::scheduler::Task>,
    pub trampoline_task: AtomicPtr<crate::scheduler::Task>,
    pub incoming_replies: Mutex<VecDeque<crate::ipc::messages::IpcReply>>,
    /// Replies this domain owes, granted only by accepted invocations.
    pub reply_credits: Mutex<crate::ipc::reply_credit::ReplyCredits>,
}

impl ProtectionDomain {
    pub fn new(id: u32, pku_key: u8) -> Self {
        let pkru_mask = PkruValue::for_domain(pku_key);
        
        let pd = Self {
            id, pku_key,
            base_pkru_mask: pkru_mask,
            current_pkru_mask: AtomicU32::new(pkru_mask),
            cap_table: CapabilityTable::new_boxed(),
            signal_ring: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(RingBuffer::new())),
            message_ring: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(RingBuffer::new())),
            main_task: AtomicPtr::new(ptr::null_mut()),
            trampoline_task: AtomicPtr::new(ptr::null_mut()),
            incoming_replies: Mutex::new(VecDeque::with_capacity(1)),
            reply_credits: Mutex::new(crate::ipc::reply_credit::ReplyCredits::new()),
        };

        // Slot 0 is an explicit syscall bridge, not an entry in this
        // one-indexed capability table. PDX_LISTEN handles the self ring.
        pd
    }

    /// A program record: its own identity and bookkeeping over one capability table
    /// and one pair of rings that every record shares. The table is empty and sealed,
    /// and holding no route to a record, nothing can ever put a message in the rings.
    pub fn record(id: u32, pku_key: u8, cap_table: *mut CapabilityTable, signal_ring: *mut RingBuffer<u8, 32>, message_ring: *mut RingBuffer<MessageType, 256>) -> Self {
        let pkru_mask = PkruValue::for_domain(pku_key);
        Self {
            id, pku_key,
            base_pkru_mask: pkru_mask,
            current_pkru_mask: AtomicU32::new(pkru_mask),
            cap_table, signal_ring, message_ring,
            main_task: AtomicPtr::new(ptr::null_mut()),
            trampoline_task: AtomicPtr::new(ptr::null_mut()),
            incoming_replies: Mutex::new(VecDeque::new()),
            reply_credits: Mutex::new(crate::ipc::reply_credit::ReplyCredits::new()),
        }
    }

    pub fn grant(&self, data: CapabilityData) -> u32 {
        unsafe { (*self.cap_table).insert(data) }
    }

    pub fn grant_capability(&self, slot: u64, data: CapabilityData) {
        if let Some(slot) = crate::capability_slot::decode_capability_slot(slot) {
            unsafe { (*self.cap_table).insert_at(slot, data) }
        }
    }

    pub fn find_capability(&self, id: u32) -> Option<Capability> {
        unsafe { (*self.cap_table).find(id) }
    }
}
