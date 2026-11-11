# Recorder boot reservation capacity

The first native recorder image at f44d429cfaa2cf4310bde4e495043cab9f065630 stopped before the desktop with `BOOT_FRAME_RESERVATIONS_FULL`; retained serial evidence is `sexos-evidence/desktop-recorder/native-f44d42/guest/record-serial.log`. The recorder's retained image and capture bank add boot-owned regions to the existing inventory.

Raise the fixed boot frame reservation table from 128 to 256 ranges. This adds 2,048 bytes of bounded kernel metadata and zero kernel source lines. Frame ownership is a kernel mechanism; overlap, consumed-frame, alignment, overflow and exhaustion checks remain unchanged. No dynamic allocation or capability changes. Kernel budget remains 21,500 nonblank lines.

Validation: nine actual reservation/planner host tests pass, including saturation, preservation on exhaustion, and overlap/consumed-frame rejection beyond the old limit without consuming slots. Six kernel design guardrail checks pass. Native recorder reboot and full recording workflow remain pending; this host result does not establish boot or recording success.
