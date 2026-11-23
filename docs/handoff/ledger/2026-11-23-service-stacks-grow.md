# 2026-11-23 Service stacks grow on touch (1 MiB each)

- **Every boot service's stack now grows on touch up to 1 MiB**, not a fixed 64 KiB.
  The x17 R1's first boots lost SexUSB (a 71 KiB `_start` frame) and SexNet (51 KiB
  `_start` plus 19 KiB in `Wifi::open`) to the 64 KiB stack: each faulted under its
  stack and ended with `dispatch_failed`. The loader still maps the top 64 KiB; the rest
  of the slot's 1 MiB under the stack top is reserved, and a not-present user touch there
  gets a zeroed page of the service's own key (`kernel/src/service_stack.rs`), in the
  page fault handler and for syscall buffers (`validate_user_bytes`), as program stacks
  already did (#306). The lowest page of the reserve is the guard page: a touch there is
  logged as `[service.stack.overflow]` and ends the service as before. Growth is logged
  as `[service.stack.grow] pd= grown_kib= depth_kib=` (first page and every 64 KiB).
- Memory is spent only where a service actually goes, so small and large machines boot
  the same services. Program slots are not registered (their stacks are in their arenas).
  Pages a run grew stay across a service restart (the run starts again at the same top).
- The per-service static stacks (SexACPI's AML stack, SexNet's 512 KiB stack) stay.
