# From today's master to a daily driver alpha (2026-11-08)

Andreas asked for a plan to get SexOS into people's hands as something they could install
and use every day. This report compares today's master (commit `f70739d`) with that alpha,
lists the gaps by area, and breaks the next days of work into parallel workstreams that can
each become a thread. It builds on three earlier documents and doesn't repeat them:

- `OPEN_SOURCE_INTEGRATION_REPORT_2026_11_08.md`: which free projects to bring in.
- `LIMITS_AND_PERFORMANCE_PLAN_2026_11_08.md`: stages 3 to 8 for limits and speed.
- `SEXOS_FEATURE_LEDGER_2026_11_02.md`: what already runs, with each feature's limits.

## Before the next wave starts

Andreas wants work in progress merged first. These cover the following topics, and this
roadmap leaves them out except where a new workstream depends on them:

| Work in progress | Where |
|---|---|
| FFmpeg images, video, then audio | PR #20, thread "Play images and video with FFmpeg" |
| Browser Canvas 2D, then WebGL and three.js | PR #26, thread "Bring HTML, CSS and JS to the Browser" |
| Hardware floating point (SSE) for programs | PR #18 |
| NVMe queue depth and disk speed, the rest of the limits plan | thread "Remove limits to reach Linux-level capability" |
| virtio network, disk and input | thread "Add virtio devices to SexOS" (done) and follow-ups |
| harfrust text shaping, more font weights | fonts thread |
| Window management | main thread |

Merge order that avoids conflicts: #18 (SSE), then #20 (FFmpeg, which builds on it), then
#26 (Canvas 2D). Each needs master CI green after it lands, as on commit `21db744`.

## What "daily driver alpha" means

A person with a recent Intel or AMD PC that has PKU can:

1. Write an image to a USB stick, boot it, try SexOS, and install it to a disk, next to
   another OS if they want.
2. Use a USB keyboard and mouse or a laptop's keyboard and touchpad, at the screen's
   native resolution, with sound.
3. Get online over wired Ethernet (Wi-Fi may come after alpha), browse ordinary websites,
   download files, and read and write them on the disk and on USB sticks.
4. Edit text, view pictures, PDFs and video, play music, copy and paste between programs,
   and change basic settings (keyboard layout, time zone, display scale, network).
5. Shut down, restart and (on laptops) see the battery; power loss doesn't lose more than
   the last few seconds of work.
6. Get updates that install safely and roll back if the new system won't start.
7. Leave it running all day without a crash, and when one happens, find a report on disk.

The alpha is judged by tests on QEMU (UEFI firmware, USB devices, Intel NICs and HDA sound
all exist there) plus one pass on Andreas's PKU PC. Real-hardware testing is still later
than features, so every workstream below must be testable in QEMU first.

## Where master stands, and the gaps

### Installer and boot

Today: `install` writes the running system to a disk and it boots alone under UEFI
(OVMF). The boot image sits at LBA 65536 behind a single MBR entry.

Gaps:
- **MBR only, and it overwrites the partition table.** Needs GPT, an EFI System Partition
  shared with other systems, and a guided "use free space / use whole disk" choice.
- **No graphical installer.** `install` is a Spindle command with no disk choice.
- **No USB stick image.** The ISO is not tested as a hybrid image written to a stick.
- **BIOS-only machines can't boot.** Acceptable for alpha (PKU machines all have UEFI).
- **Nothing tells the person what failed.** No boot-time panic screen and no saved log.

### Hardware drivers

Today: NVMe, PS/2 keyboard and mouse, an e1000 NIC, virtio sound, xHCI root-port scan
only, Limine's framebuffer, and MADT/APIC setup. `kernel/src/hal/acpi.rs` is a stub.

Gaps, in the order real PCs need them:
- **USB.** Real desktops and most laptops have no PS/2 keyboard. xHCI needs device
  enumeration, hubs, HID keyboard/mouse (`sexinput` already has HID report opcodes 0x260
  and 0x261 waiting) and USB mass storage.
- **ACPI.** Power off, restart, power button, lid, battery and thermal all need the AML
  interpreter. The `acpi` and `aml` crates (rust-osdev, MIT/Apache) fit.
- **Laptop input.** I2C HID touchpads (needs ACPI to find them) and PS/2 touchpads.
- **NICs found in PKU PCs.** Intel e1000e (I219), igc (I225/I226) and Realtek r8169/r8125.
  QEMU emulates e1000e and igb, so the Intel path can be tested now.
- **Sound.** Intel HDA (QEMU `intel-hda` + `hda-duplex`). virtio sound alone covers no PC.
- **Storage.** AHCI/SATA for older disks (QEMU `ich9-ahci`).
- **Display.** Native resolution through the UEFI GOP mode list chosen at boot, display
  scale. GPU drivers (i915, amdgpu) are post-alpha; software drawing plus SSE is enough.
- **Wi-Fi.** Intel iwlwifi needs firmware and a large driver: post-alpha. A USB Ethernet
  adapter (CDC-ECM/NCM, RTL8153) is the alpha fallback for laptops.
- **Suspend (S3/s2idle)**: post-alpha.

### Networking

Today: DHCP, DNS, HTTP and HTTPS (TLS 1.3, one cipher) through the network service's open
side. `fetch` and the Browser work against public sites.

Gaps:
- **One flow at a time across the machine.** A browser alone needs a dozen at once.
- **TLS**: TLS 1.2, ChaCha20-Poly1305, X25519, AES-256; session resumption. Many sites
  still need one of these.
- **HTTP/1.1 keep-alive and chunked bodies**, then HTTP/2. Content-Length checks.
- **IPv6**, and a network settings view (address, DNS, link state).
- **Sockets for programs.** Programs get the network only through `web`-style jobs; an
  SDK socket API (TCP/UDP streams over a shared ring) is needed for anything else.
- The `smoltcp` crate (0BSD) can replace the hand-written stack if it is cheaper than
  growing it; decide in the networking thread after measuring.

### Storage and files

Today: a copy-on-write filesystem on NVMe with persistence, crash recovery and installs.
Speed is the limits thread's.

Gaps:
- **Other people's disks.** FAT32/exFAT read and write for USB sticks and cameras;
  ext4 and NTFS read-only for dual boot (all post-USB-mass-storage).
- **A file manager that does the daily things**: copy, move, rename, delete with undo,
  open with, a picker other programs use to open and save.
- **Per-program file access by grant**, so a program sees only the files the person gave
  it (the "give a program one file" work generalised to folders).
- **Trash and free-space warnings.**

### Browser

Today: HTML with CSS laid out in Geist, page JavaScript with timers and click handlers,
real HTTPS sites. Canvas 2D and WebGL are in progress (excluded).

Gaps for daily browsing:
- **Images in pages** (PNG and JPEG through zune already exist; WebP and GIF next).
- **Forms**: text fields, buttons, checkboxes, POST, and the keyboard focus to drive them.
- **Cookies and storage** (`document.cookie`, `localStorage`), per site, saved to disk.
- **Fetch/XMLHttpRequest** and more of the DOM (events, `querySelector`, class lists).
- **Tabs, history, bookmarks, downloads, find in page, zoom, scrolling with the wheel.**
- **Web fonts** (WOFF2 through the fonts thread's runtime fonts).

### Media

Today: View opens PNG and JPEG, sine notes play on virtio sound, FFmpeg video and audio
are in progress (excluded).

Gaps:
- **A sound service with a mixer**: PCM streams from programs through shared buffers,
  per-program volume, a master volume key. Every player depends on it.
- **A music player and a video player app** on top of the FFmpeg work.
- **PDF viewing.** `hayro` (Apache/MIT, pure Rust) or `pdf-rs` plus the tiny-skia canvas.
- **Screenshots** to a file, and a screen recorder later.

### Apps and the desktop

Today: Spindle (terminal), Jot and Quil (text), Files, View, Browser, calculator, shapes,
Wasm guests, programs from disk with arguments, a heap, tiny-skia drawing and Geist at
any size.

Gaps:
- **Clipboard for every program.** A text clipboard already works: Shell brokers it
  (`servers/silk-shell/src/clipboard.rs`, `crates/sex-pdx/src/clipboard.rs`), Ctrl+C/X/V
  route through it, Quil can copy, cut and paste, and the Browser can copy, up to 12 KiB.
  Missing: programs run from disk (`App::Program` is refused), Spindle, Jot and View;
  larger text through a shared region; images and rich text.
- **Keyboard layouts and compose/dead keys.** Only US QWERTY exists.
- **Settings**: time zone and clock (sex-time exists), display scale, layout, network,
  sound, default apps, about this system.
- **Lock screen and one local account with a password.** Multi-user is post-alpha.
- **A Rust `std` target** (integration report 1b) so ordinary Rust programs and crates
  build for SexOS. This is the largest single lever for apps.
- **POSIX as a guest** (relibc is in the tree) for C programs: post-alpha.
- **An app store that installs from the network** (sexstore exists, `sex-pkg` is a stub).

### Updates

Today: nothing. An installed system stays at the version it was installed with.

Gaps:
- **A/B system slots.** The boot slot at LBA 65536 becomes two; an update writes the
  other slot, marks it "try once", and the bootloader entry falls back if the new
  system doesn't confirm a healthy start.
- **Signed images.** Ed25519 signatures checked by the running system before writing a
  slot, with the public key in the image. Release key handling stays offline.
- **An update channel**: a small signed index on a static host (GitHub Releases works)
  that names the newest image and its hash.
- **App packages** through the same signing, installed into the program store.

### Stability and diagnostics

Today: a 3-hour soak passed at 4 cores; Network and Files restart after a crash; a
program that ignores Close is ended.

Gaps:
- **Crash reports**: a panic screen with the reason, and the last serial log and panic
  record kept in a reserved disk area, shown after the next boot.
- **Every service restartable** (Display, Input, Shell, Bell, not only Network and Files).
- **Syscall and IPC fuzzing** in CI (the kernel's argument checks are hand-reviewed).
- **A 24-hour soak** at 4 and 8 cores, with memory and handle counts graphed per round.
- **Delete fixed claims**: `kernel/src/benchmark.rs` prints numbers it did not measure.

### Release engineering

Gaps:
- Version numbers, a changelog drawn from the ledger, a signed release ISO built by CI.
- An install guide, a list of tested hardware, and a "report a bug" path that attaches
  the crash report.

## Workstreams for the next days, in priority order

Each line is one thread. The first wave can all start once the work in progress is
merged; they touch different servers and crates, so they shouldn't collide. Any change to
an ABI input needs `abi_version_hash` in `sexos_build_spec.toml` recomputed; threads that
touch `kernel/src/boot_memory.rs` (rows and leases) should merge one at a time.

### Wave 1: what an alpha can't ship without

1. **USB keyboard, mouse and sticks** (Senses + Storage organs). xHCI enumeration, hubs,
   HID boot protocol into `sexinput`, then USB mass storage (bulk-only, SCSI) as a block
   device. QEMU: `-device qemu-xhci -device usb-kbd -device usb-tablet -device usb-storage`.
   Done when the desktop runs with no PS/2 devices and a USB stick's file opens.
2. **Power, restart and ACPI.** The `acpi` + `aml` crates, FADT power off and reset,
   the power button event, battery and lid on laptops. Shut Down and Restart in the
   desktop that flush Files first. QEMU: `system_powerdown` ends the VM cleanly.
3. **Installer with GPT and dual boot.** GPT writer, shared ESP with a Limine entry,
   install to free space, a graphical installer program with a disk and size choice,
   hybrid ISO for USB sticks. Done when the gate installs next to an existing partition
   and both survive.
4. **Many network flows, wider TLS.** Concurrent flows in the network service, HTTP/1.1
   keep-alive and chunked bodies, TLS 1.2 and ChaCha20/X25519, then a socket API in the
   SDK. Done when the Browser loads a page and its images over parallel connections
   and a test matrix of top sites passes.
5. **Safe updates.** A/B slots, Ed25519 signed images, "try once" with fallback, an
   `update` command and a signed index on GitHub Releases. Done when a gate installs
   v1, updates to v2, boots v2, then forces a bad v3 and lands back on v2.
6. **Crash reports and restartable services.** Panic screen, reserved crash area on disk,
   report shown after reboot, restart for Display, Input and Shell, a syscall fuzzer in
   CI. Done when killing each service in a gate leaves the desktop usable.

### Wave 2: what makes it pleasant every day

7. **Clipboard everywhere, keyboard layouts and Settings.** Extend the existing Shell
   clipboard broker to disk programs (an SDK call), Spindle, Jot and View, with a
   revocable shared region for large text and images; layouts (US, UK, DE, FR, Nordic, Dvorak) with dead keys, a Settings
   app for time zone, scale, layout, network and sound.
8. **Real-PC network and sound drivers.** e1000e and igc (testable on QEMU e1000e and
   igb), Intel HDA with the mixer below. r8169 needs real hardware, so it waits.
9. **Sound mixer and players.** PCM streams from programs, mixing, volume keys, then a
   music player and a video player on the FFmpeg work.
10. **Browser for daily use.** Images in pages, forms, cookies and localStorage, fetch,
    tabs, history, bookmarks and downloads. Runs beside the Canvas/WebGL thread in
    different modules (`crates/sex-web` layout and network vs. the JS canvas bindings).
11. **Rust `std` for SexOS.** A `std` target over the SDK (files, time, threads later,
    sockets from workstream 4), so crates such as `hayro` and app frameworks build.
12. **File manager and other people's disks.** FAT32/exFAT read-write, ext4 read,
    copy/move/rename/trash, an open/save picker that hands programs file grants.

### Wave 3: after the first alpha

13. **PDF viewer** (on `std` and tiny-skia), screenshots, an archive tool.
14. **Lock screen and an account password**, then disk encryption.
15. **App store that installs signed packages** from the update channel.
16. **Native display resolution and scale** from the GOP mode list, then multiple
    monitors and the first GPU driver.
17. **Laptop extras**: I2C HID touchpads, USB Ethernet adapters, backlight, suspend.
18. **Wi-Fi** (Intel first), and **POSIX guests** on relibc for C programs.
19. **Release engineering**: versioned, signed ISOs from CI, install guide, tested
    hardware list, bug reports that attach the crash record. Can run in parallel from
    wave 1 with little code.

## Alpha exit checklist

- [ ] Boots from a USB stick and installs next to another OS under UEFI (QEMU + the PKU PC).
- [ ] USB keyboard, mouse and stick; PS/2 still works.
- [ ] Shut down, restart and power button through ACPI.
- [ ] Wired network on virtio and Intel NICs, parallel flows, TLS 1.2 and 1.3.
- [ ] Browser shows images, fills forms, keeps cookies, has tabs and downloads.
- [ ] Sound on HDA and virtio with a mixer; music and video players.
- [ ] Clipboard in every program, keyboard layouts, Settings.
- [ ] Signed A/B updates with rollback.
- [ ] Crash reports on disk; every desktop service restarts.
- [ ] 24-hour soak at 4 cores with no fault and flat memory.
- [ ] The limits plan through stage 6 (windows that fit the screen, rows without limit,
      IPC fast path), owned by the limits thread.

## Risks

- **Real hardware is the unknown.** QEMU's devices are kind; real xHCI controllers, ACPI
  tables and NICs are not. One day on the PKU PC early in wave 2 will find more than a
  week of emulation.
- **PKU key count.** Many daily programs at once lean on the organs design; the limits
  thread's stage 5 has to land before an alpha that runs a browser, a player and a
  terminal side by side without sharing keys.
- **Network stack size.** Growing the hand-written stack to many flows, IPv6 and TLS 1.2
  may cost more than adopting `smoltcp` and `rustls`'s full suite; measure before
  deciding.
- **Updates are the one thing that can't be fixed later.** An alpha without rollback
  strands its testers on a broken build, so workstream 5 ships with the first release.
