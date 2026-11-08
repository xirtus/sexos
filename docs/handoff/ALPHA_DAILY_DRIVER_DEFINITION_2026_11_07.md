# What "alpha daily driver" means for SexOS (2026-11-07)

Andreas's goal: build SexOS into an alpha daily driver, true to the single address
space design, moving toward a real competitor to Linux. This says what done means and
what's left, so progress is checkable. Alpha means rough and missing things is fine.
It doesn't mean a demo: a person must be able to get real daily work done on it.

## Done means

One person can install SexOS on a real Intel or AMD PC with PKU, and for a working day:

1. **Boot and install.** Install to the machine's own disk from a USB stick. Boot
   without help.
2. **Keep files.** Real directories and gigabytes of files, which survive a power cut.
3. **Reach the internet.** Get an address by itself, on wired Ethernet at least, and
   open ordinary websites by name over HTTPS.
4. **Run real programs.** Install a program as a file and run it, in a window, with
   files and network if allowed. Many at once, on every core.
5. **Work in windows.** Move, resize, switch and close windows. Readable text, with a
   real font and real pixels for images.
6. **Have the basics.** Terminal, editor, file manager, browser, settings, image viewer.
   Sound if the hardware is common.
7. **Stay up.** A full day with no crash, and a crashed service restarts without losing
   the session. Nothing a person saved is ever lost.
8. **Stay honest about the design.** One address space, boot-only capability genesis,
   typed authority, PKU, no Unix process model. A POSIX layer may come later, as a guest.

## Where it stands (updated 2026-11-08)

| | Need | Today | Gap |
|---|---|---|---|
| 1 | Install and boot | Boots from ISO in QEMU only | **Never run on real hardware.** No installer |
| 2 | Files | sexfs: COW B-tree, directories, gigabytes, crash-safe commits | Bigger caches, fsck tooling, real-disk testing |
| 3 | Internet | DHCP, DNS, checked TLS 1.3; `fetch` and a Browser that opens real sites | TLS 1.2, more cipher suites, one flow at a time, no real NIC tested |
| 4 | Programs | Installed as files, 8 windowed at once with keys and their own folder, `sex-app` SDK, 4,096 background | Network for programs, arguments, more rows, signed installs |
| 4 | Cores | Services and apps spread across cores | Shell and Display on core 0, no balancing, no interrupt routing |
| 5 | Windows | Tiling grid, title bars on every window | **Move, resize, overlap** |
| 5 | Graphics | Fixed bitmap font, text-first | **Scalable font, pixel surfaces** |
| 6 | Apps | Spindle, Quil, Linen, Browser (text reader of real sites), Jot | Settings, image viewer, layout and CSS in the browser |
| 7 | Reliability | 30 minutes qualified, services don't restart | **Restartable services, day-long soak** |
| 8 | Design | Held throughout | Keep holding it |

## Order of work

Each step should leave the machine more usable than before:

1. **Real filesystem.** Everything else stores things, and today's 1 MiB blocks it.
2. **Network that reaches the internet.** DHCP, real DNS, any HTTPS host, with a capability
   that says which programs may use it.
3. **Windowed programs from disk.** Program records with routes, then a small SDK.
4. **Window management and a real font.**
5. **Restartable services.** Most of the desktop must survive any one crash.
6. **Real hardware and installer.** Sooner if a test machine is available, since hardware
   problems found late are expensive. The stress tests already found two boot bugs QEMU's
   defaults had hidden.
7. **Browser, settings, image viewer, sound, then a day-long soak on real hardware.**

Multi-core continues alongside: Shell and Display off core 0, interrupt routing, and a
real speed measurement once there's hardware to measure on.
