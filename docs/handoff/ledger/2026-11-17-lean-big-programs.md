# 2026-11-17: Big programs build lean (#305)

- New Cargo profiles: `lean` (opt-level "s", stripped; no LTO, which cost CI ~13 min)
  and `lean-fast` (stripped, opt-level 3). A build stage picks one with
  `profile = "..."` in `sexos_build_spec.toml`; the build trace passes it to Cargo.
- Spreadeagle and Assuit build `lean` (were 12.2 and 27.4 MB; the start gate logs sizes),
  and Polyamour builds `lean-fast`. Smaller files load sooner from disk; CI's
  big-program start gate reports the launch times.
