use crate::capability::CapabilityData;
use core::sync::atomic::Ordering;

pub mod messages;
pub mod pagefault;
pub mod router;
pub mod state;
pub mod sender;
mod domain_registry;
pub use domain_registry::DomainRegistry;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BootPhase {
    Init = 0,
    RegistryBuild = 1,
    RegistryFrozen = 2,
    SchedulerArmed = 3,
    SchedulerRunning = 4,
    ScanoutEnabled = 5,
}

pub struct BootController {
    phase: BootPhase,
}

impl BootController {
    pub const fn new() -> Self {
        Self { phase: BootPhase::Init }
    }

    #[inline(always)]
    pub fn phase(&self) -> BootPhase {
        self.phase
    }

    #[inline(always)]
    pub fn advance(&mut self, next: BootPhase) {
        use BootPhase::*;

        let valid = match (self.phase, next) {
            (Init, RegistryBuild)
            | (RegistryBuild, RegistryFrozen)
            | (RegistryFrozen, SchedulerArmed)
            | (SchedulerArmed, SchedulerRunning)
            | (SchedulerRunning, ScanoutEnabled) => true,
            _ => false,
        };

        assert!(valid, "BOOT_PHASE_VIOLATION");
        self.phase = next;
        BOOT_PHASE.store(next as u8, Ordering::SeqCst);
    }

    #[inline(always)]
    pub fn require(&self, min: BootPhase) {
        assert!(self.phase as u8 >= min as u8, "BOOT_PHASE_GATED");
    }
}

pub static mut BOOT_CONTROLLER: BootController = BootController::new();

pub static BOOT_PHASE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

pub fn get_phase() -> BootPhase {
    unsafe { BOOT_CONTROLLER.phase() }
}

pub const MAX_DOMAINS: usize = 1024;
pub const LOCAL_NODE_ID: u32 = 1;

pub static DOMAIN_REGISTRY: DomainRegistry = DomainRegistry::new();

pub enum GraphEdge {
    AsyncEnqueue {
        ring: *mut crate::ipc_ring::RingBuffer<crate::ipc::messages::MessageType, 256>,
        task: *mut crate::scheduler::Task,
    },
    AsyncDequeue {
        ring: *mut crate::ipc_ring::RingBuffer<crate::ipc::messages::MessageType, 256>,
        source: DequeueSource
    },
}

pub enum DequeueSource {
    MessageRing,
    InputRing,
}

pub fn resolve_edge(cap_data: CapabilityData, caller_pd_id: u32, is_listen: bool) -> Result<GraphEdge, u64> {
    match cap_data {
        // An IPC capability names a target only. It never selects code for
        // the kernel to execute; invocation is a typed Domain enqueue.
        CapabilityData::IPC(_) => Err(sex_pdx::ERR_CAP_INVALID),
        CapabilityData::Domain(id) => {
            if !crate::app_resources::accepts_ipc(id) {return Err(sex_pdx::ERR_SERVICE_NOT_READY);}
            if is_listen { return Err(sex_pdx::ERR_CAP_INVALID); }
            let pd = DOMAIN_REGISTRY.get(id).ok_or(sex_pdx::ERR_CAP_INVALID)?;
            Ok(GraphEdge::AsyncEnqueue {
                ring: pd.message_ring,
                task: pd.main_task.load(Ordering::Acquire),
            })
        },
        CapabilityData::MessageQueue => {
            if !is_listen { return Err(sex_pdx::ERR_CAP_INVALID); }
            let pd = DOMAIN_REGISTRY.get(caller_pd_id).ok_or(sex_pdx::ERR_CAP_INVALID)?;
            Ok(GraphEdge::AsyncDequeue { ring: pd.message_ring, source: DequeueSource::MessageRing })
        },
        CapabilityData::InputRing => {
            if !is_listen { return Err(sex_pdx::ERR_CAP_INVALID); }
            Ok(GraphEdge::AsyncDequeue {
                ring: &crate::interrupts::INPUT_RING as *const _ as *mut _,
                source: DequeueSource::InputRing
            })
        },
        _ => Err(sex_pdx::ERR_CAP_INVALID),
    }
}

pub fn traverse_edge(edge: GraphEdge, opcode: u64, arg0: u64, arg1: u64, arg2: u64, caller_pd: u32, caller_task: u32) -> Result<u64, u64> {
    match edge {
        GraphEdge::AsyncEnqueue { ring, task } => {
            if caller_task == 0 { return Err(sex_pdx::ERR_CAP_INVALID); }
            let msg = crate::ipc::messages::MessageType::IpcCall {
                func_id: opcode, arg0, arg1, arg2, caller_pd, caller_task,
            };
            unsafe { (*ring).enqueue(msg).map_err(|_| sex_pdx::ERR_SERVICE_NOT_READY)?; }
            // Publish the event only after its payload is visible. A full ring
            // grants no wake; an accepted invocation retains its receiver's
            // pending event even across park/save/stack-release boundaries.
            crate::scheduler::unpark_thread(task);
            Ok(0)
        },
        GraphEdge::AsyncDequeue { ring: _, source: _ } => {
            Err(sex_pdx::ERR_CAP_INVALID)
        }
    }
}

pub fn safe_pdx_call(cap_id: u32, opcode: u64, arg0: u64, arg1: u64, arg2: u64) -> Result<u64, u64> {
    let current_pd = crate::core_local::CoreLocal::get().current_pd_ref();
    let caller_pd_id = current_pd.id;
    let cap = unsafe { (*current_pd.cap_table).find(cap_id) }.ok_or(sex_pdx::ERR_CAP_INVALID)?;
    let edge = resolve_edge(cap.data, caller_pd_id, false)?;
    traverse_edge(edge, opcode, arg0, arg1, arg2, caller_pd_id, sender::current_task(caller_pd_id).unwrap_or(0))
}

pub fn resolve_phys_pdx(cap_id: u32) -> u64 {
    let current_pd = crate::core_local::CoreLocal::get().current_pd_ref();
    if let Some(cap) = unsafe { (*current_pd.cap_table).find(cap_id) } {
        match cap.data {
            CapabilityData::MemLend(data) => return data.base,
            CapabilityData::DMA(data) => return data.phys_addr,
            _ => (),
        }
    }
    0
}

pub fn pdx_spawn_pd(elf_addr: u64, elf_len: u64) -> Result<u32, u64> {
    crate::pd::create::spawn_from_elf(elf_addr, elf_len)
        .map_err(|_| sex_pdx::ERR_CAP_INVALID)
}

pub fn route_signal(target_pd_id: u32, signal: u8) -> Result<(), &'static str> {
    if !crate::app_resources::accepts_ipc(target_pd_id) {return Err("IPC: retired target");}
    let target_pd = DOMAIN_REGISTRY.get(target_pd_id)
        .ok_or("IPC: Target not found")?;

    unsafe { (*target_pd.signal_ring).enqueue(signal) }
}

pub fn dequeue_local() -> Option<crate::ipc::messages::Message> {
    let core_local = crate::core_local::CoreLocal::get();
    core_local.pdx_ring.dequeue()
}
