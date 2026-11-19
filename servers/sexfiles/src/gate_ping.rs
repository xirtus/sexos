// SPDX-License-Identifier: GPL-3.0-only
//! PKU gate proof (kernel study P6.2c): bind SexDrive's ping over the block route and
//! call it with no kernel entry. Unavailable (no PKU, no RDPID, no frame key) is fine.

fn expect(a: u64, b: u64, c: u64, d: u64) -> u64 {
    (a ^ b.rotate_left(16) ^ c.rotate_left(32) ^ d.rotate_left(48)).wrapping_add(0x5EC5_D21E)
}

fn ticks() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

pub fn prove() {
    let stub = match sex_pdx::gate::bind_route(sex_pdx::SLOT_BLOCK, sex_pdx::gate::EXPORT_PING) {
        Ok(stub) => stub,
        Err(status) => {
            let unavailable = status == sex_pdx::gate::UNAVAILABLE;
            sex_pdx::serial_record!("[gate.ping] ok=0 bound=0 unavailable={} status={:#x}", unavailable as u8, status);
            return;
        }
    };
    let (a, b, c, d) = (0x1111, 0x2222_0000, 0x3333, 0x4444);
    let ok = stub(a, b, c, d) == expect(a, b, c, d);
    let mut best = u64::MAX;
    for _ in 0..64 {
        let t = ticks();
        core::hint::black_box(stub(core::hint::black_box(1), 2, 3, 4));
        best = best.min(ticks().wrapping_sub(t));
    }
    sex_pdx::serial_record!("[gate.ping] ok={} bound=1 round_trip_ticks={}", ok as u8, best);
}
