// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Which core uses which device queue and interrupt vector.
//!
//! Ideal: one queue and one vector per core. Real devices grant fewer (an NVMe drive
//! may offer 8 queue pairs, a NIC 4 RX/TX pairs, MSI-X tables 2 vectors), so cores
//! share: core `c` uses queue `c mod queues`, and a queue's worker runs on the first
//! core that uses it. Vectors are shared the same way: queue `q` raises vector
//! `(q + vector_base) mod vectors`. NVMe's admin queue keeps vector 0 to itself when
//! there is more than one vector, as Linux does, so I/O completions never wait
//! behind admin work.

/// At most this many lanes; enough for any laptop and most desktops.
pub const MAX_LANES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub cores: u32,
    pub queues: u32,
    pub vectors: u32,
    /// First vector I/O queues use (1 when vector 0 is kept for admin work).
    pub vector_base: u32,
}

impl Plan {
    /// `cores` online, `queues` the device granted, `vectors` the kernel programmed.
    /// With `admin_vector` the device's vector 0 is reserved (NVMe).
    pub fn new(cores: u32, queues: u32, vectors: u32, admin_vector: bool) -> Plan {
        let cores = cores.clamp(1, MAX_LANES as u32);
        let vectors = vectors.max(1);
        let vector_base = if admin_vector && vectors > 1 { 1 } else { 0 };
        let queues = queues.clamp(1, cores);
        Plan { cores, queues, vectors, vector_base }
    }

    /// The queue (0-based; an NVMe queue id is this + 1) core `core` submits on.
    pub fn queue_of(&self, core: u32) -> u32 { core % self.cores % self.queues }

    /// The core whose worker owns queue `q`: the first core mapped to it.
    pub fn home_of(&self, q: u32) -> u32 { q % self.queues }

    /// The vector index queue `q` raises.
    pub fn vector_of(&self, q: u32) -> u32 {
        let io_vectors = self.vectors - self.vector_base;
        self.vector_base + q % io_vectors
    }

    /// Whether queue `q`'s interrupt is its own (no other queue raises its vector).
    pub fn vector_private(&self, q: u32) -> bool {
        let io_vectors = self.vectors - self.vector_base;
        self.queues <= io_vectors || (q % io_vectors) + io_vectors >= self.queues
    }
}
