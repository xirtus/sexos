## 2026-11-17 Limine BIOS countdown hang fixed (WS3)

- Root cause of the CI boots that stopped at "Booting automatically in 1" with no kernel output (std programs, RAID, Splicy Studio gates): Limine 7.13.3's BIOS `_pit_sleep_and_quit_on_keypress` leaves its loop on `je` when the IRQ0 tick counter equals the target. With SERIAL=yes it sleeps one tick at a time; if two PIT ticks land inside one `int 0x16` poll (a busy TCG host), the counter skips 1 and the menu waits forever. Upstream later changed it to `jae`.
- `scripts/patch_limine_bios.py` rewrites that one `je` as `jae` in stage 2: inside limine-bios-cd.bin's gzip stream (CD boots), the `limine` tool's limine-bios-hdd.h (disk/hybrid installs) and limine-bios.sys. Pinned to the ca3a5b20 binaries and their stage-2 hashes; idempotent.
- CI's Prepare Limine step and the build trace's `copy_limine` stage apply it.
- Local proof: 16 tiny-kernel ISOs booted at once while QEMU was frozen and thawed at random: stock Limine hung 3 of 96 boots exactly like CI, the patched one 0 of 96.
