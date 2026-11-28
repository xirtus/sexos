# 2026-11-28: `sex-sof` stage 1: register map, `Hw`, firmware check, DSP boot, IPC transport

- `crates/sex-sof/src/regs.rs`: BAR0 HDA (GCTL, GCAP, INTCTL, stream descriptors, Intel EM2/LTRP, extended capability walk: PP `PPCTL.GPROCEN`/`PIE`/per-stream `PROCEN`, SPIB, ML) and BAR4 TGL DSP (ADSPCS SPA/CPA/CRST/CSTALL, ADSPIC/ADSPIS, CNL-style IPC `HIPCTDR/TDA/TDD/IDR/IDA/IDD/CTL` with BUSY/DONE, ROM purge IPC, SRAM windows, FSR states, default mailboxes), Linux-like timeouts.
- `hw.rs`: `trait Hw` (BAR0 8/16/32-bit, BAR4 32-bit, `delay_us`, `dma()` scratch slice + bus address, optional `dma_sync`).
- `fw.rs`: `fw::check(image) -> Info` validates extended manifest (`XMan`: version, windows), `$CPD`, CSS `$MN2`, `$AM1`; no allocation.
- `boot.rs`: `boot::boot(hw, image, &Config, fw_ready_buf) -> Result<Booted, BootError>` runs the Linux TGL sequence (controller ready, PP on, ICCMAX stream + 95 us guardband, decoupled code-loader output stream with BDL/FMT/CBL/LVI/SPIB, core 0 power, purge IPC with loader tag, unreset, unstall, ROM DONE, IPC interrupts, INIT_DONE, 3 attempts, DMA run, FW_ENTERED, DMA stop, streams re-coupled, EM2/LTRP restored, FW_READY raw bytes). `BootError { step, cause, rom_status, rom_error }`. `boot::power_down`. `Transport` (send/reply, poll/wait notifications, panic detection).
- Tests: `tests/boot_model.rs` (register model playing ROM + DMA + firmware), `tests/boot_sequence.rs` (write order, timeout per step, clean-up, transport), `tests/fw_image.rs` + `tests/fw_builder.rs`.

## Real firmware (not committed)

- Tarball: `https://github.com/thesofproject/sof-bin/releases/download/v2026.09.2/sof-bin-2026.09.2.tar.gz`, SHA-256 `5f7bc30b2b5c1e6b0da55a33fc44e2b7f56ae9b06952edcde614f51ef6b943e1`.
- File: `sof-bin-2026.09.2/sof/intel-signed/sof-tgl-h.ri` (`sof/sof-tgl-h.ri` is a symlink to it), 447232 bytes, SHA-256 `b9be9b8c542cb042b777438410aa169ed9b39bdc30c70edbfec41cd23ee80528`.
- `SOF_TGL_IMAGE=<that file> cargo test -p sex-sof` passes: extended manifest 768 bytes, payload 446464 bytes, SOF 2.2.0, ABI 3.22.1 (`0x03016001`), `$AM1` 2.2.0.1, 2 modules, 109 preload pages, 3 `$CPD` entries, mailboxes host `0xa0000`/8 KiB and DSP `0x81000`/4 KiB (match the TGL defaults).

## Open

- Not yet run on hardware; first real check is stage 4 on the x17 via tether.
- `Config::loader_stream`/`iccmax_stream` default to the last output/input streams; Bell must not use them while booting.
