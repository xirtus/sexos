# std::fs, Command, and libc select/poll/getaddrinfo/mmap (slices I5, I6)

**I5, `crates/sex-std`.** `std::fs` on the hermit target now covers more. Two things work through std itself: `create_dir_all` and file types. `metadata` now fills `st_mtim`, `st_atim` and `st_ctim` from Files' `modified`, so `modified()` returns `Ok`. The rest goes through a new `sex_std::fs` module, because std's Hermit backend refuses these calls before it ever reaches a `sys_*` call: `rename`, `copy` (std's version fails on `set_permissions` after copying), `remove_dir_all`, `canonicalize`, a `read_dir` that works on an empty folder, and `FileExt::{fsync, truncate}` for `sync_all` and `set_len`. `fsync` maps to Files SYNC. sex-std also gained `sys_pread`/`sys_pwrite`, which read or write at an offset without moving the descriptor's position. `poll` on fd 0 is now real: it is readable once a whole line has been typed or the window is closed, and the console was refactored so that non-blocking key intake and `read` share one line buffer. Stdout and stderr report POLLOUT only.

`std::process::Command` still fails with `ErrorKind::Unsupported`. std's Hermit process layer is `unsupported`, and the platform has no route for a program to start another one. `sex_std::process` documents what is missing:
- Shell accepts `OP_RUN` from Spindle only.
- A run has no parent that receives its exit status.
- There are no pipes between runs.

All three need Shell work, which was out of scope.

**I6, `crates/sex-libc`.**
- New `select`, `pselect` and `ppoll`, built on `sys_poll`.
- `getaddrinfo` now handles:
  - service names, through a well-known service table
  - `AI_NUMERICHOST`, `AI_NUMERICSERV` and `AI_CANONNAME`
  - `EAI_BADFLAGS`, `EAI_SOCKTYPE`, `EAI_SERVICE`, and node and service both null
- `getnameinfo` (numeric only, since there is no reverse DNS) and `gethostbyname`/`h_errno`.
- `gai_strerror` is complete.
- `mmap` is a library emulation. File bytes are read into page-aligned heap memory:
  - `MAP_PRIVATE` is exact.
  - `MAP_SHARED` with `PROT_WRITE` writes back at `msync`, at `munmap` and at `exit`, up to the file's current end and never past it.
  - `MAP_ANONYMOUS` returns zeroed memory.
  - `MAP_FIXED` fails with ENOTSUP.
  - `mprotect`, `madvise` and `mlock` are accepted as no-ops.
- `pread`, `pwrite`, `dup`, `fdatasync`, `sync` and `truncate`.
- The pure logic is in `posix_util.rs`, covered by 6 host tests: fd_set and poll conversion, timeouts, services, numeric hosts, and mapping lengths.

**Gate** (`scripts/program_std_gate.py`): stdhello needs 22 passing checks (was 14), chello 32 (was 21), and `chello net` 35 (was 23; it adds select on a TCP socket).

**Gaps left:**
- Files writes `mtime = 0` (`servers/sexfiles/src/fs_service.rs` `let now = 0`), so `modified()` gives the epoch on SexFS. Foreign volumes report real times.
- A mapping does not see later writes made to the file through another descriptor.
- Partial `munmap` is not supported, and neither is `dup2`.
- `std::fs::rename`, `set_len` and `sync_all` themselves stay refused by upstream std. Use `sex_std::fs`.
- `Command` needs the Shell work above.
