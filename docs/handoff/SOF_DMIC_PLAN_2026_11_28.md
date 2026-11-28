# Built-in microphones on Tiger Lake: audio DSP + Sound Open Firmware (2026-11-28)

## Why

The Alienware x17 R1's built-in microphones are digital microphones (DMIC) wired to
Tiger Lake's audio DSP (Intel cAVS 2.5). They are not on the Realtek HDA codec Bell
drives today, so only the headset jack records. Linux reaches them with the SOF driver
and Intel-signed SOF firmware (`sof-tgl-h.ri`, from `thesofproject/sof-bin`,
BSD-3-Clause source, redistributable binary). SexOS gets a native driver for the same.

## Shape

The PCI function (`00:1f.3`, `8086:43c8` TGL-H / `8086:a0c8` TGL-LP, class `04:01`)
has two faces:

- **BAR0** (16 KiB): HD Audio. Bell already runs the codec here (playback + jack mic).
  Extended capabilities: processing pipe (PP, `PPCTL.GPROCEN` + per-stream `PROCEN`
  decoupling) and multi-link (ML).
- **BAR4** (1 MiB): the DSP. Core power/reset/stall (`ADSPCS`), IPC doorbells
  (`HIPCIDR`/`HIPCIDA`/`HIPCTDR`/`HIPCTDA` + control), SRAM windows (mailboxes,
  ROM/FW status) from `0x80000`.

Design choice: **Bell keeps one owner for the whole function.** The codec streams
stay *coupled* (plain HDA, exactly as now). The DSP gets its own *decoupled* host DMA
streams (the code loader, then the DMIC capture PCM). No second service fights over
BAR0; the DSP lives in a new `servers/sexbell/src/dsp.rs` next to `hda.rs`.

Design choice: **no topology parser.** SOF topology files are just a recipe of IPC3
messages. We send the fixed DMIC capture graph that `sof-hda-generic-2ch.tplg` builds
(host PCM capture <- buffer <- DMIC DAI 0, 48 kHz, 2 ch, S32_LE on the wire) straight
from `crates/sex-sof/src/topology.rs`. IPC3 is TGL's default firmware ABI.

## Stages (each its own PR, merges on green)

| # | Change | Where | Testable in CI |
|---|--------|-------|----------------|
| 0 | Plan, `sex-sof` crate skeleton, host gate | this PR | gate builds |
| 1 | Register map, `Hw` trait, firmware image check, cAVS 2.5 boot (core 0 power, ROM handshake, code-loader DMA), IPC transport (doorbells + mailbox) | `crates/sex-sof/src/{regs,hw,fw,boot}.rs` | register-model tests |
| 2 | IPC3 messages + fixed DMIC topology (pipeline, buffer, host + DAI comps, connect, complete, DMIC DAI config, PCM params, trigger) | `crates/sex-sof/src/{ipc3,topology}.rs` | byte-layout tests vs SOF ABI |
| 3 | Plumbing: Intel cAVS lease gets the BAR4 DSP view + `sof.pack` firmware module; `scripts/fetch_sof_firmware.sh` (pinned sof-bin, SHA-256) + pack; image build carries the pack | `kernel/src/{device_lease,boot_memory}.rs`, `crates/sex-pdx`, `scripts/` | hwdb + QEMU gates stay green (QEMU has no DSP: absent path) |
| 4 | Bell: `dsp.rs` boots the DSP when the view + pack exist, starts the DMIC graph, adds "Built-in microphones" as a capture source in `mic.rs` ranking (plugged jack mic still wins) | `servers/sexbell/src/{dsp,mic,sound}.rs` | QEMU: DSP absent, Bell unchanged; x17 via tether |
| 5 | Later: NHLT (ACPI) DMIC count/geometry instead of the 2-mic default; 4-mic arrays; runtime power gating of the DSP when nobody listens | sexacpi + Bell | |

Stages 1, 2 and 3 are independent and run in parallel. Stage 4 needs all three.

## Contract between stages

- `sex-sof` is `no_std`, never allocates, `GPL-2.0-or-later OR MIT`.
- `hw::Hw` is what Bell implements: 32-bit (and 8/16-bit where HDA needs it) reads and
  writes on the BAR0 and BAR4 views, a microsecond delay, and one DMA scratch area
  (virtual slice + bus address) for the firmware copy, its BDL and the capture ring.
- `ipc3` and `topology` are pure: they write messages into a caller's `&mut [u8]` and
  return the length; they parse replies from `&[u8]`. `boot` owns the transport that
  puts those bytes in the outbox window and rings the doorbell.
- Log lines Bell prints: `[bell.dsp] ...` (detect, rom, fw_ready version, pipeline,
  capture periods). Every failure names its step so a tethered x17 log is enough.

## Not touched

- `servers/sexbell/src/hda.rs` input-pin logging from PR #853 (x17 trackpad/mic thread).
- Codec playback/jack mic paths: they stay coupled HDA and must behave exactly as now.

## Bare-metal test

Boot the x17 with the pack, then via tether: `logs` and look for
`[bell.dsp] fw_ready`, `[bell.dsp] dmic capture periods=N`; record with the built-in
mic selected and play it back.
