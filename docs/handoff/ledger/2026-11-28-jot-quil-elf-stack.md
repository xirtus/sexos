# 2026-11-28: Jot and Quil take their 1 MiB stack from the ELF, committed on touch

Safe version of Gemini's Jot stack removal (from #696). Dropping `with_stack` alone
would have left Jot on whatever stack the kernel default happens to be; here the size is
stated in the program itself.

- `apps/jot/build.rs`, `apps/quil/build.rs` (new): `-z stack-size=1048576`, so the
  kernel gives the main task exactly 1 MiB with a guard page below
  (`app_resources::load_program`), committed page by page as it is touched
  (`stack_fault`).
- `_start` calls `main` directly instead of `sex_app::with_stack(1 << 20, main)`, which
  zero-filled a 1 MiB heap block (and grew the heap by 256 pages) at every start.
- Same stack size as before. `readelf -l jot`: GNU_STACK MemSiz 0x100000. No ABI change.
