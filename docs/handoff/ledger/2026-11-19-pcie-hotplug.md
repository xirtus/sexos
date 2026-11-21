# PCIe hot-plug, stage H1 (2026-11-19, hot-plug docks, monitors and eGPUs)

- **What changed:** devices plugged into a PCIe hot-plug port after boot are now
  seen and given addresses. That is the first half of Thunderbolt docks and eGPU
  enclosures (both appear as a PCIe tree below a Thunderbolt root port) and of
  hot-swap NVMe. BOOT records every root or downstream port whose slot says
  "hot-plug capable"; core 0 looks at each slot every 20 ms (three config reads).
- **Rules:** `crates/sex-hotplug` (pure, no heap): `slot` settles a new card for
  200 ms, powers the slot, waits 100 ms of link-up, and treats presence or link loss
  as surprise removal; the attention button gives 5 s to cancel before the slot is
  quieted and powered off. `plan` numbers nothing itself but places every BAR and
  bridge window below the port (nested bridges for docks and eGPUs, prefetchable
  windows, 1 MiB bridge granularity). `pcie` decodes the slot registers.
- **Kernel:** `kernel/src/hotplug.rs` (about 230 lines, mechanism only) numbers the
  buses below the port depth-first, sizes BARs, writes the plan, turns on memory and
  bus mastering on bridges only, and logs `[hw.device]` census lines. Endpoints stay
  off until a driver lease turns them on (stage H2). On removal everything below the
  port has decode and bus mastering turned off.
- **Boot log:** `[hotplug.port]` per port (kind, slot, button, power, buses, windows),
  then `[hotplug.power]`, `[hotplug.bar]`, `[hotplug.arrived]`, `[hotplug.button]`,
  `[hotplug.left]`, `[hotplug.failed]`, `[hotplug.bar.fail]`.
- **Gates:** `scripts/hotplug_host_gate.sh` (CI host job);
  `scripts/pcie_hotplug_gate.py` (QEMU q35 root port with native hot-plug: NVMe
  plugged, removed with the button, plugged again) in the CI devices job.
- **Not yet:** runtime driver leases (H2), monitor hot-plug and re-layout (H3), eGPU
  binding (H4), USB-C dock polish (H5); see `docs/handoff/HOTPLUG_PLAN_2026_11_19.md`.
