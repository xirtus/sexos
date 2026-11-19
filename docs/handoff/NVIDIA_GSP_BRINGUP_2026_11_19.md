# NVIDIA RTX 3070 Laptop: GSP boot, display, modesetting, acceleration (2026-11-19)

Thread "Bring up the NVIDIA RTX 3070". Follows `NVIDIA_GPU_PLAN_2026_11_09.md` (stage N1,
power-off at boot, is merged: #82, #88). Target: the Alienware x17 R1's **RTX 3070 Laptop GPU,
GA104 (chipset 0x174), 8 GB GDDR6, PCI 10de:249d or 10de:24dd**.

Andreas (2026-11-18): NVIDIA and hardware support are the top priority.

## Where this stands

| Stage | What | State |
|---|---|---|
| N1 | Name the GPU, power it off (D3cold) when unused | merged (#82, #88) |
| **N4a** | **GSP-RM boot as driver logic: `crates/sex-gpu-nv`, host gate, real 535.113.01 firmware** | **this PR** |
| N4b | Kernel lease (BAR0 16 MiB view, command grant, 48 MiB DMA pool behind the IOMMU) + `servers/sexgpu-nv` runs the boot on the x17 behind a boot-menu entry; power-on through `sexacpi` + `FW_DGPU RESTORE` | next |
| N4c | First RM objects over RPC: root client, device, subdevice; `GET_GSP_STATIC_INFO`; GPU name, VRAM, VBIOS version in the boot log and Spindle `hw` | after N4b |
| N5a | Display: allocate the display classes (NVD7 core channel), read connectors and EDID through RM, light the HDMI 2.1 / mini-DP port with a framebuffer in VRAM | after N4c |
| N5b | The internal panel on the dGPU: mux / Advanced Optimus (to confirm on the x17: whether its BIOS offers a discrete-only or dynamic mux setting) | after N5a |
| N5c | Full modesetting: atomic commits through sex-kms, flips at vblank, cursor, hotplug, multiple monitors with the Intel panel | after N5b |
| N6a | Acceleration groundwork: channels (GPFIFO), copy engine for blits, a VM-bind interface shaped like nouveau's new uAPI | after N5 |
| N6b | 3D: NVK (Mesa, MIT) as an ordinary program over the POSIX guest; WebGPU in Kaleidoscope on real Vulkan | after N6a |

## Why GSP first, and why this order

Every Turing-and-later NVIDIA GPU runs its own RISC-V management processor (GSP) with NVIDIA's
signed firmware, GSP-RM. On GA10x there is no way to clock memory, light a display, or run a
shader without it: nouveau, NVIDIA's own open kernel modules, and the Rust nova driver all boot
GSP-RM first and then talk to it over shared-memory RPC. Display (N5) and acceleration (N6) are
RM objects you allocate over that RPC, so N4 is the gate for everything.

## N4a: what the crate does (in boot order)

`crates/sex-gpu-nv/src/boot.rs` runs eight named steps; each logs one line in the service.

1. **chip**: `PMC_BOOT_0` must decode to GA102..GA107 or AD102..AD107; VRAM size from
   `0x1183a4`; refuse if WPR2 is already set (`0x1fa828`, a previous driver left GSP-RM running).
2. **vbios**: the ROM window (BAR0 + 0x300000, shadow off via the PCI mirror bit `0x88050`),
   IFR header handled; walk PC/AT, EFI, FwSec, FwSec images; BIT table; token `p` gives the PMU
   table; entry 0x85 is FWSEC (descriptor v3). Offsets past image 0 are relative to the first
   FwSec image (they skip EFI).
3. **layout**: from the top of VRAM down: VGA workspace, FRTS (1 MiB), bootloader, GSP-RM ELF
   (64 KiB aligned), heap (LibOS 3 sizing: 20 MiB + 8 MiB + 96 KiB/GB + 96 MiB, at least 84 MiB),
   `GspFwWprMeta` (256 bytes); a 1 MiB non-WPR heap under WPR2.
4. **prepare** (system memory, all GPU-visible): GSP-RM `.fwimage` (0x2448000 bytes for 535) and
   its radix3 table, the GA10x signature, the RISC-V bootloader, the meta, LibOS arguments
   (LOGINIT, LOGINTR, LOGRM, RMARGS), `GSP_ARGUMENTS_CACHED`, the queues (page table + 256 KiB
   command queue + 256 KiB message queue), then `GSP_SET_SYSTEM_INFO` (BAR addresses, bus/devfn,
   PCI mirror) and `SET_REGISTRY` (nouveau's three keys) queued with checksums.
5. **fwsec_frts**: stage FWSEC with the signature the fuse at `0x8241c0` selects, patch the DMEM
   mapper to command 0x15 with the FRTS region, reset the GSP falcon, DMA IMEM (secure) and DMEM,
   boot through the boot ROM (`BROM_*` registers), check mailbox 0, scratch 0xe, and that WPR2 now exists.
6. **riscv**: reset the GSP into RISC-V mode (`BCR_CTRL |= 0x111`), LibOS argument address in its mailboxes.
7. **booter**: stage `booter_load` with the signature the fuse at `0x824148` selects, run it on
   SEC2 with the meta's address in the mailboxes; it copies GSP-RM into WPR2 and starts it. Check
   mailbox 0 and the RISC-V "active" bit; write the app version to the GSP's OS register.
8. **init_done**: read the message queue until `GSP_INIT_DONE` (0x1001), running
   `GSP_RUN_CPU_SEQUENCER` scripts (register writes, polls, core reset/start/resume).

Memory for the boot: about 39 MiB GPU-visible (the gate prints the exact figure for 535).

### How it is tested without a GPU

`scripts/nvidia_gsp_host_gate.sh` (CI):
- NVIDIA's real `booter_load`, `bootloader` and `gsp` 535.113.01 files parse to the values
  nouveau uses (IMEM 0x100+0x8300, DMEM 0x8400+0x6200, two 384-byte signatures, SEC2 ucode 3, ...).
- Every structure GSP-RM reads is pinned to the C layout: offsets were printed by compiling
  nouveau's MIT copy of the 535.113.01 headers (`GspFwWprMeta` 256 bytes, `GspSystemInfo` 664,
  `GSP_ARGUMENTS_CACHED` 80, `LibosMemoryRegionInitArgument` 32, queue element 48 + RPC header 32).
- A simulated GA104 (VBIOS with the real image chain, two falcons with DMA and BROM checks) runs
  the whole boot: it checks the signature the boot ROM would verify is in place, FWSEC is asked
  for FRTS at VRAM top - 2 MiB, Booter's radix3 walk returns exactly the GSP-RM image, the first
  RPCs carry good checksums; then it sends a log, a sequencer script and `GSP_INIT_DONE`.
  Failure paths: Booter error (names the step), silent GSP (bounded wait), WPR2 already set.

What the simulation cannot prove (only the x17 can): that the PROM has the VBIOS on this
laptop (some Optimus laptops expose it only through ACPI `_ROM`, which N4b adds as a fallback
through `sexacpi`), the real fuse values, Booter's acceptance of our meta, and GSP-RM's timing.

## N4b: running it on the x17 (next PR)

- **Power**: at boot the dGPU is in D3cold (N1). `sexgpu-nv` asks `sexacpi` to run `_ON`/`_PS0`
  for the GPU's power resources, then `FW_DGPU RESTORE` brings the link and headers back.
  Coordinate with "Prepare Alienware x17 for bare metal": its safe-start entry keeps the dGPU
  off, and so will the default entry until N4b is proven; a separate boot-menu entry
  ("SexOS with NVIDIA GPU") turns the boot on.
- **Lease** (`sex_hwdb::manifest` `Lease::NvidiaGpu`, kernel `devmgr`): BAR0 view of 16 MiB,
  memory + bus-master command bits, MSI (one vector), and a 48 MiB DMA pool inside the service's
  IOMMU lend window (128 MiB), so GSP-RM's own DMA stays fenced. BAR1/BAR3 addresses are passed to
  GSP-RM in `GSP_SET_SYSTEM_INFO` but not mapped yet.
- **Firmware**: 38 MB is far too big for the boot slot (about 1 MB free). The service reads the
  four files from the system volume (`/system/firmware/nvidia/ga102/gsp/`), checks their SHA-256
  against the pins, and copies them into the DMA pool.
- **Log**: `[nvidia.gsp] step=<name> ...` per step, `[nvidia.gsp.failed] step=... reason=...`;
  Spindle `hw nvidia` shows them.

## N5: display through the dGPU and the mux

The x17 R1's HDMI 2.1 and mini-DP ports are wired to the NVIDIA GPU; the internal panel is on
the Intel iGPU in Hybrid mode, and on NVIDIA when a mux setting (if the BIOS has one) selects it; to confirm on the machine. With GSP-RM up,
display is RM objects: allocate the display common/core classes, read connectors and EDID via
RM controls, then program heads through the core channel's push buffer (nouveau's `r535/disp.c`
is the reference). The panel and monitors join `sexdisplay`'s model as more outputs (sex-kms).
`GSP_SET_SYSTEM_INFO`'s ACPI method data (`_DOD`, `_MUX`, JT, NVOP) is how GSP-RM learns the mux
and backlight on laptops; `sexacpi` will evaluate those methods and hand the results over.

## N6: acceleration groundwork

Channels (GPFIFO + USERD) and the copy engine come first: they give `sexdisplay` fast blits and
Splicy/Polyamour a way to move frames. Then a VM-bind interface in `sexgpu-nv` shaped like
nouveau's new uAPI (`VM_INIT`, `VM_BIND`, `EXEC`), so Mesa's NVK ports as an ordinary program.

## Licences

- nouveau GSP code: MIT (Red Hat, NVIDIA). Re-implemented in Rust with notices; no code copied.
- NVIDIA open-gpu-kernel-modules: dual MIT/GPL; we used the MIT structure layouts.
- nova-core: GPL-2.0 code not used; its VBIOS documentation is GPL-2.0+ OR MIT.
- GSP-RM/Booter/bootloader binaries: NVIDIA redistributable licence (LICENCE.nvidia), data only.
- Our crate: `GPL-2.0-or-later OR MIT`, like the other driver crates. Rows in `docs/BRIDGE_LEDGER.md`.

ABI: none in N4a.
