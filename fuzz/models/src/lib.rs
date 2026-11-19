// SPDX-License-Identifier: GPL-3.0-only
//! Host-only models of kernel structures. Tests live in tests/model_*.rs at the repo
//! root; this library only carries the atomics shim, the loom copies made by build.rs
//! and, under Kani, the real kernel sources plus the proof harnesses.
extern crate alloc;

/// Atomics the loom copies use: loom's under `--cfg loom`, core's otherwise.
pub mod shim {
    #[cfg(loom)]
    pub use loom::sync::atomic;
    #[cfg(not(loom))]
    pub use core::sync::atomic;

    /// The ring's payload cell. Under loom every `get()` records a mutable access,
    /// made just before the kernel code reads or writes through the pointer, so an
    /// access not ordered by the ring's atomics fails the model.
    #[cfg(loom)]
    pub mod cell {
        pub struct UnsafeCell<T>(loom::cell::UnsafeCell<T>);
        impl<T> UnsafeCell<T> {
            pub fn new(value: T) -> Self { Self(loom::cell::UnsafeCell::new(value)) }
            pub fn get(&self) -> *mut T { self.0.with_mut(|p| p) }
        }
    }
    #[cfg(not(loom))]
    pub use core::cell;

    /// A spin-wait: under loom it yields, so the thread it waits on can run.
    #[inline]
    pub fn spin_loop() {
        #[cfg(loom)]
        loom::thread::yield_now();
        #[cfg(not(loom))]
        core::hint::spin_loop();
    }
}

/// The kinds of `kernel/src/capability.rs` the capability table stores inline, with
/// the kernel's tags (tests/capability_v2.rs checks they match), plus one BOOT record
/// kind. The rest of that file needs the whole kernel, so the models use this.
pub mod capability_stub {
    #[macro_export]
    macro_rules! capability_stub {
        ($table:path) => {
            use $table as table;
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub enum CapabilityData { Endpoint(table::ObjectCap), Notification(table::ObjectCap), Reply(table::ObjectCap), Domain(u32) }
            #[derive(Clone, Copy, Debug)]
            pub struct Capability { pub id: u32, pub data: CapabilityData }
            impl CapabilityData {
                pub const REPLY_TAG: u8 = 4;
                pub fn object_tag(&self) -> Option<(u8, table::ObjectCap)> {
                    match *self {
                        CapabilityData::Endpoint(cap) => Some((2, cap)),
                        CapabilityData::Notification(cap) => Some((3, cap)),
                        CapabilityData::Reply(cap) => Some((Self::REPLY_TAG, cap)),
                        _ => None,
                    }
                }
                pub fn from_object_tag(tag: u8, cap: table::ObjectCap) -> Option<Self> {
                    match tag {
                        2 => Some(CapabilityData::Endpoint(cap)),
                        3 => Some(CapabilityData::Notification(cap)),
                        Self::REPLY_TAG => Some(CapabilityData::Reply(cap)),
                        _ => None,
                    }
                }
            }
        };
    }
}

#[cfg(loom)]
#[allow(dead_code)]
pub mod loom_scheduler_queue {
    include!(concat!(env!("OUT_DIR"), "/loom_scheduler_queue.rs"));
}

#[cfg(loom)]
#[allow(dead_code)]
pub mod loom_ipc_ring {
    include!(concat!(env!("OUT_DIR"), "/loom_ipc_ring.rs"));
}

/// IPC v2 endpoint and notification, loom copies (build.rs).
#[cfg(loom)]
#[allow(dead_code)]
pub mod loom_ipc_v2 {
    pub type TaskId = u64;
    pub mod endpoint { include!(concat!(env!("OUT_DIR"), "/loom_endpoint.rs")); }
    pub mod notification { include!(concat!(env!("OUT_DIR"), "/loom_notification.rs")); }
}

/// Authority v2 capability table, loom copy with 4 slots and 4 objects (build.rs).
#[cfg(loom)]
#[allow(dead_code)]
pub mod loom_authority {
    pub mod capability { crate::capability_stub!(super::capability_table); }
    pub mod capability_table { include!(concat!(env!("OUT_DIR"), "/loom_capability_table.rs")); }
}

/// The real capability table for host models and proofs.
#[cfg(not(loom))]
#[allow(dead_code)]
pub mod capability { crate::capability_stub!(crate::capability_table); }
#[cfg(not(loom))]
#[allow(dead_code)]
#[path = "../../../kernel/src/capability_table.rs"]
pub mod capability_table;
#[cfg(not(loom))]
#[allow(dead_code)]
#[path = "../../../kernel/src/ipc_v2"]
pub mod ipc_v2 {
    pub type TaskId = u64;
    pub mod endpoint;
    pub mod notification;
    pub mod reply;
}
#[cfg(not(loom))]
#[allow(dead_code)]
#[path = "../../../crates/sex-pdx/src/program_code.rs"]
pub mod program_code;

#[cfg(kani)]
#[path = "../../../kernel/src/ipc/reply_credit.rs"]
pub mod reply_credit;
#[cfg(kani)]
#[path = "../../../kernel/src/scheduler_queue.rs"]
pub mod scheduler_queue;
#[cfg(kani)]
#[path = "../../../kernel/src/ipc_ring.rs"]
pub mod ipc_ring;
#[cfg(kani)]
#[path = "../../../tests/model_kani.rs"]
mod model_kani;
