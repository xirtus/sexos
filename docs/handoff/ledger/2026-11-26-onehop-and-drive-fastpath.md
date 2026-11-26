# One-hop `Window::open()` and Drive IPC fast path fixes (2026-11-26)

Re-lands the two speed-ups reverted from PR #821 (`877c92f1c`) after fixing their exact root causes.

## 1. One-hop `Window::open()` (`OP_WINDOW_OPEN_COMBINED`, `0x10B`)

### Root causes
1. **Jot's first text never committed (`[display.text.committed]` timed out)**:
   `apps/jot` renders via `window.show_pixels(...)` and never calls `window.show(...)`; its only text commit was the initial `window.show(b"")` inside `Window::open()`. `servers/sexdisplay`'s `OP_WINDOW_OPEN_COMBINED` handler only zeroed `surface.text_len = 0` directly without staging an empty text transaction on `TEXT_STAGES[index]`, so `commit_published_text()` never ran and `[display.text.committed]` was never emitted.
2. **Async ordering race between `SLOT_SHELL -> SLOT_DISPLAY` and direct `SLOT_DISPLAY` calls**:
   `Window::open()` previously sent `OP_WINDOW_OPEN_COMBINED` via async `pdx_call_checked(SLOT_SHELL, ...)` and returned immediately before `silk-shell` forwarded `0x10B` to `SLOT_DISPLAY`. Immediate calls to `window.title()`, `window.show()`, or `window.show_pixels()` could arrive at `sexdisplay` before `0x10B` — causing `show()` to be dropped (surface not yet in `SURFACES`) or `title()`/`show_pixels()` to be wiped when `0x10B` ran `set_program_title(surface_id, None)` and reset `PIXEL_SIZE`.
3. **Incomplete pixel reset on `0x10B`**:
   `OP_WINDOW_OPEN_COMBINED` reset `PIXEL_SIZE[k]` but omitted `PIXEL_PAGE[k] = 0; PIXEL_SHOWN[k] = (0, 0);` (which `OP_APP_PRESENT` resets).

### Fixes
- `crates/sex-app/src/lib.rs`: `Window::open()` sends `OP_WINDOW_OPEN_COMBINED` via `pdx_call_recv(SLOT_SHELL, ...)` (1 IPC hop) and waits for Shell's registration reply (`type_id == 1 && caller_pd == SHELL`, stashing any unrelated messages in `mailbox`). Because `silk-shell` enqueues `0x10B` to `SLOT_DISPLAY` before sending `registration_reply`, Display is guaranteed to process `0x10B` in strict FIFO order before any subsequent `window.title()` / `window.show()` / `window.show_pixels()`. On acceptance, `window.sequence` starts at `1`.
- `servers/silk-shell/src/main.rs`: Retries transient `ERR_SERVICE_NOT_READY` when forwarding `OP_WINDOW_OPEN_COMBINED` to `SLOT_DISPLAY`, and replies `0` on rejection so `Window::open()` falls back cleanly without blocking.
- `servers/sexdisplay/src/surface_lifecycle.rs` & `servers/sexdisplay/src/main.rs`: Added `Lifecycle::task_for(target, sid)`. `OP_WINDOW_OPEN_COMBINED` stages an empty text transaction (`sequence = 1`, `len = 0`, `hash(b"")`) on `TEXT_STAGES[index]` so the first frame publication commits it and emits `[display.text.committed]`, and resets `PIXEL_PAGE`/`PIXEL_SHOWN` alongside `PIXEL_SIZE`.
- `tests/display_text_transaction.rs`: Added `combined_open_stages_initial_empty_text_and_allows_seq2_show` host test.

## 2. Drive IPC fast path (`pdx_reply_recv` / `pdx_call_recv` / `reply_fast_*`)

### Root causes
1. **Watchdog false-positive restart of `sexdrive` (and latent bug in `sexbell`)**:
   In `kernel/src/ipc/recv.rs`, `wait_and_take_from` entered `wait_for_message` for `pdx_reply_recv` (syscall 103) without clearing `service_restart::HOLDING[pd]` via `service_restart::inbox(pd.id, false)`. Because `take_event` had set `HOLDING[DRIVE_PD] = true` on the previous request, `sexdrive` slept completely idle with `HOLDING[2] == true`. After `hang_limit(DRIVE_PD)` (45 s), `service_restart::poll_one` fired `[service.hung]`, killing and restarting `sexdrive` during boot/idle gates and interrupting in-flight saves.
2. **Missing `organize::pending()` guard in `sexfiles` trampoline fast-reply macros**:
   `reply_fast_queued!` and `reply_fast_direct!` omitted `!crate::organize::pending()` from their `idle` predicate (present in the main loop's listen guard at line 269).

### Fixes
- `kernel/src/ipc/recv.rs`: Added `if reply { crate::service_restart::inbox(pd.id, false); }` before spinning/waiting in `wait_and_take_from` (1-line kernel mechanism fix; `take_event` sets `HOLDING = true` again as soon as the next request is delivered).
- `apps/sexdrive/src/main.rs`, `servers/sexfiles/src/backends/diskfs.rs`, `servers/sexfiles/src/trampoline.rs`: Re-landed `pdx_reply_recv` in `sexdrive`, `pdx_call_recv` in `diskfs`, and `reply_fast_queued!`/`reply_fast_direct!` (including `!crate::organize::pending()`) in `sexfiles`.
- Restored host doubles and gates in `tests/sexdrive_service_loop.rs`, `tests/recording_runtime.rs`, `tests/recording_runtime_cases.rs`, `scripts/sexdrive_dma_host_gate.sh`, and `scripts/ipc_fastpath_ci_gate.sh`.
