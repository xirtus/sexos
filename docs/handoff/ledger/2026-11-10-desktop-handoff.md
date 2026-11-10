## Desktop like Hyprland and KDE: state and handoff (2026-11-10)

Workstream E of `docs/handoff/LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md` section 6.

**Merged:**
- E1 workspaces and tiling layouts (#72)
- E2 panel window title and fuzzy launcher (#98)
- E3 notifications with text, panel notice and history (#110)

**In PR #128:** E4 wallpaper (Super+B) and night light (Super+Shift+N).

**Still to do, in suggested order:**
1. **Settings app with saved config.** Persist the wallpaper, night light, layout and keyboard choices across boots, for example as a sexfiles object read at Shell start. This is the main gap: E4 choices reset on every boot.
2. **Notification history paging.** Today the Bell window shows whole entries plus a "+N older" line. Add J/K paging over `NOTIFY_RING` (`servers/silk-shell/src/main.rs`, `notify_history_text`).
3. **Keyboard layouts and compose.** `launcher_search::key_char` and app input map set-1 scancodes as US only.
4. **Clipboard for disk programs and images.**
5. **Screenshots:** a key that saves the published frame to a file.
6. **Lock and idle.** `WmAction::Lock` (Super+Esc) is bound but does nothing.
7. **Animations:** window open, close and workspace slide.
8. **Image wallpapers and a night light schedule.**

**Lessons from E1 to E4:**
- **Source-text host gates.** Many host gates (`scripts/*_gate.sh`) compile named functions extracted from production files, so any new call inside those functions needs harness stubs. Known ones:
  - display_surface_groups, display_scene, display_frame_pipeline and display_cursor_output: add a `#[path]` mod for each new `sex_pdx` module used by `bg()` or `publish_frame`.
  - bell_app_cleanup
  - launcher_host and linen_text_delivery, which pin the Shell poll line order
  - launcher_workflow_gate, which pins the literal launcher header
  - display_scene, which pins the `publish_frame` write loop. Keep that loop unchanged when adding a fast path.
- **Opcodes.** Check that a new opcode is unused across `crates/`, `servers/` and `apps/`. 0xCB was taken by `sound_ring::OP_LISTEN`. Used here: 0xCC, 0x2D8, 0x2D9 and 0x2DA.
- **ABI pins.** After an ABI change, run `scripts/update_abi_hashes.sh`. On a merge conflict, take master's `spec/abi_inputs.sha256` first.
- **Spindle `notify <text>`.** It must keep sending `OP_BELL_NOTIFY` because `desktop_close_gate` waits for `bell.notify.recv`.
- **QEMU gates for this workstream:** `scripts/notifications_gate.py` and `scripts/desktop_look_gate.py`. They take `--iso sexos-v1.0.0.iso --source-disk <blank image> --out <dir>`.
