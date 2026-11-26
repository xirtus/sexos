# Files Lazy Page Cache & Kernel Slot 6 Unconditional Grant (2026-11-26)

## 1. `sexfiles` Lazy, Heap-Sized File Data Page Cache
- **Root cause**: `servers/sexfiles` starts with a 128 KiB fixed ELF `.bss` heap (`sex-rt::heap::HEAP_BYTES`) backed by a growable service arena (`SYS_PROGRAM_MEMORY`, drawing from `program_frames`). Previously, `ahead::ensure_cache()` requested `PageCache::new_with_fallback(1024)` (4 MiB -> 1 MiB -> 256 KiB -> 64 KiB). Under tight QEMU memory, the 64 KiB (16-block) fallback fit under `HEAP_BYTES` and ate half of Files' 128 KiB fixed heap, while larger tiers drained `program_frames`. Furthermore, routing metadata `read_run` and `write_through` through `ensure_cache()` allocated the cache during mount and double-cached metadata blocks already held in `sexfs`'s own `PLACE` tree cache.
- **Fix**:
  - Exposed `sex_rt::heap_usage()` (`sex-rt/src/lib.rs`) so services can inspect their live heap usage without growing.
  - Added `HeapBudget`, `budget_for_heap()`, `PageCache::new_for_budget()`, and `PageCache::heap_bytes()` in `servers/sexfiles/src/page_cache.rs`:
    - `0` bytes allocated at boot/mount (`PAGE_CACHE: Option<PageCache> = None`, tiny static footprint).
    - Under any memory pressure (`pressure > 0`): `0` blocks (pass-through).
    - Ungrown 128 KiB fixed heap: `0` blocks if `< 64 KiB` free, otherwise capped at `<= 4` blocks (16 KiB max).
    - Grown service arena (`place()` succeeded): scales `64 -> 256 -> 512` blocks (256 KiB -> 1 MiB -> 2 MiB), always `> 128 KiB` so `data` allocates from the grown arena rather than the fixed ELF heap, while keeping QEMU `program_frames` headroom intact.
  - Re-wired `servers/sexfiles/src/fs_service.rs` (`read_data_run` / `read_data_ahead`) and `servers/sexfiles/src/backends/diskfs.rs` (`diskfs_block_read` preserves cached read-ahead streams; `diskfs_block_write` surgically invalidates overlapping blocks without triggering lazy allocation). Metadata `read_run` remains direct.
  - Added host unit tests in `tests/sexfiles_page_cache.rs` covering lazy zero-byte startup, static footprint bounds, and tight-heap / pressure / grown-arena budget capping.

## 2. Kernel Slot 6 Unconditional Grant (Files -> Shell)
- **Root cause**: `kernel/src/init.rs` only granted Files (`PD 11`) slot `6` -> Shell (`PD 3`) when `recorder_source.is_some()`, forcing non-recorder images (e.g. Vellum's image fence) to fall back.
- **Fix**: Removed the `if recorder_source.is_some()` guard in `kernel/src/init.rs` so Files unconditionally receives slot `6` -> Shell at BOOT (0 net kernel lines added).
