# Native graphics and monitors: plan and status (workstream L)

Date: 2026-11-13. Thread "Drive real displays and monitors". Slices L1–L8 come from
`docs/handoff/DAILY_DRIVER_NEXT_100_SLICES.md` §3 L.

## Where SexOS is today

- Every screen is the firmware framebuffer (Limine/GOP). `sexdisplay` draws into two
  BOOT frame spans and publishes changed rows (`frame_output.rs`). There is no
  modesetting, no vsync (opcode 55 is a placeholder), no second monitor, no
  hardware cursor.
- EDID is read once by BOOT, only for the HiDPI scale (`sex_pdx::display_boot`).

## What this PR adds (L1, and the host-tested half of L3, L6, L7)

| Piece | Where | What it does |
| --- | --- | --- |
| KMS model | `crates/sex-kms` | connectors, CRTCs, planes, framebuffers, modes; atomic `check` that returns the whole next state or a typed refusal; modeset vs. flip detection; 64-bit vblank sequence from a wrapping frame counter; PDX wire format (opcodes `0x2F0..=0x2F8`) |
| EDID | `crates/sex-kms/src/edid.rs` | EDID 1.3/1.4: vendor, product, size, name, range limits, bpc, detailed / established / standard timings; CTA-861 extensions (VICs, HDMI VSDB, audio); CVT reduced-blanking generator |
| Multi-monitor layout | `crates/sex-kms/src/layout.rs` | internal panel first then left-to-right, bounds, output under a point, move a window to another output keeping its relative place (L5's model) |
| Intel Gen12 display | `crates/sex-gpu-intel` | Tiger Lake register map; GMBUS EDID (HDMI); DP AUX native + I2C-over-AUX EDID and DPCD caps (eDP, DP, USB-C); readout of what GOP lit (transcoder, timings, plane, PSR) into a KMS device+state; tear-free flips on the primary plane (SURF written last, SURFLIVE confirms); hardware cursor 64/128/256 with signed positions; frame-counter vblank wait; backlight PWM (never 0 %); hotplug live status for combo and Type-C ports; GGTT PTE encoding |

Gate: `scripts/kms_host_gate.sh` (CI "Display model (KMS, EDID) and Intel Gen12
display engine on the host"). The Intel tests run the driver against simulated
registers: a GMBUS that serves an HDMI EDID or NAKs, an AUX sink that defers, a
frame counter that ticks, and planes that latch at vblank.

Nothing in the kernel changed. All of it is ring-3 library code (guardrails:
drivers live in services). The Intel code is written from the public PRMs; the
MIT i915 headers were a cross-check only. Ledger table 2 has the row.

## The x17 R1 and its screens (what to check on the hardware)

- The internal panel is on the Intel iGPU in hybrid (Optimus) mode. If the BIOS
  or Alienware Command Center has the panel switched to the dGPU (Advanced Optimus
  / "discrete only"), the iGPU has no panel and only the NVIDIA path can drive it.
  **Test with hybrid mode.**
- HDMI 2.1 and mini-DP on gaming laptops of this class are normally wired to the
  NVIDIA GPU; the Thunderbolt 4 / USB-C port carries DP from the Intel iGPU (Type-C
  port TC1..). So on the x17, L4 "external screen" on Intel means USB-C/TB4. HDMI
  and mDP wait for `sexgpu` (NVIDIA plan N5). The first boot prints which ports
  are live, which settles this.
- The receipt the x17 boot should print (next PR wires the service):
  `[gpu.intel] pipe A port A eDP 1920x1080@165 psr=1 edid=AUO...`, a flip count
  that advances with `flip_done=1`, and the hotplug mask.

## Next PRs, in order

1. **L2 virtio-gpu in `sexdisplay` (QEMU).** Manifest row for virtio-gpu
   (1af4:1050) leased to `sexdisplay`; 2D resources backed by the existing frame
   spans (needs their device addresses through the DMA-lend window, syscall 90);
   `SET_SCANOUT` + `RESOURCE_FLUSH` per flip, fence completion as the flip event,
   paced at the mode's frame time; `GET_EDID` into `sex-kms`. Gate: QEMU
   `-device virtio-gpu-pci,max_outputs=2,edid=on`, boot log shows two connectors,
   flips at 60 Hz with monotonically increasing sequence. Falls back to today's
   GOP path when no virtio-gpu is present, so other gates do not change.
2. **`servers/sexgpu-intel` service (L3 on hardware).** Manifest row for
   8086:9a40/9a49/9a60/9a68/9a70/9a78 (Display class) → new service in the Graphics
   organ; leases BAR 0 (16 MiB: registers + GGTT in the top half). Boot: readout,
   AUX EDID of port A, compare with the running mode, then take over the plane by
   mapping our frame spans into free GGTT entries and flipping. Speaks the
   `sex-kms` wire protocol to `sexdisplay`. Only the x17 can test it; QEMU has no
   Intel display, so the host gate is the CI evidence.
3. **L6 cursor plane.** `sexdisplay` stops drawing the pointer into the frame and
   moves the cursor plane instead (`plane::cursor_move`); the virtio-gpu path uses
   `UPDATE_CURSOR`/`MOVE_CURSOR`. Gate: pointer motion with no frame damage.
4. **L4/L5 hotplug and multi-monitor.** Hotplug interrupt (SDE/DE HPD through the
   MSI vector of the lease) → re-read EDID → bump `OP_KMS_INFO` generation →
   `sexdisplay` re-arranges with `layout::Desktop` and tells the shell its new work
   areas per output. Type-C needs the TC cold/ownership handshake (`PORT_TX_DFLEXDPSP`,
   `DFLEXDPCSSS`) before AUX works; that is the first new hardware sequence.
5. **Full modeset on Intel (when GOP did not light a screen).** Sequence per the PRM
   "Display mode set" flow: panel power on (`PP_CONTROL`, T1–T3 waits), DPLL for
   the link rate (combo PHY DPLL0/1 or TC PLL), DDI buffer and link training
   (TPS1→TPS2/3/4 over AUX), transcoder timings, pipe, plane. Required for
   external DP on USB-C and for changing the panel refresh (165 ↔ 60 Hz saver).
6. **L7 PSR and backlight hand-off.** Readout already reports PSR/PSR2. With our
   own flips PSR1 exits on plane writes by hardware tracking; PSR2 selective fetch
   needs damage rectangles, which `sexdisplay` already computes (rows that differ).
   Backlight: workstream H2 owns the policy; it calls `backlight::set_percent`
   through the Intel service only when ACPI `_BCM` is absent.

## L8: the 3D plan

Goal: GPU-accelerated drawing for the desktop and the browser (WebGPU/WebGL), then
games, without a Linux driver bridge.

- **Memory.** `sexgpu-intel` owns the GGTT and per-context PPGTT (4-level, same
  format as the IOMMU page tables in `crates/sex-iommu`). Buffers are SexOS memory
  regions lent to the GPU through revocable caps, like the display frame spans.
  A process never names a GPU address it was not given.
- **Firmware.** GuC (scheduling) and HuC (media auth) blobs come from the firmware
  store (D8, same boot-pack path as Wi-Fi) with their Intel redistributable licence;
  ledger table 3 gets rows. Gen12 can submit through execlists without GuC, so step
  one does not need firmware at all.
- **Command submission.** Step one: one render engine, execlists, a ring per client
  context, `MI_BATCH_BUFFER_START` of a client-built batch in a lent buffer,
  completion by seqno writes + the engine's MSI vector. The service validates
  nothing inside batches (hardware PPGTT isolation does that job, as on Linux with
  full PPGTT), so it stays small.
- **User-mode driver.** Port Mesa's `iris` (Gallium) and `anv` (Vulkan) as
  libraries linked into the program (MIT), with a SexOS winsys over the
  `sexgpu-intel` PDX calls instead of DRM ioctls. That is the "Mesa-class path":
  GL/GLES/Vulkan for programs, wgpu on Vulkan for the browser's WebGPU (#37
  currently runs on the CPU), and `sex-gl` keeps its software path as fallback.
- **NVIDIA.** The same winsys shape fits NVK on top of the GSP driver (NVIDIA plan
  N6), so programs see one API on either GPU; PRIME-style offload copies through a
  shared lent buffer.
- **Order.** (a) GGTT/PPGTT + execlists + a blitter copy test (XY_FAST_COPY) as
  the first visible GPU work: the compositor's frame copy runs on the GPU;
  (b) Mesa `iris` hello-triangle in a program; (c) `anv` + wgpu; (d) GuC submission
  and media (HuC) for video decode with workstream G.
