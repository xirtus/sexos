# 2026-11-25 Spindle: sweep results on the heap

PR #526's Qupid Kodi gate failed with Qupid's fifth window refused for memory
(`app.program.rejected reason=no_memory`). Side by side in QEMU, master had 499 more
free pages at the same point: the `tether sweep` results buffer was a 200 KiB static,
50 pages in every Spindle at boot. It is now a Vec reserved when a sweep starts; the
gate passes locally again. (The keep-alive changes made while chasing this stay:
keep-alives are on only for Spindle's connected flows.)
