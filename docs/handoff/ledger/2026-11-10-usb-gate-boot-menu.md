# USB gate: select an observed stalled boot menu

Two independent exact-head CI runs failed different USB lanes without entering
the kernel. #159 run 42900439356 stopped the tablet lane after 45 seconds; #160
run 42900917718 did the same in the relative-mouse lane. Each failed lane's
entire serial file contains only Limine 7.13.3's menu/countdown. Other lanes on
the respective same image booted and passed. The underlying reason the menu
countdown stopped is not established; these are not USB driver failures or
proof that unattended hardware boot is reliable.

Retained original artifacts, verified against GitHub's archive digests:

- #159 artifact 13804990066, ZIP SHA-256
  `61e6bacb72bd5c9e3b54a559caed482837854ffa2bb476870475fb9b8d74e848`,
  local `sexos-evidence/pr159-usb-failure/sexos-usb/lane_tablet.log`.
- #160 artifact 13804626032, ZIP SHA-256
  `3d516e00ec7b635c84284e18932ca8b0df1e18d63172a4d9608d89e51b665d91`,
  local `sexos-evidence/pr160-usb-failure/sexos-usb/lane_mouse.log`.

The gate now sends one ordinary QMP Enter to the configured default only when
it observes a Limine menu/countdown for at least three seconds and has not
observed kernel loading/entry. A separate boot-actions log records the attempt.
The existing readiness deadline is unchanged, and an Enter reply is never
treated as guest readiness. No QEMU restart, source/image replacement, implicit
passing lane or extra input after observed kernel loading is introduced.
All original discovery, enumeration, producer, contact, fault, keyboard, mouse
and multi-device/hotplug requirements remain.

Six tests execute the actual Bash helpers with synthetic fixture logs: menu
recognition, already-ready suppression, one successful selection followed by
an actual readiness marker, loading/no-menu timeouts, key-delivery failure, and
successful key reply without guest readiness. Existing Bash3 row/fault helper
checks remain. These tests are not native input evidence; exact-head CI must
still run all five real USB lanes before integration. Preserve any later
failure rather than interpreting boot selection as an outcome.

## Bounded QMP acknowledgement reader

Review of the new boot-key path found an existing helper bug: its `recv()` loop
never handled EOF, so a disconnected QMP peer could spin forever instead of
respecting the readiness bound. A preserved negative control using the unchanged
prior helper reproduces that spin until an external three-second fixture timeout
(`sexos-evidence/usb-qmp-reader-control/old-reader-eof-negative.log`).

The key helper now fails on EOF/error, preserves coalesced JSON records, skips
asynchronous events and accepts only the matching command ID's acknowledgement.
One monotonic deadline covers connect, greeting, both commands and settling;
the boot waiter passes its remaining readiness budget. Keyboard-lane callers
retain the five-second helper maximum. A bounded response buffer rejects malformed
oversized records. This changes no guest or device assertion.

Four actual local Unix-socket fixture cases cover EOF, coalesced events/replies,
command rejection and a missing ACK. Together with the six boot-wait and two
existing Bash helper cases, all 12 pass in 25.986 seconds. These fixtures are not
native device evidence. The earlier controlled native menu and full five-lane
proofs remain scoped to the earlier helper; fresh native acknowledgement-path
validation and exact-head CI are required for this refinement.
