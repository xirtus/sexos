# Integrated native browser media qualification

PR #124 was merged at `828ca0d2` after all thirteen checks for its exact head
`60e096f822a240f8739575f588ab209130b53f06` passed, including media, both JIT
core counts, and the aggregate build. Its required aggregate explicitly waits
for both media and JIT acceptance.

A separate local sealed build of that exact head uses FFmpeg/dav1d and the
native AOT JavaScript engine, with the experimental JIT feature off. It includes
the integrated Geist, HiDPI, wallpaper/night-light and quiet-boot changes.
The unchanged gates ran sequentially on an otherwise idle QEMU TCG window,
one core, 1280x800, on the ARM macOS host:

| Gate | Observed result |
| --- | --- |
| Seek while downloading | Range from byte 1,094,748 of 1,482,547; resumed at eight seconds after 3,820 ms, below the unchanged 6,000 ms limit; 50 frames, no full-file wait or decoder error |
| Streaming playback | Opened at 614,400 bytes; 18 frames before download completion; all 250 frames and 480,256 audio frames; continuous 10.0 seconds of 440 Hz sound |
| Media Source playback | Independent exclusive run by the browser-gate agent passed all expected segment stages and continuous 12.01 seconds of 660 Hz sound on this same immutable ISO/Web pair |

Immutable inputs:

- ISO SHA-256: `8193a3426d6af3c83c24edab285f4a81e33804f8d7494b49c190e69f1018733b`.
- Web SHA-256: `d34bc565beff76aefcb2e927bd46930e4ef9c8afd09e169db818722aebd37128`.
- ABI manifest SHA-256: `c0e5300d63ca9b770e84a88166ed1c6ceba012015cab85399f1099c0e7a8b7e1`.

Raw serial, host output, generated clips, WAVs, exit status and provenance remain
outside Git under `Projects/sexos-evidence/pr124-integrated-native-60e0/`.
The independent MSE run is under
`Projects/sexos-evidence/browser-integrated-native/60e096f8/mse-exclusive/`.
Both seek/stream guests exited and were reaped before releasing the test window.
No timing threshold was relaxed. These are synthetic guest playback results;
real YouTube playback and x17 hardware media behavior remain unverified.
