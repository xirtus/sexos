# x17: the desktop is usable seconds after it appears (2026-11-24)

Andreas, first x17 R1 boots: about a minute from the desktop appearing until the mouse
worked, Spindle about 30 s to open, and Quil, Linen and Kaleidoscope opened on every boot.

## Root cause: the 1 ms tick was ~417 ms
The kernel calibrated the local APIC timer against PIT channel 2 only. Tiger Lake and
newer Intel laptops gate the PIT's clock off (Linux skips the PIT there too), so the
calibration timed out and the timer fell back to a fixed 1,000,000 counts. The x17's
APIC timer counts the 38.4 MHz crystal /16 = 2.4 MHz, so one "millisecond" tick lasted
~417 ms: every sleep, poll interval, timeout and time slice ran ~400 times slow. The
2026-11-23 x17 report says `uptime_ms=769` minutes after boot.

`apic::init_timer` now takes the rate from, in order: the core crystal clock CPUID
reports (leaves 0x15/0x16, Intel bare metal; what Linux's `native_calibrate_tsc` uses
for `lapic_timer_period`), the PIT (VMs, older PCs; the gate bit on 0x61 is now set and
the wait is TSC-bounded), then the TSC at its CPUID rate. `[timer.source] source=...
per_ms=...` says which (in Spindle `hw`). The stop screen's 10 ms wait uses the CPUID
TSC rate too.

## Boot windows
`servers/silk-shell/src/boot_windows.rs`: on a real machine (CPUID hypervisor bit
clear, first Shell run) Linen, Quil and Kaleidoscope are minimized 1.5 s after each
registers, and Spindle gets the focus: `[shell.boot.windows] tidy=1 minimized=0b111`.
In a VM the reference layout the desktop gates use stays (`tidy=0
reason=virtual_machine`).

## Log volume
`point_in_surface` no longer logs `[shell.surface.dead.skip]` (dozens of lines per
pointer move; 731 in one x17 report). Per-key Spindle lines: PR #523. USB pointer
lines were already budgeted (cb3803e9).

Gate: `scripts/boot_windows_gate.py` (CI, after boot time): one boot with
`-cpu ...,hypervisor=off`, one ordinary.
