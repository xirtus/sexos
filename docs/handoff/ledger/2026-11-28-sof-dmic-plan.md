# 2026-11-28: Built-in mics on Tiger Lake: SOF DSP plan + `sex-sof` crate skeleton

- Plan: `docs/handoff/SOF_DMIC_PLAN_2026_11_28.md` (x17 R1 built-in mics are DMICs on the cAVS 2.5 DSP, not the Realtek codec).
- New crate `crates/sex-sof` (`GPL-2.0-or-later OR MIT`, `no_std`): module stubs `regs`, `hw`, `fw`, `boot`, `ipc3`, `topology`, filled by stages 1 and 2.
- New host gate `scripts/sof_host_gate.sh` (`cargo test --locked -p sex-sof`) in the CI host job.
- No runtime change yet.
