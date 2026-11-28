# 2026-11-28: A full Shell input queue folds wheel steps instead of dropping them

Safe version of Gemini's wheel coalescing (from #698, left out of #828 because it merged
every queued step and let opposite steps cancel, breaking the one-entry-per-wheel
contract in `tests/browser_wheel_route.rs`).

- `servers/silk-shell/src/app_input.rs` `Queue::push_scroll`: only when the 32-entry
  queue is full, a wheel step for the same window in the same direction is added to the
  last queued scroll (refused at the i8 limit, never wrapped). Below capacity, and for
  opposite steps or other windows, it is exactly `push`. Before, a stalled program lost
  every step that arrived while the queue was full.
- `servers/silk-shell/src/main.rs` `browser_wheel_input` uses `push_scroll`.
- `scripts/http_content_gate.py` wheel source contract names the new body exactly.
- New host test `full_queue_folds_a_same_direction_step_into_the_last_wheel_only`
  (`scripts/clipboard_host_gate.sh`: 33 pass). No ABI change.
