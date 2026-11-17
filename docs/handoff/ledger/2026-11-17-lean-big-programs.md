# 2026-11-17: Big programs build lean (#305)

- New Cargo profiles: `lean` (opt-level "s", fat LTO, one codegen unit, stripped)
  and `lean-fast` (the same with opt-level 3). A build stage picks one with
  `profile = "..."` in `sexos_build_spec.toml`; the build trace passes it to Cargo.
- Spreadeagle and Assuit build `lean` (local: 12.2 to 10.3 MB and 27.4 to 22.9 MB),
  and Polyamour builds `lean-fast`. Smaller files load sooner from disk; CI's
  big-program start gate reports the launch times.
