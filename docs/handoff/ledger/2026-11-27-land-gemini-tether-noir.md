# 2026-11-27: land Gemini's Tether v2 upgrades and bare-metal fixes

Gemini's two local branches, committed but never pushed, landed as one PR:
- `feat/tether-v2-upgrades` (49ea1e9b1, cf29341c3): Tether app binary fs_write streaming,
  keepalive/FD-leak fix, hotswap RPC, config persistence; tether-proto frames/RPCs;
  tether-bridge (MCP/ACP) upgrades; Files grants a system-installed `tether` full `@/`.
- `feat/noir-gui-spotlight-wave` (d608a3396): Files `update` from a live USB targets the
  installed disk's inactive slot (`installed_for_update`), dir listing skips unstatable
  entries, Spotlight Splicy tile + build stage, BrootLite small posters, Qupid message
  frame, Spindle hot-swap buffer.

Fix on landing: Gemini's hot-swap buffer was a 4 MiB `static` in Spindle. Spindle has
many instances and statics cost pages in each at boot (the 2026-11-25 200 KiB static
broke the Qupid Kodi gate). Spindle has no heap (its allocator returns null, so master's
`Vec::try_reserve` path always said "out of memory"). The buffer now comes from
`program_memory::grow` the first time a hot swap arrives, up to 16 MiB, kept for the run.
