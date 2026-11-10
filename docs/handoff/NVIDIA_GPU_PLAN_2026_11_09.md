# NVIDIA laptop GPU: power off now, GSP and outputs next (2026-11-09)

Thread "Run the NVIDIA laptop GPU". Follows section 4 (P3) of
`DRIVERS_AND_X17_R1_PLAN_2026_11_09.md`. Target: the Alienware x17 R1's RTX 30 laptop GPU
(GA104 for the 3070/3080 Laptop, GA106 for the 3060 Laptop; no x17 R1 shipped a 3090).
Andreas's machine is most likely the **RTX 3070 Laptop: GA104, 8 GB, PCI 10de:249d or
10de:24dd, HDMI audio 10de:228b**. GA104 is the first target for every stage below.

## Where this stands

| Stage | What | State |
|---|---|---|
| N1 | Name the GPU and power it off (D3cold through ACPI) when nothing drives it | **done, this PR** (QEMU gate; bare metal next week) |
| N2 | Firmware store: GSP-RM, booter and FWSEC images as data files | planned (needs driver stage D8) |
| N3 | `sexgpu` service in its own Graphics organ: BAR0/BAR1 lease, MSI-X, IOMMU domain, power on through a capability | planned (needs D2-D4) |
| N4 | GSP boot on Ampere: VBIOS, FWSEC-FRTS, WPR2, booter on SEC2, GSP-RM, RPC queues | planned, hardware only |
| N5 | Display: HDMI and mini DisplayPort outputs through the GPU's display engine | planned, hardware only |
| N6 | 3D: NVK (Mesa, MIT) over a SexOS channel/VM-bind interface; WebGPU in the browser on top | planned |

## N1: what BOOT does now

`kernel/src/hal/dgpu.rs`, called from `init.rs` just before device leases.

1. Every display controller (class 03) is listed: `[gpu.found] at=01:00.0 vendor=10de
   device=249c class=0300 name="GeForce RTX 3080 Laptop" chip=GA104:0x174`. For NVIDIA the
   chip comes from the GPU's own `PMC_BOOT_0` register (BAR0 + 0), read only when BAR0 is
   below 4 GiB, memory decode is on and the kernel's direct map covers it. **This answers
   the open lspci question on the first boot.**
2. It stays on when it sits on the root bus (an integrated GPU: the Intel UHD at 00:02.0) or
   when the boot framebuffer is inside the memory its root port forwards (BIOS
   "discrete only" mode, or a mux set to the NVIDIA GPU). `[gpu.power] action=keep reason=...`.
3. Otherwise, like Linux runtime power management: every function in the slot goes to D3hot
   (PCI PM registers, 10 ms wait); the GPU's ACPI node runs `_PS3` and turns off the power
   resources its `_PR0`..`_PR3` name; then, when the GPU is all that is behind the root port,
   the port goes to D3hot and its own `_PS3` and power resources run. On Tiger Lake laptops
   the resource is on the root port (`\_SB.PC00.PEG1`, `_PR3 = {PG01}` or similar), so that
   last step is what cuts power. A resource another device lists in its `_PR0` is left on.
4. Result: `[gpu.power] ... action=off state=d3cold|d3cold_requested|d3hot ... steps: ...`.
   `d3cold` means the GPU stopped answering configuration reads (power really went);
   `d3cold_requested` means the firmware methods ran but the device still answers (QEMU);
   `d3hot` means no ACPI path, only the PCI state.

Safety:
- Firmware AML runs quarantined (from the ACPI work) and now also with a **work budget**
  (`acpi::with_aml`): about 3 s of register access and stalls. The `acpi` crate has no loop
  limit, so a `_OFF` that waits forever on a link bit would otherwise hang the boot; now the
  budget runs out, the attempt ends, AML is set aside and the boot continues.
- Each function's configuration header (and the root port's) is saved first. `power_on`
  turns the resources back on, runs `_PS0`, restores D0 and the headers, and waits up to a
  second for the link. Nothing calls it yet except the gate; the N3 driver will.
- Known limit: the `acpi` crate has no Embedded Controller region handler yet. If Dell's
  `_OFF` touches EC fields it stops there; the log then shows `sta=1` and the GPU stays in
  D3hot. Fix: the EC handler the ACPI work already lists as next.

Gate: `scripts/dgpu_power_gate.py` (CI). QEMU gets a PCIe root port at 00:1c.0 with a display
controller behind it, plus an SSDT (`tests/acpi/dgpu_gate_ssdt.asl`) giving the port a power
resource named by `_PR0`/`_PR3` and the GPU `_PS0`/`_PS3`; the methods write to the debug
console, so the gate sees them run in order (`3FN03F`: off, on, off). The SSDT's `SXGT` name
makes BOOT power the GPU back on and off once, so `power_on` is tested too. The boot VGA on
the root bus stays on and the desktop still comes up.

ABI: none. No syscall, capability or `sex-pdx` change; `abi_version_hash` unchanged.

### On the x17 next week

With BIOS Graphics set to **Hybrid**, look for (boot log screen or crash record; the x17
has no serial port):
- `[gpu.found] ... vendor=10de ... chip=GA104:0x174` (or GA106) and the Intel GPU kept;
- `[gpu.power] ... state=d3cold` and the `steps:` list (names of the nodes and resources);
- fans quieter and lower idle draw than with the GPU on (compare with the BIOS screen).

If it says `d3hot` or `sta=1`, send the line: it names the firmware method that stopped.
With BIOS set to discrete only, expect `action=keep reason=boot_framebuffer`.

## N2-N6: why it is long, and the order

Every RTX 20 and later GPU runs its own management CPU (GSP) with NVIDIA's firmware
(GSP-RM, about 60-70 MB for GA10x). Open drivers (nouveau's GSP path, the Rust nova driver,
NVIDIA's own open kernel modules) all boot it and then talk to it over shared-memory RPC
queues. There is no way around it for clocks, memory training or 3D.

Licences: NVIDIA's open-gpu-kernel-modules are **dual MIT/GPL**, so their RM interfaces and
GSP boot code can be ported (MIT notices kept). nouveau is MIT. nova (Rust) is GPL-2.0: read
only. The firmware blobs come from linux-firmware (redistributable, never linked; loaded as
data). Mesa NVK is MIT.

- **N2 firmware store** (driver stage D8): `gsp-*.bin`, `booter_load/unload`, `bootloader`
  for GA104/GA106, verified by hash, read from the system volume by `sexgpu`.
- **N3 `sexgpu`**: a Graphics organ (its own PKU key; `PROGRAM_SLOTS_DESIGN`). The kernel
  leases BAR0 (16 MiB registers) and a BAR1 window, MSI-X vectors (D3), and an IOMMU domain
  (D4) before any DMA is allowed: GSP reads and writes system memory on its own, so it must
  be fenced. A capability op asks the kernel to `power_on` the GPU, and to power it off
  again when `sexgpu` exits or idles (runtime D3cold, which is also how Linux saves battery).
- **N4 GSP boot** (Ampere sequence): read the VBIOS from PROM, run FWSEC-FRTS on the GSP
  falcon to carve the WPR2 region, load `booter_load` on SEC2, hand GSP-RM its radix-3 page
  tables and the RPC queues, wait for `GSP_INIT_DONE`, then query the GPU's info over RPC.
  Testable only on the laptop; each step logs a line so a boot photo is enough to debug.
- **N5 outputs**: the x17's HDMI 2.1 and mini DisplayPort are wired to the NVIDIA GPU. Allocate
  the display classes through RM, drive the core channel, read EDID over the AUX/I2C
  channels, train DP links, start with TMDS HDMI (4K60) before FRL. Outputs join the display
  service's model as more monitors (coordinate with "Native resolution and HiDPI scale").
- **N6 3D**: a channel and VM-bind interface in `sexgpu` shaped like nouveau's new uAPI, so
  NVK (Mesa) ports as an ordinary C program over the POSIX guest; WebGPU in the browser then
  gets real Vulkan instead of the CPU rasterizer.

## Differences from the sex-manual

The manual's "Phase 5: NVIDIA 3070 GPU PD (nouveau-lifted skeleton)" was a placeholder with no
code. What changed and why:
- **Power before pixels.** On a hybrid laptop the Intel GPU already gives us the panel; the
  NVIDIA GPU's first job is to stop costing battery and heat. The manual started with a
  driver skeleton.
- **No nouveau lift.** Nouveau's pre-GSP code (its own reverse-engineered firmware) does not
  reclock Ampere and is being replaced upstream; GSP-RM is the only path to full speed. We
  follow the GSP design, porting from NVIDIA's MIT code.
- **Kernel does power, a service does the GPU.** Power resources are AML, which only the
  kernel's quarantined interpreter runs, so D3cold lives in the kernel next to the ACPI code;
  everything that touches the GPU's registers or memory belongs to `sexgpu` in its own organ,
  as the multiserver design wants.
