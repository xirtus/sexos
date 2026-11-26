# Program pages the file fills come unzeroed (Gemini #696, reviewed subset)

From Gemini's `perf/app-open-close-speed` (#696), rebuilt on current master.

- `program_frames::take_unzeroed()`: a frame for a caller that overwrites all of it.
  `take()` still zeroes.
- `boot_memory::program_image`: the image copy window takes unzeroed frames, copies the
  file in and zeroes only the tail past its length.
- `app_resources::load_program`: a page the segment's file bytes cover completely is
  taken unzeroed, since those bytes overwrite it right away. Partly covered and bss pages
  are still zeroed, so nothing of another run survives.
- Shell polls the pending program rebirth as soon as the REBORN hint arrives.

Left out of #696 on purpose:
- The shared read-only code cache. Frames mapped by several runs could be freed by
  Denim paging out or by a forced retire while another run still maps them, and its key
  was a 128-bit non-cryptographic hash.
- Files' last-loaded binary shortcut and the cached directory. The program store can
  change at runtime, and a cached directory broke `recording_image_loader` before (#821).
- Jot without its 1 MiB stack, `Window::open` without its first `show`, Jot skipping
  session writes, the Shell row-order flip and the kernel's immediate hint sends (the
  tick path already sends within one tick). Small gains, each a behaviour risk.
- `scripts/app_open_close_speed_gate.py`: its thresholds were set with the code cache.
