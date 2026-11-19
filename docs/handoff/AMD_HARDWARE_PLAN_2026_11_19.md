# AMD hardware plan (2026-11-19)

Thread "Support AMD laptops and GPUs". Goal: SexOS runs well on Ryzen laptops and desktops
and drives Radeon displays natively, under docs/DESIGN_GUARDRAILS.md (kernel = mechanism,
drivers in ring 3, no Linux code running). NVIDIA and Intel/general devices have their own
threads.

## Where AMD stood

| Area | State before this thread | Where |
| --- | --- | --- |
| CPU, PKU | Vendor-neutral (PKU on Zen 3 and newer) | `kernel/src/pku.rs` |
| Spectre | AutoIBRS turned on when CPUID 8000_0021 says so | `kernel/src/hal/x86_64.rs` |
| IOMMU | AMD-Vi: IVRS parse, device table, command ring, event log, invalidation | `kernel/src/iommu.rs`, `crates/sex-iommu` |
| Frequency | MSR CPPC (Zen 3+): enable, min/max, EPP from the power profile | `kernel/src/hal/acpi.rs` |
| Virtual machines | Intel VT-x only; AMD answered UNAVAILABLE | `kernel/src/vmx.rs` |
| Display | Nothing: Radeon ran only on the firmware framebuffer | - |

## Slices

| Slice | Delivers | Gate | State |
| --- | --- | --- | --- |
| D5 | SVM backend for SYS_VM: VMCB, nested paging, VMRUN with GIF held, exits translated to the VMX-shaped record | `drag_host_gate.sh`; `drag_gate.py --accel tcg --require-guest` (QEMU TCG emulates SVM) | PR #440 |
| A1 | `crates/sex-gpu-amd`: chip table, DCN layouts, firmware scanout readout (OTG, HUBP, VM aperture), DCN AUX engine (DPCD, EDID), vsync flips | `kms_host_gate.sh` (simulated registers) | this PR |
| A2 | Boot receipt: lease the Radeon's BAR 5 to sexdisplay read-only (like Intel's L2 receipt), print `[display.amd.*]` (chip, OTGs lit, timing, surface, aperture, AUX DPCD/EDID) | QEMU cannot emulate a Radeon; receipt from a Ryzen laptop | next |
| A3 | Takeover: allocate scanout buffers in the carve-out (APU) or VRAM (dGPU) through the aperture, flip at vblank, tear-free in sexdisplay | receipt | |
| A4 | Pads and connectors: DC_GPIO AUX/DDC mux, DIG/AUX/HPD per connector from the VBIOS object table, HDMI DDC through the DC_I2C engine, hotplug | host + receipt | |
| A5 | Pixel clock and modeset: DCCG DTO readout (exact clock without EDID), then full modeset (DIG, PHY link training) | host + receipt | |
| A6 | Backlight (eDP AUX backlight and the ABM/PWM path), PSR | receipt | |
| P1 | CPPC on Zen 2 (shared-memory `_CPC` from ACPI): belongs in `sexacpi` (AML), not the kernel | `laptop_power_gate.py` style SSDT | |
| P2 | Preferred cores: CPPC highest-perf ranking into the scheduler's core order (Zen 4/5 laptops boost two cores) | host | |
| P3 | Laptop vendor bits on AMD (ASUS/Lenovo WMI, AMD PMF platform profiles): build on the x17 thread's `sex-laptop` WMI module and its CTL_PROFILE forwarding | SSDT gate | |
| P4 | IP discovery: read the discovery table so DCN base and versions are known on any new chip instead of from the device table | receipt | |

## Notes

- Register facts come from AMD's MIT-licensed headers; the DC code was read for behaviour
  (timing encoding, flip order, AUX sequence), not copied. Ledger: docs/BRIDGE_LEDGER.md
  table 2.
- OTG/HUBP/AUX offsets are identical from DCN 2.0 to 3.2; DCN 3.1 moved the VM aperture
  registers and DCN 3.5 moved part of the OTG block (`Layout`). Field masks are identical
  across 2.1, 3.1.4 and 3.5.
- Chips amdgpu finds by IP discovery (Phoenix, Hawk Point, Raphael, Mendocino, Strix, Navi 3x)
  are `confirmed: false` until a receipt shows the expected registers.
- The best AMD test machine would be any Ryzen 6000/7040/8040 laptop: its eDP panel is lit by
  GOP on OTG0 and AUX0, which is exactly what A2 reads.
