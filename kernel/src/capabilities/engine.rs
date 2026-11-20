use crate::capability::{ProtectionDomain, CapabilityData};
use crate::serial_println;

pub struct CapEngine;

impl CapEngine {
    /// Mints and grants the initial set of capabilities required for a new PD.
    pub fn grant_initial_rights(pd: &ProtectionDomain) {
        serial_println!("cap: Granting root rights to PD {}...", pd.id);

        // 1. Slot 1 (SLOT_STORAGE) stays empty for BOOT to wire to Files; the
        // retired VFS node placeholder granted nothing.
        unsafe { (*pd.cap_table).skip() };

        // 2. Control Ring Capability (Self-management)
        pd.grant(CapabilityData::Domain(pd.id));

        // 3. Signal Capability (Self-delivery)
        // IPCtax: Capability required to trigger signal routing via kernel.
        pd.grant(CapabilityData::Interrupt(crate::capability::InterruptCapData {
            irq: 0, // Mock: Signal Cap
        }));
    }

    pub fn verify_signal_rights(pd: &ProtectionDomain, cap_id: u64) -> bool {
        // RCU Lookup: Wait-Free
        if let Some(cap) = unsafe { &*pd.cap_table }.find(cap_id as u32) {
            match cap.data {
                CapabilityData::Interrupt(_) | CapabilityData::Domain(_) => return true,
                _ => (),
            }
        }
        false
    }
}
