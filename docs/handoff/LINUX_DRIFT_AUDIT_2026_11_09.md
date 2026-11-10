# Linux drift audit (2026-11-09)

Question from Andreas: are we changing SexOS so much that it stops being the microkernel
SASOS we wanted, and turning it into Linux by accident?

Short answer: no, with one real exception. The compatibility layers are clean libraries,
the new drivers are native Rust servers on leased devices, and no Linux code is in the tree.
The one place that leans Linux-ward is the open ACPI work (#70, and #82 on top of it),
which runs the firmware's AML interpreter in ring 0. Dead POSIX-shaped stubs in the
kernel (fork, exec, pipes, "Linux driver translation") were found and removed.

Scope: master at `5594b33`, open PRs #68 to #84 (#73 and #81 are docs/CI only), the
sex-manual (`docs/handoff_html/pages/*manual*.html`, `docs/handoff_html/legacymd/archive/manual_*.md`)
and `ARCHITECTURE.md`. The rules that came out of it are in `docs/DESIGN_GUARDRAILS.md`,
and the ledger in `docs/BRIDGE_LEDGER.md`.

## What the manual asks for

- One address space, isolation by PKU keys (now organs), not page tables.
- Lock-free PDX IPC and capabilities; no ambient authority.
- "No dynamic kernel modules: all drivers compile into server ELFs."
- "NO Linux-style process model" (`ARCHITECTURE.md` section 3).
- "Core: pure Rust no_std microkernel (< 7 kLOC)" (user manual). The kernel is ~18.4k
  non-blank lines today, so this target is already far off; see recommendation 3.
- Linux compatibility (Lin-Sex, sexc, Tuxedo) as translation layers outside the kernel.

## Findings

### Clean (no action)

| Area | PR | Why it is fine |
|------|----|----------------|
| POSIX for C programs (`sex-libc` `posix.rs`), Rust `std` (`sex-std`) | #68 | Library inside the program. No kernel change, no fork/exec/pids ("a program is a run of its row, not a Unix process"). Every call becomes a PDX call with the program's own grants. |
| TCP/UDP sockets, IPv6, TLS | #69 | `sex_app::net` sockets are flows on the program's window to SexNet; TLS is a library. No kernel change, no fd table in the kernel. |
| Wired NICs, HD Audio, AHCI | #79 | Native Rust in `sexnet`, `sexbell`, `sexdrive` on leased BARs. Kernel only gains lease plumbing (`device_lease.rs`, BAR bounds), which is kernel mechanism. Linux used for a few defaults only (ledger table 2). |
| Intel Wi-Fi | #83 | One Rust crate in the network organ instead of iwlwifi + mac80211 + wpa_supplicant. OpenBSD `iwx` (ISC) as reference. Zero kernel lines. |
| IMSM RAID | #84 | `sex-imsm` and `sexdrive` `raid.rs` in userspace; layout facts from mdadm, no mdadm code. |
| Tiling WM, installer/A-B, display HiDPI, USB, Files volumes | #71, #72, #74, #75, #76, #77 | Servers and libraries; kernel changes in #74 and #77 are memory/tenancy and boot display state, i.e. mechanism. |
| Hardware census, x17 R1 plan | #80 | `sex-hwdb` is a library; kernel change is a census print in `devmgr`. |

### Fixed in this PR (small violations on master)

1. **POSIX and Linux-driver stubs in the kernel.** `kernel/src/syscalls/` had `fork.rs`
   (`sys_fork`, `sys_execve`), `pipe.rs` (`sys_pipe`), `translator.rs`
   (`sys_translate_driver`: "LOAD_LINUX_DRIVER", "Phase 15 Linux Driver Translation
   Layer + DDE-style Reuse"), `fs.rs` (`sys_read`/`sys_write` on a "VFS capability") and
   `spawn.rs`. None were reachable from the syscall dispatcher; only the never-built
   phase tests and `benchmark.rs` (itself unused) called them. They were the Linux
   process model and Linux driver loading, in the kernel, in shape if not in effect.
   Removed, with `benchmark.rs` and the nine `kernel/tests/phase*.rs` files that called
   them (those tests use `std` and have not compiled for a long time).
2. **Uncompiled kernel directories** that read as "desktop, input, network and crypto in
   the kernel": `kernel/src/{desktop,input,network,gemini,crypto,surface,xipc}` (139+
   lines, no `mod` declarations). Removed. `spec/architecture.sexspec` cited
   `desktop/replay.rs` for the open DETERMINISTIC_REPLAY obligation; the citation now says
   replay belongs in a server.
3. **Backup and editor leftovers in `kernel/src`** (`*.bak.*`, `init.rs.patch`, files named
   like `main.rs:58:56`). Removed.
4. **No written rule.** Added `docs/DESIGN_GUARDRAILS.md`, `docs/BRIDGE_LEDGER.md` and the
   host test `scripts/tests/test_kernel_design_guardrails.py`, which runs with the
   existing host unit tests in CI.

The kernel still builds (`cargo build -p sex-kernel --target x86_64-sex.json --release`).

### Needs a decision (not changed here)

**A. Firmware AML interpreter in ring 0 (#70 ACPI power, #82 dGPU power off).**
_Decided 2026-11-10 (Andreas): move it. Done in the sexacpi PR: AML runs in the ring-3 `sexacpi` server; see "Outcome of A" below._
#70 adds ~780 lines to `kernel/src/hal/acpi.rs` that run the `acpi`/`aml` crates'
interpreter in the kernel for `\_S5`, `_PTS`, batteries, lids, thermal zones, `_CST` and
I2C HID. #82 adds `kernel/src/hal/dgpu.rs` (~350 lines) that runs `_PS3`/`_PRx`/`_OFF` to
cut the NVIDIA GPU's power. This is the Linux design (ACPICA in the kernel). The PR knows
the risk: AML is "untrusted input" so it runs on its own stack with a work budget and a
panic quarantine. But ring 0 ignores PKU, so a firmware bug the interpreter does not catch
can write anywhere in the single address space. Fuchsia, Genode and seL4 systems run AML
in a userspace driver for this reason.

Recommendation: keep the fixed-hardware parts in the kernel (FADT reset and S5 using the
table bytes, the SCI route, the `Power` capability check) and move the AML interpreter and
everything that needs it (batteries, lids, thermal, `_CST` hints, I2C HID enumeration, dGPU
`_OFF`) into a ring-3 **`sexacpi`** server in the Core organ. It gets leases on exactly the
I/O ports, ECAM ranges and memory regions the DSDT/SSDT `OperationRegion`s name, and gives
other servers typed capabilities (battery state, lid events, "power off device X").
Shutdown never depends on it. Cost: about one thread-week; the PRs' code moves largely as
is, with the `Hw` handler backed by leases instead of raw physical access. It can land
after #70 merges, as a follow-up, if bare-metal testing next week needs #70 now.

**B. VMD bus enumeration in the kernel (#84, `kernel/src/vmd.rs`, ~100 lines).** The
kernel owns PCI enumeration and leases, so finding devices behind VMD there is defensible
and small. The cleaner end state is a VMD bus server that holds the VMD lease and
sub-leases the child devices, which needs sub-leasing in the lease model. Fine to merge;
revisit when sub-leases exist (needed for Thunderbolt and USB4 anyway).

**C. Boot log and stop screen (#78, ~940 kernel lines incl. font).** The stop screen must
be in the kernel (it runs when servers are dead). Drawing every boot log line on the
framebuffer is a kernel console in the Linux fbcon sense, but it stops once Display owns
the screen and has no input or VT. Fine to merge as is; keep it output-only.

## Recommendations

1. ~~Decide A (ring-3 `sexacpi` server).~~ Done 2026-11-10, see "Outcome of A".
2. Make the bridge rule stick: when the first Tuxedo bridge appears (none exist today;
   `servers/tuxedo` is a 7-line placeholder), it lands with a ledger row and a `REA:`
   tracker. The host test enforces the row.
3. Replace the manual's "< 7 kLOC" with a real budget. The host test starts at 21,500
   non-blank lines (18,383 today) and every raise is recorded with a reason. A follow-up
   could audit the kernel for further dead modules (`sunni`, `ucgm`, `amdahl`, `cheri`,
   `hw`, `vga` have few or no callers) and bring the budget down.
4. Done in this PR: the user manual (`docs/handoff_html/pages/manual.html`) promised
   "Lin-Sex: run unmodified Linux binaries, the kernel handles the rest". It now says
   POSIX is a library inside the program and Linux drivers are temporary Tuxedo bridges.

## Outcome of A (2026-11-10)

Andreas chose to merge #70 as is for the x17 R1 test and then move AML out. The follow-up:

- `servers/sexacpi` (pd 50, Senses organ key 4) builds the `acpi` crate's interpreter from the
  DSDT/SSDTs, runs `_PIC(1)` and `_INI`/`_STA`, reads `\_S5`, counts batteries, lids, thermal
  zones and I2C HID devices, picks the deepest `_CST` idle state, polls sensors, and runs
  `_PTS(5)` when the kernel announces a shutdown. AML runs on its own 512 KiB stack with a
  work budget per pass; a panic reports and parks, and only sexacpi stops.
- The kernel keeps FADT reset and S5 from the table bytes, the SCI and power button, the
  `Power` capability check and a firmware window (`FIRMWARE` right, `sex_pdx::power` ops
  16-22). The window refuses usable RAM, kernel/bootloader memory and the framebuffer, the
  PIC/PIT/PS2/COM1/CF8 ports, and writes to PCI BARs and ROM.
- Shutdown and restart never wait on sexacpi. The shell and SilkBar sensor ABI is unchanged
  (the kernel caches what sexacpi publishes).
- Known limits: MMIO of devices leased to other drivers is not yet refused (needs the IOMMU
  work, #86); `_PTS` is best-effort and races Files' flush; key 4 is shared with input, USB
  and Bell. #82 (dGPU `_OFF`) merged first; #88 moved its ACPI half into sexacpi too.
- Kernel acpi.rs went from 804 to ~650 lines; `test_kernel_never_runs_firmware_aml` keeps it out.

## Follow-up: in-kernel NIC probe removed (2026-11-10)

`kernel/src/hal/pci.rs` still held a historical boot probe that drove an e1000 directly from
the kernel (rings, DMA, and hand-built ARP, ICMP, UDP, TCP SYN, DNS and HTTP frames), gated
behind the build-time `SEXOS_BOOT_NIC_DIAGNOSTICS=1`. That is a network stack in ring 0. It
was removed in #88 (3,266 lines) when #78 and #86 pushed the kernel over its line budget.
The kernel now only logs `[pci.net.device]`/`[pci.net.passive]` and leaves the NIC to SexNet.
`SYS_NET_DIAG` (52) stays for SexNet's ABI and now always reports nothing; it can go once
SexNet stops asking.
