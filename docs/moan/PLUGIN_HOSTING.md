# Offline plugin hosting: intermediate slice

This work is in progress. It does **not** complete Moan live inserts, general
third-party desktop binary compatibility, or hostile-plugin sandboxing.

The first boundary is an offline file-processing job. Moan retains its timeline
and PCM caches; a disposable hidden `moanplug` program reads only explicitly
inherited plugin/input/output path tokens. Shell's authenticated Started and Done
receipts bind parent cookie, child domain and task. A nonzero fresh 128-bit nonce
binds the request/result; exact lengths, stereo48k, finite samples, frame and
parameter bounds are checked before publishing PCM. A nonce is a stale-result
fence, not a cryptographic file-integrity claim against another folder grantee.
The parent must keep polling after cancellation until exact retirement. Drop is
only best-effort kill; lost launch-event availability is not solved by this SDK.

`moanrack` is a developer Weft qualification window, not the Moan workstation UI.
Its fixtures prove external ABI processing and parent survival. Integration with
Moan's actual project/mixer UI is a subsequent adapter step coordinated separately.

## Supported target and format subset

Freestanding x86_64 little-endian ELF64 ET_DYN, SysV C/system ABI, SSE2, stereo
32-bit float, 48kHz, blocks1..256, at most60s per offline job. ELF input<=16MiB,
image<=32MiB, at most128 headers/sections and65536 relocations. Only RELATIVE,
64, GLOB_DAT and JUMP_SLOT relocations into mapped ranges are supported. There is
no dependency search, TLS, interpreter, constructor execution, W+X or overlapping
load pages. Relocation writes occur before SYS82 seals executable heap pages;
failed sealing or unsealing fails closed. Only explicitly enumerated libc and
allocator import addresses can resolve.

CLAP1: entry/factory first descriptor (count<=64), plugin init, exactly one stereo
input/output bus, activate/start, process, stop/deactivate/destroy and entry deinit.
No GUI, state, note events, parameter control or dynamic port changes yet. Restart
requests fail the render. The bundled external module is the upstream CLAP C
example, whose original DSP swaps stereo channels without modification.

VST3: generated bindings, ModuleEntry/GetPluginFactory/ModuleExit, first Audio
Module Class (count<=64), IComponent and IAudioProcessor, stereo buses, offline
32-bit processing, initial normalized parameters<=64, stop/deactivate/terminate
and COM release. Host context is currently null: plugins requiring host interfaces
are refused. No editor/controller GUI, MIDI, automation, state, dynamic buses,
latency/tail compensation or ordinary OS services are claimed. The source-built
vst3-rs Gain fixture is an upstream ABI example with unchanged DSP; initial param0
at0.25 makes processed audio distinguishable from a bypass. Native runtime proof
is recorded separately when completed.

Mach-O, PE, ordinary Linux runtime-dependent `.so`/bundles and system-installed
commercial binaries are unsupported. A `.clap`/`.vst3` suffix grants no trust.

## Containment and remaining authority

Separate run, self-only heap/code syscalls and program PKU/tenancy support contain
ordinary native faults. This is not a restricted hostile-plugin role. Ordinary
helper rows still have standard server routes, writable persistent own-data paths,
picker interaction and generic descendant launch. Exact inherited paths are
read/write grants, not read-only handles. No network flag is requested; Shell
requires a parent live network grant before forwarding it. No network isolation
proof is inferred merely from that omission. Parent exit does not cascade-kill
arbitrary descendants. Native syscall resource exhaustion is not bounded by the
host's format buffers, allocator conventions or job deadline.

The private Files relay-delegation repair2f85fff7 is not assumed to be present in
the c84 runtime image. Shell exit-status exact-task repair8b070e96 is a separate
userspace fix; existing c84 runtime evidence is not relabeled as testing it.

A restricted-helper design and route/syscall inventory must be reviewed before
changing Shell/Files policy. Required actual probes include ungranted reads/writes,
network, picker, direct BOOT routes, descendant launch, parent-memory access,
timeout/retirement and same-domain new-task replay. These remain pending.

## Live transport follow-up

Existing Bell playback/microphone rings are per-program and shared with Bell,
not delegated parent-child buffers. There is no audited parent-child PCM ring.
Offline file jobs must not be marketed as live inserts. A later live design needs
an existing authenticated broker or reviewed userspace transport, per-instance
sequence/epoch/nonce, bounded256-frame blocks, at most256 note edges/block,
initial parameters/state caps, cancellation/backpressure and exact retirement.
No shared-memory syscall, Unix IPC or kernel change is presumed.

At48kHz a256-frame block is5.333ms. A live implementation must measure p50/p95/p99
round-trip and CPU on actual supported hardware/guest, keep p99 processing+transport
within one block for an insert, bound queued blocks and discard stale responses.
Sustained overload must choose a documented bypass/mute and recover without
blocking Moan's UI/audio thread. These are requirements, not achieved measurements.
