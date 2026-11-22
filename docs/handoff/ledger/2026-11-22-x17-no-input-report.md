# x17 no-input report: autorun entries, tether listen, i8042 hardening, 2026-11-22

The x17's first start reached the desktop with no keyboard, mouse or USB keyboard input.

- **Boot entries** "SexOS report to the Mac (no typing)" (`CMDLINE=autorun`) and "SexOS safe
  start, report to the Mac" (`safe autorun`). Spindle reads `autorun` from the kernel's
  `[iommu.boot_options] cmdline=` line and runs `hw record /media/Ventoy/sexos-report.txt` and
  `tether listen` by itself.
- **`tether listen [PORT]`**: listener on flow 2, the accepted connection on flow 1; a dropped
  session goes back to listening. Host: `tether_host.py connect HOST|--scan`.
- **Kernel**: the boot log head kept is 1 MiB (was 256 KiB). i8042 init waits on the
  controller's status bits, flushes stale bytes, takes the 0xF4 ACK, logs `[kernel.ps2]`; a
  key byte whose IRQ1 never came is read by the input ring's reader (`keyboard::take_pending`,
  `[kernel.ps2.polled]`).
- `hw save`/`hw record` keep the newest 1,200 head lines and more input tags.
