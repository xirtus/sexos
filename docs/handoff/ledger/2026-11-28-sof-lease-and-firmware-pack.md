# 2026-11-28: Tiger Lake audio DSP lease + SOF firmware pack (SOF stage 3 plumbing)

- Plan: `docs/handoff/SOF_DMIC_PLAN_2026_11_28.md`, stage 3. Bell gets what it needs to boot
  the cAVS 2.5 DSP; Bell itself is unchanged until stage 4.
- New `sex_pdx::audio_dsp`: `cavs`/`firmware` (only `8086:43c8` TGL-H -> `sof-tgl-h.ri`,
  `8086:a0c8` TGL-LP -> `sof-tgl.ri`), `SLOT_REGISTERS` 1015 / `REGISTERS_REGION_ID` 0x4301
  (BAR4, 1 MiB: SRAM windows end at `0x80000 + 4 * 0x20000`, Linux `sof/intel/hda.h`),
  `SLOT_DMA` 1016 / `DMA_REGION_ID` 0x4302 (1 MiB code-loader + capture pool),
  `SLOT_FIRMWARE` 1017 / `FIRMWARE_REGION_ID` 0x4303, `FIRMWARE_MODULE` `"sof.pack"`.
- Kernel (`boot_memory::hda_dsp`, called from the first HDA class lease): for a cAVS
  controller with a >=16 KiB BAR0 view (PP/ML extended capabilities) and a >=1 MiB BAR4, maps
  BAR4 read/write into Bell, adds the DMA pool and the `sof.pack` boot module read only. Logs
  `[boot.hda.dsp] pd= bar4= bytes= slot= region=` or `[boot.hda.dsp.absent] pd= reason=`
  (`not_cavs` on QEMU and every other HDA controller, which keep exactly their old lease).
- Firmware: `scripts/fetch_sof_firmware.sh` (sof-bin v2026.09.2 tarball, SHA-256 pinned, then
  `sof/intel-signed/sof-tgl{,-h}.ri` and `LICENCE.Intel`, each SHA-256 pinned);
  `scripts/make_sof_firmware_pack.py` packs them stored in the `SEXWIFW2` format
  (`sex_wifi::pack::Pack::file`). Build stage `sof_firmware`; an offline build ships an empty
  pack. `limine.cfg` carries `boot:///firmware/sof.pack` beside every Wi-Fi pack line; the
  installed boot image keeps it only when the pack exists.
- Tests: `cargo test -p sex-pdx --test audio_dsp` (CI host job),
  `scripts/tests/test_make_sof_firmware_pack.py`, `crates/sex-hwdb/tests/x17r1.rs` (TGL-LP
  takes the same Bell lease).
