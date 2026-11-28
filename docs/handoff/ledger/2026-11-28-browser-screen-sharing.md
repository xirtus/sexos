# 2026-11-28: screen sharing for web pages (getDisplayMedia), stage C6 of StreamYard

Goal (Andreas): Kaleidoscope works with StreamYard, whose "Share screen" calls
`navigator.mediaDevices.getDisplayMedia`. Plan: docs/handoff/BROWSER_CAPTURE_WEBRTC_PLAN_2026_11_28.md.
Stacked on C1 (getUserMedia, PR #863).

## How it goes
- A program only asks (`sex_app::screen_share::ask`, op `OP_SHARE` ASK to Shell).
  Shell takes the ask only from the focused program right after the person pressed a
  key or clicked in it (the clipboard's rule), else answers DENIED.
- Shell opens its own picker: the window overview (every window as a live card with
  its title) and, drawn by Display over the top margin, a pill naming the asking
  program with an "Entire screen" button. Click a card or Enter shares that window;
  the button or E shares the screen; Escape says no. Clicks elsewhere do nothing.
  Programs cannot draw over it.
- Shell tells Display what is shared and for whom (`OP_SHARE_VIEW`). Display copies
  the window's own pixels (even where covered) or the whole screen, fitted into
  960x540, into the bank at most every 66 ms, rings the shared area in red, and shows a
  "Stop sharing" pill at the bottom of the screen. Only Shell answers the pill.
- Shell answers the program `OP_SHARED` STARTED (kind SCREEN/WINDOW, session), DENIED,
  BUSY (one share at a time) or STOPPED (the person pressed Stop sharing, the window
  closed, or Display restarted). The program can stop with `OP_SHARE` STOP; it also
  stops when the program ends. Display zeroes the bank when a share ends.
- Host-tested pieces in crates/sex-pdx/src/screen_share.rs (wire, picker layout, fit,
  the bank's torn-read-safe double buffer, Shell's custody): crates/sex-pdx/tests/screen_share.rs.

## Numbers taken
- Ops: `OP_SHARE = 0x330` (program -> Shell), `OP_SHARED = 0x331` (Shell -> program),
  `OP_SHARE_VIEW = 0x332` (Shell -> Display).
- Region: `REGION_ID = 0x4409`, 4 MiB; Display's writer slot `1015`, program rows'
  read-only slot `264` (next to the webcam frame bank's 1012 / 263 / 0x4408).
- sex-js: no new op. `op::CAPTURE` (40) gains the verb `display` and a fourth `use`
  flag (screen); answers add `\tSURFACE` (`monitor`/`window`); `capturegone\tscreen`.

## Kernel (tiny)
- kernel/src/boot_memory.rs `init_screen_share(display)`, called once from
  kernel/src/init.rs after `init_screen_capture`: one boot-time backing, aliased
  writable into Display at slot 1015 and read-only into every Program row at slot 264,
  exactly as `init_usb_uvc` does the webcam frame bank. No runtime mint, no new syscall.
  Boot log: `[boot.screen.share] display=.. slot=1015 client_slot=264 bytes=..`.

## Browser
- `getDisplayMedia({video, audio})`: secure pages only, not from iframes; a stream with
  one video track (`label` = window title or "Screen", `getSettings().displaySurface`
  `window`/`monitor`, width/height, frameRate 15, `contentHint` "detail"). No audio track yet.
- The browser shows no card of its own for it; the Shell asks. The page's promise
  resolves when the first shared picture came (or after 3 s).
- Each stream element shows its own source, so the camera and the shared screen are
  live at once in different `<video>`s. Track `ended` fires when the person stops in
  the Shell; the page stopping its track tells the Shell to stop.
- `--fake-screen` shares a moving test pattern without asking the Shell (for gates
  that test pages, not the Shell). The QEMU gate uses the real picker.

## Self-wait
- The Shell's retry loop for answers lives in `sex_pdx::screen_share::send`, so
  servers/silk-shell/src/main.rs's wait fingerprint (the passive observer pin) is
  unchanged; scripts/check_self_wait.py source contract and passive observer both ok,
  and the self_wait unit tests pass. No pin added.

## Tests
- Host: `cargo test -p sex-pdx --test screen_share` (CI Host regressions job),
  `cargo test -p sex-js --test display_media` (runs with the sex-js suite in CI).
- QEMU: scripts/program_web_screenshare_gate.py (CI browser daily job, after the
  capture gate): click asks, picker opens and nothing is shared, "Entire screen" shares,
  the video looks like the screen, red ring, then the camera beside it (both videos get
  frames), Stop sharing ends the track and keeps the camera, a second ask with Escape
  rejects NotAllowedError.

## Install
- Changes Shell and Display: install with a full update and reboot. Never hot-swap the
  Shell on a real machine.

## Open items
- Every program row maps the bank read-only, as with the webcam's frame bank; the
  header names the program it is for and sex-app checks it, but a hostile program
  could read pixels while a share is on. Per-program mapping needs a runtime grant.
- No sound shared (tab or system audio). No cursor drawn into the shared picture.
- One share at a time, at most 960x540, about 15 frames a second.
- A Shell restart is not handled yet (a Display restart ends the share and tells the program).
