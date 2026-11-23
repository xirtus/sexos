# 2026-11-23 The kernel keeps the newest 2 MiB of its log (was 64 KiB)

- `sex_pdx::crash_log::LOG_TAIL_BYTES` = 2 MiB: the kernel log ring that `OP_LOG_TAIL`
  (Spindle `hw`) reads. On the x17 R1 a USB mouse logs about a dozen lines per move, so
  64 KiB lost a Wi-Fi join's lines within seconds and `hw wifi.join` showed nothing.
- No ABI shape change (constant only); spec/abi_inputs.sha256 and the crash_log.rs
  observer pin in scripts/check_self_wait.py updated.
- Spindle `hw` no longer echoes each shown line to the log as `[spindle.hw.line]`: every
  `hw` refilled the ring with copies (the x17 photos showed only echoes, no join lines).
