//! Phase-6 orchestrator — reached by ERET chain from the Phase-5 EL0
//! warden (final tag), EL1h, DAIF masked.

use super::serial;

#[unsafe(no_mangle)]
pub extern "C" fn arm64_phase6_entry() -> ! {
    serial::write_str("[sex.arm.p6] entry=1\n");
    super::poe::detect();
    super::poe::tag_plumbing_status();
    serial::write_str("SEX_ARM_PHASE6_OK\n");
    // Phase 7 (POR_EL0 switching) is deferred by design: it requires
    // Phase-6 RUNTIME facts (probes P1-P6) unavailable on this host.
    // See docs/handoff/ARM64_PHASE_6_DETECT_TAG_V1.md.
    serial::write_str("[sex.arm.p7] deferred=1 reason=runtime_probes_required\n");
    loop {
        super::registers::wfe();
    }
}
