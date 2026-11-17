# 2026-11-17: Wi-Fi firmware pack is compressed, so it fits the boot partition again

- The firmware pack (`firmware/wifi.pack`) is now `SEXWIFW2`: the Intel `.ucode`
  files are raw DEFLATE inside it. The pack shrinks from 13.8 MB to 5.5 MB, so the
  kernel, services and firmware fit the installed disk's 32 MiB boot partition
  (`sex_pdx::fs::BOOT_SECTORS`) again. The disk layout is unchanged.
- Boot RAM also drops: Limine loads 8.3 MB less, and the network service inflates
  only its own chip's file (at most 1.75 MB) into a 2 MiB buffer.
- `sex_wifi::pack::Pack::open` inflates and checks the SHA-256 of the inflated file;
  `Pack::file` still serves stored files (the `.pnvm` files, and Bluetooth `ibt-*`
  files for Cufflinks, which the packer never compresses). `SEXWIFW1` packs from older
  boot partitions still read.
- Tests: `firmware_pack_inflates_compressed_files_and_still_reads_v1_packs`, and
  the real-pack test (`scripts/wifi_host_gate.sh`) brings up the x17 chip from the
  compressed pack.
