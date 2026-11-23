# 2026-11-23 Wi-Fi turns on memory decoding before reading the chip

- **The x17 R1's Killer AX1675w (AX210) read all ones when SexNet opened it**
  (`rf_id=0xffffffff`, `hw_rev=0xfffc`): its firmware left memory decoding off, and SexNet
  enabled it only after identifying the chip. The bogus revision went into the firmware
  start, which then failed at once (`int=0x2000002`, SW_ERR).
- SexNet now enables memory and bus mastering first, and an all-ones `HW_REV` is reported
  as `chip_reads_all_ones` instead of being normalised. The kernel wakes a leased Wi-Fi
  part to D0 as it already did for Serial IO (`[kernel.pci.class.d0] kind=Wifi`).
- A firmware error before ALIVE is now `Failed(Crashed { cause })`, not `AliveTimeout`, and
  `[wifi.failed]` prints `int=` and `gp_cntrl=` first so a narrow screen shows them.
- A new scan starts from the last scan's networks, so `wifi` lists them (with the same
  numbers) while it runs and `wifi join N` works mid-scan. On the x17 a `wifi scan`
  emptied the list for its 15 seconds and the join looked broken.
