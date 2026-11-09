# SexOS against Linux and the modern desktops: gaps, advances and the plan (2026-11-09)

Andreas asked what Linux still has that SexOS lacks, what river, Hyprland and KDE have that
we need, which media capabilities are missing, how the design can match and then beat Linux
on capability and efficiency, and what would make people switch. This report answers those
from master `845de95` and turns the answers into workstreams. It builds on, and doesn't
repeat:

- `DAILY_DRIVER_ROADMAP_2026_11_08.md` (alpha definition, waves 1 to 3),
- `LIMITS_AND_PERFORMANCE_PLAN_2026_11_08.md` (stages 1 to 8, owned by the limits thread),
- `OPEN_SOURCE_INTEGRATION_REPORT_2026_11_08.md` (which free projects to bring in),
- `SEXOS_FEATURE_LEDGER_2026_11_02.md` (what runs today, feature by feature).

The headline: since the roadmap was written, the browser, media, JIT and limits threads
moved fast (WebGL, WebGPU, three.js, FFmpeg Play with sound, HTTP/1.1 keep-alive with flows
made on demand, C++ programs, W^X JIT pages, 256 KiB file I/O at Linux speed). **None of
wave 1 started.** Wave 1 is exactly what a real PC needs to boot, take input, shut down and
recover, and Andreas tests on bare metal next week. So wave 1 goes first, in parallel, now.

## 1. What Linux has that SexOS doesn't

Ranked by how soon a person on a real PC hits the gap.

### Kernel and hardware

| Area | Linux | SexOS today | Gap and owner |
|---|---|---|---|
| USB | xHCI/EHCI, hubs, every class driver | xHCI with **one** HID device on a root port (`SingleHidBind`), no hubs, no storage | Many devices at once, hubs, mass storage, then USB Ethernet, audio, UVC camera. **This thread.** |
| ACPI | Full AML, power off, reboot, buttons, lid, battery, thermal, S3/s2idle | `hal/acpi.rs` is an 11-line stub; MADT only | `acpi` + `aml` crates, FADT reset/poweroff, power button, battery, lid. **New thread A.** |
| Boot and install | GRUB/systemd-boot, GPT, dual boot, live USB | MBR install that overwrites the table, ISO not tested as a stick | GPT, shared ESP, hybrid USB image, graphical installer. **New thread B.** |
| Diagnostics | dmesg, pstore, kdump, journald | Serial only | On-screen boot log and panic screen (real PCs have no serial port), crash record on disk. **New thread C.** |
| NICs | Hundreds | e1000, virtio-net | e1000e (I219), igc (I225/I226), r8169/r8125; QEMU covers e1000e and igb. **New thread D.** |
| Sound hardware | HDA, USB audio, SOF | virtio-sound only | Intel HDA codec driver (QEMU `intel-hda`). **Thread D.** |
| Storage | NVMe, AHCI, USB, SD, RAID, LUKS | NVMe only | AHCI (thread D), USB mass storage (this thread), encryption later. |
| Display | DRM/KMS, i915, amdgpu, nouveau, multi-head | Limine GOP framebuffer, one mode | Native GOP mode, HiDPI scale, then KMS-style modesetting on i915/amdgpu. **New thread F.** |
| GPU compute/3D | Mesa (GL, Vulkan, VA-API) | Software WebGL/WebGPU across cores | Long horizon; a Mesa-class stack on our own kernel driver after modesetting. |
| Wi-Fi/Bluetooth | iwlwifi, mt76, BlueZ | None | Post-alpha; USB Ethernet as the laptop fallback. |
| Scheduling | EEVDF, cgroups, NUMA, real-time | Round-robin with wake queues, SMP | Priorities and latency classes, per-organ budgets. Limits thread stage 7. |
| Memory | Swap, zram, huge pages, KSM, OOM killer | Buddy + slab, arenas on demand | Memory pressure signal, compressed RAM, huge pages for arenas. Limits thread. |
| Virtualization | KVM | None | VT-x/AMD-V host for Linux guests (section 5). Post-alpha decision. |
| Security | LSMs, seccomp, namespaces | Capabilities and PKU domains by design | Already ahead in model (section 4); missing syscall fuzzing (thread C) and a signed boot chain (thread B). |

### Userland and platform

| Area | Linux | SexOS today | Gap and owner |
|---|---|---|---|
| Program toolchain | glibc/musl, every language | `no_std` Rust SDK, C via sex-libc, C++ via libc++ | A Rust `std` target, then POSIX as a guest library. **New thread H.** |
| Networking | Full TCP/IP, IPv6, sockets, TLS via OpenSSL | IPv4 over smoltcp, flows on demand, keep-alive, TLS 1.3 with AES-128-GCM only, no sockets for programs | TLS 1.2, ChaCha20, P-256/X25519 everywhere, IPv6, a socket API in the SDK. **New thread G.** |
| Files | ext4/btrfs/xfs, FAT, NTFS, FUSE | Copy-on-write sexfs on NVMe | FAT32/exFAT read-write, ext4/NTFS read, file manager, open/save picker with grants. **New thread I.** |
| Packages and updates | apt/dnf/flatpak, atomic distros | `sex-pkg` stub, no updates | Signed A/B system slots with rollback, signed app packages. **Thread B.** |
| Services | systemd restarts anything | Network and Files restart | Display, Input, Shell, Bell restartable. **Thread C.** |

## 2. What river, Hyprland and KDE have that we need

SexOS already tiles (Shell lays out windows, a window dragged off floats, edges resize,
tiles refill the space), has tabs per frame, a top bar (SilkBar with a civil clock), Alt+Tab,
a launcher and a text clipboard for built-in apps. Compared with the three:

| Feature | river | Hyprland | KDE Plasma | SexOS | Need |
|---|---|---|---|---|---|
| Workspaces / tags | Tags (bitmask) | Workspaces, special scratchpad | Virtual desktops, Activities | None | **Yes**: numbered workspaces, Super+1..9, move window to workspace, scratchpad. |
| Layouts | External layout generators | Dwindle, master | KWin tiling + floating | One tiling plus float | Master/stack, dwindle, monocle; per-workspace choice; keyboard focus and swap (Super+H/J/K/L). |
| Animations | None | Bezier window/workspace animations, blur, rounded corners, shadows | Effects (blur, wobbly, overview) | None in Shell | Window open/close/move and workspace slide animations, on the compositor's damage path, with a setting to turn them off. |
| Panel / bar | Via waybar | Via waybar | Plasma panel, tray, widgets | SilkBar clock | Workspace indicator, focused title, network, sound volume, battery, a tray area for programs. |
| Notifications | mako | dunst/mako | Plasma notifications with history | None | A notification service (capability-gated) with toasts and history; programs post through the SDK. |
| Launcher | fuzzel | wofi/rofi | KRunner | Launcher list | Fuzzy search over programs, files and settings, Super key to open. |
| Settings | None | Config file with live reload | System Settings | None | A Settings app: display scale, keyboard layout, time zone, network, sound, default apps, appearance, keybindings, saved as a config file that reloads live. |
| Keyboard layouts | xkb | xkb | xkb + GUI | US only | Layouts with dead keys and compose (US, UK, DE, FR, ES, Nordic, Dvorak, Colemak), switch shortcut. |
| Clipboard | wl-clipboard | Same, plus history via clipman | Klipper with history | Built-in apps, 12 KiB, text only | Every program (SDK), large text and images through a revocable region, history. |
| Screenshots / recording | grim, wf-recorder | grim/hyprshot | Spectacle | None | Region/window/full screenshots to a file and clipboard; screen recording through the media thread. |
| Lock screen / idle | swaylock, swayidle | hyprlock, hypridle | KScreenLocker | None | Lock with a password, idle dim/lock, display blank. |
| Multi-monitor | Yes | Yes, per-monitor scale | Yes | One framebuffer | Native mode + scale first (thread F), multi-head after modesetting. |
| Wallpaper / theming | swaybg | hyprpaper | Full theming | Fixed colours | Wallpaper from a file, light/dark, accent colour. |
| File manager | Any | Any | Dolphin | Files (basic) | Thread I. |
| Night light, gamma | wlsunset | hyprsunset | Night Color | None | Gamma in the compositor's final pass; cheap once settings exist. |
| Gestures | No | Workspace swipe | Touchpad gestures | No touchpad | After I2C HID touchpads (thread A finds them through ACPI). |

The desktop items except multi-monitor and native resolution go to **new thread E**, which
owns Shell, SilkBar and a new notification service. Display mode and scale go to thread F so
the two don't collide in `sexdisplay`.

## 3. Media capabilities still missing

Today: zune PNG/JPEG, FFmpeg n7.1 (SSE, nasm) in Play and in the browser's `<video>` and
`<audio>`, per-program 256 KiB sound rings mixed by Bell (16 streams) onto virtio-sound,
pause and seek, a YouTube thread working on MSE.

| Capability | Linux equivalent | Gap | Owner |
|---|---|---|---|
| Sound graph | PipeWire / PulseAudio | Per-program volume, master volume keys, device choice, sample-rate conversion, low-latency mode, capture (microphone) | Media thread |
| Sound hardware | ALSA HDA, USB audio | Intel HDA, USB Audio Class | Thread D (HDA), this thread later (UAC) |
| Streaming playback | GStreamer, mpv | Play and the browser load whole files; need progressive download and MSE | Media + YouTube threads |
| Hardware decode | VA-API, VDPAU, NVDEC | None; needs a GPU driver first | Post-GPU (thread F's later stages) |
| Screen capture | xdg-desktop-portal + PipeWire | None; in a SASOS the compositor can hand a capability to its frame buffers with no copy | Thread E (screenshots) then media (recording, encoder) |
| Encoding | FFmpeg encoders, x264, SVT-AV1 | Not built | Media thread (libx264/openh264 licence check, or rav1e/SVT-AV1) |
| Camera | V4L2, UVC | None | USB video class after USB (this thread, later) |
| Picture formats | libwebp, libavif, libheif, GIF | PNG, JPEG | WebP, GIF, AVIF through the media thread (FFmpeg already has the decoders) |
| PDF | poppler, mupdf | None | Thread H once `std` lands (hayro) |
| Streaming out (WebRTC, RTMP) | Many | None | After capture and encode |

These go to the existing **"Play images and video with FFmpeg"** thread through the
coordinator, except the hardware drivers.

## 4. Where the design already beats Linux, and how to press it

SexOS is a single address space with PKU domains ("organs") and capabilities. That gives
advantages Linux can't retrofit. Each item below says what to build so the advantage shows
up in measurements, not just on paper.

1. **IPC that costs a register write, not a page-table switch.** Moving between services is
   a `wrpkru` (tens of cycles on real hardware) instead of a CR3 switch, TLB loss and two
   mode changes. Build: direct handoff IPC (limits stage 6) and publish a Linux-vs-SexOS
   pipe/IPC round-trip benchmark on the PKU PC next week. Target: 10x fewer cycles than a
   Linux pipe round trip.
2. **Zero-copy everything.** Window pixels, sound rings, file regions and network buffers
   are already shared regions handed out as revocable capabilities. Compositing with no
   copy, screen capture with no copy, file reads into the program's own buffer. Build: the
   compositor composes directly from program surfaces (thread F), and the browser and Play
   decode into them.
3. **Drivers that crash and come back.** Like Minix, unlike Linux. Network and Files already
   restart; thread C extends this to Display, Input, Shell and Bell, and this thread to USB.
   Target: kill any driver during the soak and lose no work.
4. **Sandboxing by default, not by Flatpak.** Programs get no ambient authority: no file
   unless granted, no network unless allowed. The open/save picker (thread I) becomes the
   portal, and screen capture is a capability (thread E). Linux needs portals, seccomp and
   namespaces to approximate this.
5. **Atomic, signed updates from day one** (thread B), as in Fedora Silverblue or ChromeOS,
   but without a package manager underneath them.
6. **JIT and Wasm under hardware keys.** W^X JIT pages per PKU domain let V8 and Wasm
   engines run with their own key, so a browser exploit lands in a domain with no
   authority. Owned by the JIT/V8 work.
7. **A small trusted base.** A Rust microkernel in the tens of thousands of lines against
   Linux's tens of millions. Keep it that way: everything in this plan goes in services,
   crates and programs, not the kernel, except ACPI's power-off path and the minimum the
   USB and display drivers need to receive interrupts and DMA leases.
8. **Power.** A tickless idle (services already sleep when idle) plus ACPI C-states and
   P-states (thread A) should beat a stock Linux desktop at idle. Measure on the laptop.

## 5. What would make people switch

People switch for one thing they can't get elsewhere, and stay if nothing they rely on is
missing. In order:

1. **It installs safely next to what they have and boots on their PC** (threads A, B, C,
   D and this one). Without this nobody gets to step 2.
2. **The web works.** Most daily computing is a browser. The browser threads (V8, WebGL,
   YouTube) are the largest lever; keep them going.
3. **It feels instant.** Boot in a couple of seconds, open programs in milliseconds, no
   jank while a build or download runs. That is where the single address space should show.
   Publish the numbers.
4. **It is safe by design.** Every program sandboxed with no setup, updates that roll back,
   drivers that restart. A clear story for people burned by malware or broken updates.
5. **It looks and works like the desktops they admire.** Hyprland-style animations and
   tiling with KDE-level settings (thread E).
6. **Their programs still run.** The biggest gap for switching. Three paths, cheapest
   first: (a) a Rust `std` target and POSIX as a guest library, so open source programs
   recompile (thread H); (b) Wasm/WASI programs run as they are (wasmi is in); (c) a
   hardware-virtualised Linux guest (VT-x/AMD-V) whose windows show up in our compositor,
   the way ChromeOS runs Linux apps. Path (c) gives people every Linux program at once and
   keeps our own design pure. It is a large piece (a hypervisor service, virtio devices for
   the guest, a Wayland bridge), so it is listed here as a recommendation for after the
   alpha rather than started now.

## 6. Workstreams

Each is a separate thread so they don't collide. Every thread: QEMU gate first, real
hardware next week; follow SexOS design (services in PKU organs, capabilities, no POSIX in
the core); name any ABI change and recompute `abi_version_hash`; merge its own PRs when
tests pass; record each feature in the feature ledger.

| # | Workstream | Owner | Priority for bare metal |
|---|---|---|---|
| U | USB: many devices, hubs, keyboard + mouse together, mass storage, later USB Ethernet/audio/camera | **this thread** | Highest: desktop PCs have no PS/2 |
| A | Power and ACPI: AML, power off, restart, power button, lid, battery, C/P-states, finds I2C touchpads | new thread | Highest |
| C | Boot log on screen, panic screen, crash record on disk, restartable Display/Input/Shell/Bell, syscall fuzzing | new thread | Highest: real PCs have no serial |
| B | GPT installer next to another OS, hybrid USB stick image, signed A/B updates with rollback | new thread | High |
| D | Real-PC drivers: e1000e, igc, r8169, Intel HDA, AHCI | new thread | High |
| F | Native resolution and HiDPI scale, compositor from program surfaces with no copy, then modesetting and multi-monitor | new thread | High |
| E | Desktop: workspaces, layouts, animations, panel, notifications, launcher search, Settings, keyboard layouts, clipboard everywhere, screenshots, lock screen, wallpaper | new thread | Medium |
| G | Networking: TLS 1.2 + ChaCha20 + P-256, IPv6, a socket API for programs | new thread | Medium |
| H | Rust `std` target, POSIX guest library, PDF viewer | new thread | Medium |
| I | Files: FAT32/exFAT/ext4, file manager, open/save picker with grants, trash | new thread (after U's mass storage for sticks; FAT on images can start now) | Medium |
| — | Sound graph, streaming, screen recording, encoders, WebP/GIF/AVIF | existing media thread | — |
| — | Organ keys for many programs, IPC fast path, scheduler classes, memory pressure | existing limits thread | — |
| — | Browser daily use, V8, YouTube | existing browser, V8 and YouTube owners | — |

Later, after the alpha: Wi-Fi and Bluetooth, GPU acceleration and hardware decode, suspend,
the Linux guest (5.6c), multi-user.

## 7. How we'll know we've reached parity

- Bare-metal pass on the PKU PC: boots from a USB stick, USB keyboard and mouse, wired
  network, sound on HDA, shut down and restart, installs next to the existing OS.
- Every row of the tables in sections 1 to 3 marked done or deliberately post-alpha.
- Benchmarks on that PC beside Linux: IPC round trip, file read/write, program start,
  boot time, idle power, frame time for a window drag.
- A 24-hour soak with every service killed at random at least once.
