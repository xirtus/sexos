# Daily-driver capability gates and Linux outcome gaps (2026-11-10)

Planning snapshot: master `f034e3a2`. This is a release checklist, not a completion claim.
Existing ledger evidence below is reported from prior runs; this document reruns nothing.
PRs and hardware status change: integration must record the exact final SHA and fresh CI.

## Target and decision rule

By Thursday 2026-11-12, produce the strongest tested release candidate possible for the
x17 R1 bare-metal session next week. A daily-driver alpha requires the user's real work
for a full day on supported PKU hardware, safe storage, working network/browser/input,
recoverable services and reliable updates. Passing QEMU alone means QEMU-qualified.
Broad Linux capability parity is a longer-lived backlog and needs every outcome below
completed or explicitly excluded from a named release scope; it cannot be declared by date.

Architecture remains one address space, Rust microkernel mechanisms only, PKU organs,
BOOT capabilities and PDX. Drivers, protocols, filesystems and UI stay in servers.
Any POSIX compatibility remains in program libraries and creates no ambient authority.
Kernel budget stays 21,500 nonblank lines. Use `docs/DESIGN_GUARDRAILS.md` as binding policy.

Status vocabulary: **implemented** = source exists; **prior gate** = cited historical test;
**revalidated** = fresh reproducible run bound to candidate SHA/image; **hardware pass** =
recorded supported-machine test. No row starts revalidated or hardware-pass in this plan.

## Outcome matrix

| User outcome comparable to Linux | SexOS shape and current source/evidence | Remaining acceptance gate / known gap |
| --- | --- | --- |
| Boot a live USB without damaging another OS | `sex-disk` installer, partition-scoped sexdrive; feature ledger November 9 install entry; prior `install_gate.py`, `usb_stick_gate.py` | Revalidate UEFI/BIOS USB boot and unchanged foreign partitions, then x17; graphical install and firmware boot-entry registration remain gaps |
| Install, update and recover | GPT A/B layout implemented on master; signed/try-once implementation is #111 at snapshot | Merge only green #111, generate private release key outside repository, signed valid/tampered/rollback/power-cut gates; verify boot-chain limits explicitly |
| Keyboard, pointer, storage and USB networking | SexUSB xHCI/hubs/HID/mass-storage, shared USB-net rings; prior USB path/storage/net gates | x17 keyboard/mouse/touchpad and hotplug; I2C HID touchpad driver still needed; USB Ethernet only when no PCI NIC, ECM only, no NCM/RNDIS |
| Readable display at native resolution | SexDisplay framebuffer, Geist text, per-window zoom, #77 native mode/HiDPI | Repair/revalidate #77; source-bound screenshots at 1024x768/1280x800 and scale 2; no claim of GPU modesetting/multi-monitor support |
| Connect to IPv4/IPv6 internet | SexNet/smoltcp, DHCP/SLAAC, A/AAAA, rustls TLS 1.2/1.3; prior `program_ipv6_gate.py`, TLS/net gates | Public HTTPS on real network and x17; DNS still via DHCPv4, no DHCPv6/RDNSS/IPv6 fragmentation/MLD; UDP one-frame and TCP one pending accept |
| Stay online over Wi-Fi | Native Rust `sex-wifi` in SexNet; firmware-model gates for W1-W5e; #120 has rekey/rate/WPA3 | Read latest Intel handoff, use OpenBSD `if_iwx.c` as reference; merge green #120, then x17 association/DHCP/HTTPS, group rekey without key-reinstall/replay, rate up to 54Mb/s, WPA3 success/failure/reconnect; no QEMU radio proof |
| Browse ordinary websites | Web program + sex-html/CSS/JS, fetch/XHR/cookies, frames/postMessage and genuine Referer from #117 | Run real HTTPS embeds then YouTube /watch after JIT; selected everyday site/task suite (navigation, auth, forms, downloads, storage, history) with actual outcomes; no header spoofing |
| Execute JavaScript fast without weakening isolation | W^X CodeBuf per run and byte scanner; prior `program_jit_gate.py`; #118 is QuickJS wasm2c AOT | Repair #118 and run browser JS gate; true baseline hot-bytecode JIT remains a separate feature, behind flag, interpreter differential + bailout + W^X/PKU/revocation/multicore gates and measured speed |
| Run useful native programs | `sex-app`, `sex-std`, program threads, guest `sex-libc`, C++ toolchain; prior std/C++ gates | Integrate/revalidate #94 real clock/std::net including two-thread TCP; rebuild representative apps and exercise files/network/concurrency under grants; this is recompilation, not Linux binary compatibility |
| Keep and exchange files | COW sexfs; Files program/picker; folder/file grants; FAT/exFAT RW and ext/NTFS RO in sex-volumes; prior volumes/program-files/USB storage gates | Power-cut durability + foreign filesystem cross-checks on candidate/x17, unplug-safe behavior, large-file/data-integrity suite; NTFS compressed/encrypted reads unsupported; encryption/fsck tools need tracked work |
| Edit documents and read PDFs/images | Quil/Jot, autosave, Files picker, View/Play, hayro PDF program; prior Quil/std gates | Revalidate create/edit/save/reopen/clipboard/draft-recovery workflow with real content; define document-format scope rather than implying office-suite support |
| Play streaming audio/video | FFmpeg/dav1d, Bell per-program mixer, resampling, output selection/jack; progressive and Range media source | Run `program_web_seek_gate.py` FFmpeg ISO and streaming gates; verify seek before full download, A/V sync and failures; x17 analog speaker/headphones; hardware decode and HDMI/DP audio remain gaps |
| Use microphone/capture safely | Bell capture under explicit person's permission; prior `sound_listen_gate.py` | x17 HDA microphone path and visible live mic state; permission withdrawal stops capture; screen recording and camera are separate missing outcomes |
| Work in a consistent desktop | Shell four tiling layouts, nine workspaces, directional focus, scratchpad, search; notifications #110 merged into source | Revalidate WM/search/notification/close flows; execute UX plan first; complete Settings, international keyboard/compose, lock/idle and all-program clipboard as separately tracked work |
| Shut down and manage laptop power | sexacpi AML server, FADT mechanisms, HWP/CPPC, Files flush, NVMe shutdown/APST; prior power/crash gates | x17 power button/restart/shutdown, battery/EC/lid/thermal/fan safety, idle draw comparison; EC-backed sensors and suspend remain unqualified/gaps |
| Recover from faults without lost saved work | Restartable Network/Files/Input/Bell/Display/Shell; retained shell session; crash records; prior crash gates | 24h workload soak on candidate with deterministic failure injection for every restartable service, verify file hashes and session; hardware fault/resume test separately |
| Remain responsive under load | Per-core scheduling/latency classes, memory quotas, shared regions; prior benchmark/soak artifacts | Candidate-bound boot/app-start/input p50/p95/p99, IPC cycles, uncached reads/write, idle power beside Linux on same hardware; deeper NVMe queues only after uncached benchmark establishes bottleneck |
| GPU acceleration, multiple monitors and demanding apps | Software WebGL/WebGPU already implemented; no claim of hardware 3D | Native modesetting/monitor hotplug, GPU driver/command validation, accelerated API and video decode; benchmark actual games/workloads; long-term separate workstream |
| Broader Linux ecosystem capabilities | Guest libraries map common source APIs to PDX; no Linux process model in kernel | Explicit backlog: signed app delivery, Bluetooth, printing, cameras, accessibility tree/screen reader, disk encryption, suspend, multi-user, virtualization; investigate individually before asserting absence or completion |

## Priority and evidence bundle

1. Finish listed PRs #94/#110/#111/#77/#118/#119/#120. Check live state, synchronize
   master, preserve existing fixes, and merge only each exact tested head when all required
   CI checks succeed. If build-spec ABI hash conflicts, remove hash line and regenerate
   with `scripts/update_abi_hashes.sh`. Avoid unrelated PRs or merges.
2. x17 readiness: trim pre-init serial noise retaining `[wifi.*]`/`[sexnet.*]` failure and
   milestone tags; put PCI IDs/firmware hashes/boot options/checklist in the test bundle.
   Include USB Ethernet and UEFI network fallback. QEMU cannot replace Intel radio proof.
3. Run currently untested QEMU seek and browser JS gates on correct feature images; retain
   failing traces and fix timing only when it reflects observed protocol behavior.
4. Land true JIT in small gated increments, then repeat real HTTPS embed and watch tests.
5. Apply staged UX plan, recording before/after pixels and response times. Screen recording
   follows capture authority design. NVMe queue work follows an uncached benchmark.

Every result bundle needs candidate source SHA, image and firmware hashes, build flags,
host/QEMU or hardware details, run commands, serial logs, screenshots, expected vs observed
outcome, and failure/skip reasons. Keep secrets, passwords and signing keys out of bundles,
Git, logs and PR text. Add entries as new ledger fragments; never rewrite the big ledger.
Use small single-topic PRs and parallel CI; green CI does not certify hardware readiness.
