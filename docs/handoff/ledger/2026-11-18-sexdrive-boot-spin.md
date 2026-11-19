# 2026-11-18: SexDrive serves Files sooner at boot (#309)

- CI's boot timeline (scripts/boot_time_gate.py, medians from QEMU start) showed the
  desktop at 3.5 s but SexDrive serving Files only at 5.9 s, 2.8 s after its storage
  was up. Files and the first program wait on it.
- SexDrive no longer spins 10 million times "for the shell" before binding its frame
  (the bind is a call and waits for the shell anyway). Two new boot markers,
  `[sexdrive.boot.probed]` and `[sexdrive.boot.frame]`, split the rest of the gap
  in the boot gate.
