# 2026-11-19: Devices come up in parallel at boot

- Baseline (CI, 1 core TCG, medians from QEMU start, master 3bfa9b1f): kernel entry
  2.53 s, Shell 3.56 s, SexDrive storage 4.00 s, SexDrive serving 4.03 s, Files 4.05 s,
  desktop with its terminal 4.26 s, boot program 5.24 s.
- Every boot service already starts at once (the kernel builds all boot domains, then
  admits them together), but on a machine with spare cores only Network, Files, Storage
  and the disk driver left core 0. USB (port resets and enumeration), Bell (codecs),
  ACPI (AML `_INI`, a 20 ms TSC calibration spin) and Cufflinks took turns with Display
  and Shell on core 0. They now start on cores 2 and up (`smp::home_core`, `MOVED`), so
  on 3+ cores device probing runs beside the desktop instead of in its time slices.
  On 1 or 2 cores placement is unchanged.
- SexUSB resets every connected USB 2 root port at once and then addresses each device
  in turn (`Xhci::start_reset` / `finish_reset`). Root ports are separate links, so two
  devices at their default address never share a bus. Hub ports still reset one at a
  time (one bus), but take one debounce per pass instead of one per device.
- `scripts/boot_time_gate.py` now times every service's ready marker (Display, Shell,
  Input, ACPI, Bell, USB ready and enumerated, SexDrive, Files), prints a
  `[boot.timeline]` line per boot, reads only new serial text each pass, and CI runs it
  at 1 core, 4 cores (TCG) and 4 cores under KVM.
- Not done here (other threads own them): per-core NVMe/NIC queues, driver restart,
  device power, hot-plug. Left alone: ACPI's calibration spin, because the self-wait
  source contract pins sexacpi's waits (it no longer runs on core 0 on 3+ cores anyway).
