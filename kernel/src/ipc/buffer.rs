use alloc::collections::BTreeMap;
use spin::Mutex;
use crate::memory::manager::GLOBAL_VAS;
use crate::memory::va_allocator::allocate_va;
use crate::pku;
use x86_64::VirtAddr;
use x86_64::structures::paging::PageTableFlags;

pub struct IpcBufferMetadata {
    pub addr: u64,
    pub size: usize,
    pub producer_id: u8,
    pub consumer_id: u8,
}

pub struct IpcBufferManager {
    pub buffers: Mutex<BTreeMap<u64, IpcBufferMetadata>>,
}

pub static IPC_BUFFER_MANAGER: IpcBufferManager = IpcBufferManager {
    buffers: Mutex::new(BTreeMap::new()),
};

impl IpcBufferManager {
    pub fn allocate_shared_buffer(&self, size: usize, producer_id: u8, consumer_id: u8) -> Result<u64, &'static str> {
        let va = allocate_va(size).ok_or("IpcBufferManager: VA allocation failed")?;
        
        {
            let mut vas = GLOBAL_VAS.lock();
            let vas_ref = vas.as_mut().ok_or("IpcBufferManager: VAS not initialized")?;
            
            let flags = PageTableFlags::PRESENT 
                      | PageTableFlags::WRITABLE 
                      | PageTableFlags::USER_ACCESSIBLE
                      | PageTableFlags::NO_EXECUTE;
            
            // Validation: pkey MUST be SHARED (14)
            let pkey = pku::SHARED;
            assert_eq!(pkey, 14, "IpcBufferManager: Invariant violated - SHARED pkey must be 14");
            
            vas_ref.map_pku_range(VirtAddr::new(va), size as u64, flags, pkey)?;
        }

        let metadata = IpcBufferMetadata {
            addr: va,
            size,
            producer_id,
            consumer_id,
        };

        self.buffers.lock().insert(va, metadata);

        Ok(va)
    }

    pub fn get_metadata(&self, va: u64) -> Option<IpcBufferMetadata> {
        self.buffers.lock().get(&va).map(|m| IpcBufferMetadata {
            addr: m.addr,
            size: m.size,
            producer_id: m.producer_id,
            consumer_id: m.consumer_id,
        })
    }
}
