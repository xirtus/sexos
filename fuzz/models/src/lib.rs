// SPDX-License-Identifier: GPL-3.0-only
//! Host-only models of kernel structures. Tests live in tests/model_*.rs at the repo
//! root; this library only carries the atomics shim, the loom copies made by build.rs
//! and, under Kani, the real kernel sources plus the proof harnesses.

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
