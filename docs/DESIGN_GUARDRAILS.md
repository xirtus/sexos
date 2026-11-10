# SexOS design guardrails: staying a SASOS microkernel, not a slow Linux

Status: binding project rule, approved by Andreas on 2026-11-09.
Audit that produced it: `docs/handoff/LINUX_DRIFT_AUDIT_2026_11_09.md`.
Ledger it requires: `docs/BRIDGE_LEDGER.md`.
Host test that enforces the mechanical parts: `scripts/tests/test_kernel_design_guardrails.py`.

SexOS wants to match and beat Linux on what a machine can do. It does not want to become
Linux. We adopt ideas and permissively licensed code into SexOS's own design (one address
space, PKU organs, capabilities, PDX IPC, servers outside the kernel). These rules keep
that true while we move fast on real hardware.

## 1. The kernel keeps mechanism, servers keep policy and devices

The kernel does: address space and frames, PKU keys, capabilities and their revocation,
PDX IPC, scheduling, interrupts and their routing, PCI enumeration and device leases, IOMMU
setup, timers, and the few things only ring 0 can do when everything else is dead (the
stop screen, the crash log, a fixed-hardware reset or power off).

The kernel does not do: device drivers, filesystems, network protocols, firmware
interpreters (AML), window or desktop logic, or anything a capability-holding server could
do from ring 3. When a feature needs a little ring-0 help, add the smallest mechanism
(a lease, a capability kind, an interrupt route) and put the feature in a server.

- New kernel code is named in its PR as kernel growth, with the reason it cannot be a
  server.
- The kernel's non-blank line count has a budget (`KERNEL_LINE_BUDGET` in the host test).
  Raising it is allowed, but the PR that raises it says why in this file's table below.
- Firmware is untrusted input. Running it in ring 0 (where PKU does not apply) is a
  kernel-growth decision that needs Andreas's OK, not a default.

| Date | Budget | Reason |
|------|--------|--------|
| 2026-11-09 | 21,500 | Baseline after removing dead POSIX/Linux-shaped stubs (18,383 lines). Headroom covers the open boot-log/stop-screen (#78), VMD enumeration (#84) and lease (#79) work. ACPI AML in ring 0 (#70, #82) is the open question in the audit. |
| 2026-11-10 | 21,500 | Unchanged. AML moved out of ring 0 into `servers/sexacpi` (Andreas, 2026-11-10). The kernel keeps FADT reset/S5, the SCI route, the `Power` check and a filtered firmware window (I/O ports, MMIO and PCI config that refuse RAM, kernel-owned ports and BARs). |
| 2026-11-10 | 21,500 | Unchanged. #78 (stop screen, which must stay in the kernel) and #86 (IOMMU page tables, mechanism) took the kernel to 22,121 and turned master red. The fix removed the historical in-kernel e1000 bring-up probe from `hal/pci.rs` (3,266 lines: NIC rings, ARP/ICMP/UDP/TCP/DNS/HTTP frames, off unless built with `SEXOS_BOOT_NIC_DIAGNOSTICS=1`). That is a protocol stack in ring 0, and SexNet owns NICs. Kernel now about 18,600 lines. |
| 2026-11-10 | 21,500 | Unchanged. A 22,500 raise for #89 (scheduler latency classes, ~115 lines, landed with #96) was dropped: after the NIC probe removal the kernel is ~18,700 lines with them included. |

## 2. No Linux-shaped kernel API

- No fork, exec, pipes, signals, file descriptors, sockets, ioctl or errno in the kernel.
  A SexOS program is a run of its BOOT row holding capabilities, not a Unix process.
- POSIX, C library, Rust `std`, sockets and any other compatibility layer live in
  libraries linked into the program (`sex-libc`, `sex-std`, `sex-app`) or in a server,
  and are built on PDX calls and capabilities. They never add a syscall that exists only
  to look like Linux.
- The kernel crate never depends on a compatibility crate (`sex-libc`, `sex-std`,
  `sex-app`, `relibc`, `sexc`, `tuxedo`, ...). The host test checks this.
- A compatibility library has no more authority than the program it is linked into: a
  socket is a flow the program's own window holds, a file is a grant the program's own
  row holds. No ambient namespaces.

## 3. Linux drivers are temporary bridges; native drivers are the goal

The aim for every device is a native SexOS driver: written in Rust for SexOS, running as
its own server on a leased device, built from specs, datasheets, our own reverse
engineering (REA) and permissively licensed references.

A **bridge** is a driver that runs Linux driver code (normally GPL-2.0-only) inside a
SexOS driver server through Tuxedo or a similar adapter, to get hardware working before
the native driver exists. Bridges are allowed, and they are always temporary:

1. Every bridge has a row in `docs/BRIDGE_LEDGER.md` before it merges: what it wraps,
   the upstream source and licence, the server it runs in, and its REA replacement
   (a tracked issue or thread, the plan, and what "replaced" means for that device).
2. A bridge runs in ring 3 in its own organ, on a leased device, behind the IOMMU. It is
   never linked into the kernel and never into a GPL-3-only crate.
3. When the native driver passes the same gates on the same hardware, the bridge is
   deleted in that PR or the next one, and its ledger row moves to "Retired".
4. A bridge that has had no progress on its replacement for 90 days is raised with
   Andreas, not left to become permanent.
5. Licensing: only crates a driver links (`sex-pdx`, `sex-rt`, `sex-hwdb` and the other
   driver-facing crates) are `GPL-2.0-or-later OR MIT`, so GPL-2.0-only driver code can
   link them. Everything else stays GPL-3.0. The dual licence exists for bridges; it does
   not make Linux code a permanent part of SexOS.

Using Linux, BSD or vendor sources as **reference** (register meanings, quirks, default
values, firmware file names) while writing native code is normal and is not a bridge.
Those drivers still get a ledger row in the "native" table, so we know which Linux
behaviour we copied and must confirm on real hardware. Copying GPL-2.0-only code into a
GPL-3.0 crate is never allowed.

Firmware blobs a device needs (Wi-Fi microcode, NVIDIA GSP) are not bridges, because
they run on the device, not the CPU. They get a row in the ledger's firmware table.

## 4. Adopt ideas into SexOS's shape

When we take an idea from Linux (runtime power management, KMS, mac80211, ...), we take
the behaviour, not the structure. It is mapped onto organs, capabilities and PDX: a
subsystem that is one tree inside Linux becomes a server or a library in the organ that
owns the device. A PR that adopts a Linux design says what it changed to fit SexOS.

## 5. How this is checked

- `scripts/tests/test_kernel_design_guardrails.py` (runs with the host unit tests in CI):
  kernel dependencies, Linux-shaped kernel source files, stray backup files in the
  kernel, the kernel line budget, and that every GPL-2.0-only or Tuxedo-using component
  has a bridge ledger row.
- Reviews of PRs that touch `kernel/` read section 1. Reviews of driver PRs read
  section 3 and update the ledger.
