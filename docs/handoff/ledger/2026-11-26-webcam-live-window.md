# 2026-11-26 — Live USB UVC Webcam Video Window (`sexuvc` → `apps/flash`)

## Scope
Connected the live USB UVC webcam stream (`0c45:6d18`, streaming 1280x720@30fps MJPEG from `servers/sexusb` into `servers/sexuvc`) directly to SexOS's native camera app (`apps/flash`) via a lock-free, double-buffered, read-only shared frame bank so live camera video renders on screen with zero kernel-side frame copies and zero per-row physical RAM multiplication.

## Changes
1. **Lock-Free Double-Buffered Live Frame Bank (`crates/sex-pdx/src/usb_uvc.rs`)**:
   - Added `FrameBank` + `LiveFrameMeta` over a 1 MiB shared region (`FRAME_WRITER_SLOT = 1012` in `sexuvc`, `FRAME_CLIENT_SLOT = 263` in program rows, `FRAME_REGION_ID = 0x4408`, `FRAME_BYTES = 1024 * 1024`).
   - Layout: 4 KiB atomic header + 2 × 510 KiB ping-pong frame slots (`FRAME_SLOT_BYTES = 510 * 1024`).
   - `sexuvc` writes into the inactive slot bracketed by an odd/even per-slot seqlock word, then publishes `FB_ACTIVE_SLOT` and monotonic `FB_SEQ`. Clients map the region read-only (`QUERY_VIEW | READ`) and perform tear-free reads verified against the per-slot seqlock.
   - Added host unit test `frame_bank_double_buffer_publish_and_read`.

2. **Minimal BOOT Wiring (`kernel/src/boot_memory.rs`, `spec/abi_inputs.sha256`)**:
   - Extended `boot_memory::init_usb_uvc` (+11 non-blank lines, pure BOOT wiring, under the 24,500-line kernel budget) to allocate ONE 1 MiB physical backing buffer, map it writable to `sexuvc` at `FRAME_WRITER_SLOT`, and map it read-only (`QUERY_VIEW | READ`) into every `App::Program` row at `FRAME_CLIENT_SLOT` (`263 < 1024` for `program_tenancy`).
   - Re-pinned `spec/abi_inputs.sha256` via `scripts/update_abi_hashes.sh`.

3. **Live Frame Publishing (`servers/sexuvc/src/main.rs`)**:
   - Updated `UsbFramePump` to reassemble incoming USB chunks into a heap-allocated `Vec<u8>` (bounded to `FRAME_SLOT_BYTES`, no large static buffer) and publish each completed frame into `FrameBank` with `(width, height, format, fps, vendor, product, dropped)`.
   - Marks `FrameBank` active on USB camera attach and inactive on detach; QEMU `SyntheticUvcDevice` path leaves `FrameBank` inactive so existing QEMU gates stay untouched.

4. **Zero-Alloc MJPEG Decode & Aspect-Fit Scaling (`crates/sex-uvc/src/pixel.rs`)**:
   - Switched `mjpeg_to_argb` from `sex_image::decode` to `sex_image::decode_into` (powered by workspace `zune-jpeg`, zero new crates) to avoid allocating a full 1280x720 pixel buffer on every frame.
   - Added `scale_argb_fit` helper and host unit tests for YUY2→ARGB, MJPEG→ARGB decode + bounds checking, and aspect-ratio-preserving scaling.

5. **Native Live Camera Window (`apps/flash/src/main.rs`)**:
   - Opens `sex_pdx::usb_uvc::FrameBank::open_reader()` when launched without `--synthetic` or a TCP `uvc://` endpoint. If a physical USB camera is active, Flash streams live frames directly from `sexuvc`, dynamically resizes heap pixel buffers to match decoded MJPEG/YUY2/NV12 dimensions, applies live Photo Booth filters, scales with aspect-ratio preservation into the viewport, and presents via `window.show_pixels(...)`.
   - Top toolbar displays a live status dot + `LIVE USB <W>x<H> <FPS>fps` (measured real-time FPS) vs `LIVE TCP` vs `SYNTHETIC`.
   - When no physical USB camera is present (e.g. QEMU), falls back identically to `SyntheticUvcDevice` (640x480 YUY2) so `scripts/flash_gate.py` remains green.

## Verification
- `cargo test --locked -p sex-uvc` — PASS (27 tests including YUY2, MJPEG, and `scale_argb_fit`)
- `rustc --edition 2021 --test tests/sexusb_uvc.rs` — PASS (6 tests including `frame_bank_double_buffer_publish_and_read`)
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts/tests/test_kernel_design_guardrails.py` — PASS
- `python3 scripts/check_architectural_truth.py` — PASS
- `cargo check --locked -p sexuvc --target x86_64-sex.json -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem` — PASS
- `cargo check --locked -p flash --target x86_64-sex-app.json -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem` — PASS
- `cargo check --locked -p sex-kernel --target x86_64-sex.json -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem` — PASS
