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

## First timelines (PR #472 run 1, medians, ms from QEMU start)

| | kernel entry | cores online | handed over | Display | Shell | Files | desktop |
|---|---|---|---|---|---|---|---|
| 1 core TCG | 3002 | 3084 | 3797 | 4081 | 4264 | 4810 | 5207 |
| 4 cores TCG | 3082 | 3306 | 4271 | 5071 | 5214 | 4598 | 5691 |
| 4 cores KVM | 2275 | 3150 | 6346 | 7152 | 8679 | 8195 | 10505 |

- Under KVM the kernel spends 3.2 s between the cores coming online and handing over
  to services (0.7 s under TCG). The kernel's serial writer read the line status before
  every byte: two port accesses, two exits to QEMU, per byte of its boot log. It now
  gathers a line and sends it a FIFO (16 bytes) per line-status read with `rep outsb`
  (`kernel/src/serial.rs`); services' lines already go out in one write (#471).
- With 4 cores, Files is ready before the desktop (USB/drive/ACPI no longer wait on
  core 0); the desktop itself is later than on 1 core under TCG because 4 TCG vCPUs share
  the runner's host CPUs and the kernel brings up three more cores serially.
