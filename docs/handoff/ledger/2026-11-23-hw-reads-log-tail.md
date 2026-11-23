# 2026-11-23 `hw` reads the newest log too (SYS_CRASH_LOG OP_LOG_TAIL)

- **`hw TEXT` finds lines logged after the 512 KiB head filled.** On the x17 R1 the head
  was full before Wi-Fi joined, so `hw dhcp` and `hw wifi.link` showed nothing. New ABI:
  `crash_log::OP_LOG_TAIL = 7` (rsi: absolute log position; copies from the kernel's
  newest `LOG_TAIL_BYTES` = 64 KiB ring; returns bytes copied and bytes logged so far).
  Spindle's `hw` walks the head, then the ring from where the head stopped (or its oldest
  byte, dropping the cut line). Its summary now reads "N of M lines from K KiB of the boot
  log (its start and newest part)".
- A log busy on another core answered 0 bytes, and `hw` printed "0 of 0 lines from the
  first 0 KiB" (seen on the x17 while the tether waited): `hw` now asks again.
