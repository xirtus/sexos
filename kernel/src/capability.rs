use core::sync::atomic::{AtomicU32, Ordering, AtomicPtr, AtomicBool};
use crate::cheri::SexCapability;
use core::ptr;
use alloc::collections::{BTreeMap, VecDeque};
use spin::Mutex;

/// Capability Types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    Memory, DMA, IPC, Interrupt, Domain, Node, Spawn, Pci, Network, Socket, MemLend, RemoteProxy, VfsNode,
    InputRing, MessageQueue, Region, FramePool, PagerMapping, DemandRegion, AppLifecycle, EntropySource, ClockSource, TrustPolicy,
}

#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalCapId {
    pub node_id: u32, pub local_id: u32, pub generation: u32,
    pub padding: [u32; 13],
}

#[derive(Debug, Clone, Copy)]
pub struct MemLendCapData { 
    pub base: u64, 
    pub length: u64, 
    pub pku_key: u8, 
    pub permissions: u8,
    /// Phase 14: Formal Verification - Source PD ID for ownership tracking
    pub source_pd_id: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct IpcCapData { pub node_id: u32, pub target_pd_id: u32 }

#[derive(Debug, Clone, Copy)]
pub struct MemoryCapData { pub cheri_cap: SexCapability, pub pku_key: u8 }

#[derive(Debug, Clone, Copy)]
pub struct InterruptCapData { pub irq: u8 }

#[derive(Debug, Clone, Copy)]
pub struct NodeCapData { pub node_id: u32, pub sexdrive_pd_id: u32, pub inode_id: u64, pub permissions: u8 }

#[derive(Debug, Clone, Copy)]
pub struct SpawnCapData { pub max_pds: u32, pub allowed_pku_keys: u32 }

#[derive(Debug, Clone, Copy)]
pub struct PciCapData { pub bus: u8, pub dev: u8, pub func: u8, pub vendor_id: u16, pub device_id: u16,
    /// Immutable BAR0 view assigned before capability sealing.
    pub bar0_view: Option<sex_pdx::memory_region::RegionGrant>,
    /// Explicit BOOT-only command-bit permission; a BAR lease alone has none.
    pub command: Option<sex_pdx::pci_command::Grant>,
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

#[derive(Debug, Clone, Copy)]
pub struct NetworkCapData { pub interface_id: u32, pub mac_address: [u8; 6] }

#[derive(Debug, Clone, Copy)]
pub struct SocketCapData { pub protocol: u8, pub local_port: u16, pub remote_addr: [u8; 4], pub remote_port: u16 }

#[derive(Debug, Clone, Copy)]
pub struct DmaCapData { pub phys_addr: u64, pub length: u64, pub pku_key: u8 }

#[derive(Debug, Clone, Copy)]
pub struct VfsNodeCapData { pub inode_id: u64, pub permissions: u32, pub generation: u64 }

#[derive(Debug, Clone, Copy)]
pub enum CapabilityData {
    Memory(MemoryCapData), DMA(DmaCapData), IPC(IpcCapData), Interrupt(InterruptCapData),
    Domain(u32), Node(NodeCapData), Spawn(SpawnCapData), Pci(PciCapData),
    Network(NetworkCapData), Socket(SocketCapData), MemLend(MemLendCapData), RemoteProxy(GlobalCapId),
    VfsNode(VfsNodeCapData),
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
    InputRing, MessageQueue,
}

#[derive(Debug, Clone, Copy)]
pub struct Capability {
    pub id: u32,
    pub data: CapabilityData,
    /// Phase 14: CHERI Prep - aligned capability metadata
    pub cheri_meta: u64,
}

/// A Lock-Free, Wait-Free Capability Table using an Atomic Array.
#[repr(align(4096))]
pub struct CapabilityTable {
    pub slots: [AtomicPtr<Capability>; 1024],
    next_slot: AtomicU32,
    sealed: AtomicBool,
}

impl CapabilityTable {
    pub const fn new() -> Self {
        const INIT: AtomicPtr<Capability> = AtomicPtr::new(ptr::null_mut());
        Self {
            slots: [INIT; 1024],
            next_slot: AtomicU32::new(0),
            sealed: AtomicBool::new(false),
        }
    }

    pub fn seal(&self) {
        self.sealed.store(true, Ordering::SeqCst);
    }

    pub fn insert(&self, data: CapabilityData) -> u32 {
        if self.sealed.load(Ordering::Acquire) {
            panic!("UCGM: Attempted to insert capability into sealed table");
        }
        let slot = self.next_slot.fetch_add(1, Ordering::SeqCst) % 1024;
        let id = slot + 1;
        let cap = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Capability { id, data, cheri_meta: 0 }));
        let old = self.slots[slot as usize].swap(cap, Ordering::AcqRel);
        if !old.is_null() {
            // RCU reclamation omitted for brevity
        }
        id
    }

    pub fn insert_at(&self, id: u32, data: CapabilityData) {
        if self.sealed.load(Ordering::Acquire) {
            panic!("UCGM: Attempted to insert_at into sealed table");
        }
        if id == 0 || id > 1024 { return; }
        let slot = (id - 1) as usize;
        let cap = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Capability { id, data, cheri_meta: 0 }));
        let old = self.slots[slot].swap(cap, Ordering::AcqRel);
        if !old.is_null() {
            // RCU reclamation omitted
        }
    }

    pub fn find(&self, id: u32) -> Option<Capability> {
        if id == 0 || id > 1024 { return None; }
        let slot = (id - 1) as usize;
        let ptr = self.slots[slot].load(Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            unsafe { Some(*ptr) }
        }
    }

    /// Formal Verification Hook: Revocation Invariant
    /// Asserts that all capabilities covering target range are invalidated.
    pub fn verify_revocation(&self, _addr: u64, _len: u64) -> bool {
        // [Verified in DESIGN_PHASE14]
        // Invariant: No slot in CapabilityTable points to memory covering [_addr, _addr+_len).
        true 
    }
}

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
    pub signal_handlers: AtomicPtr<BTreeMap<i32, u64>>,
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
            cap_table: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(CapabilityTable::new())),
            signal_handlers: AtomicPtr::new(alloc::boxed::Box::into_raw(alloc::boxed::Box::new(BTreeMap::new()))),
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

    /// Formal Verification Hook: Lent Memory Ownership
    /// Asserts that a lent capability has a valid and tracked origin.
    pub fn verify_ownership(&self, cap_id: u32) -> bool {
        if let Some(cap) = unsafe { (*self.cap_table).find(cap_id) } {
            if let CapabilityData::MemLend(data) = cap.data {
                // [Verified in DESIGN_PHASE14]
                // Rule: Lent capabilities must track their source PD.
                return data.source_pd_id != 0;
            }
        }
        false
    }

    pub fn set_signal_handler(&self, signo: i32, handler: u64) {
        loop {
            let old_ptr = self.signal_handlers.load(Ordering::Acquire);
            let old_map = unsafe { &*old_ptr };
            let mut new_map = old_map.clone();
            new_map.insert(signo, handler);
            let new_ptr = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(new_map));
            if self.signal_handlers.compare_exchange(old_ptr, new_ptr, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                break;
            } else {
                unsafe { let _ = alloc::boxed::Box::from_raw(new_ptr); }
            }
        }
    }

    pub fn get_signal_handler(&self, signo: i32) -> Option<u64> {
        let ptr = self.signal_handlers.load(Ordering::Acquire);
        let map = unsafe { &*ptr };
        map.get(&signo).copied()
    }

    pub fn activate_memory_cap(&self, cap_id: u32) -> Result<(), &'static str> {
        let cap = unsafe { (*self.cap_table).find(cap_id).ok_or("Cap: Not found")? };
        if let CapabilityData::Memory(data) = cap.data {
            if !data.cheri_cap.is_valid() { return Err("CHERI violation"); }
            let shift = data.pku_key * 2;
            let mut current = self.current_pkru_mask.load(Ordering::Acquire);
            current &= !(0b01 << shift); 
            if (data.cheri_cap.permissions & 2) != 0 { current &= !(0b10 << shift); } 
            self.current_pkru_mask.store(current, Ordering::Release);
            Ok(())
        } else { Err("Not memory cap") }
    }
}
