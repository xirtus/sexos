# 2026-11-27: unattended signed OTA over the tether, `restart`, Claude agent mode

Goal (Andreas): drive the x17 from the Mac (Claude, Gemini, Codex) without anyone at
Spindle: update, restart into the new slot, hot-swap, all over the tether.

## Spindle tether (what runs on the x17 today)
- `update` is no longer refused from the tether (`tether_command::refused`). Files still
  applies only an image signed by the release key, newer than the running system, into
  the slot that is not running, tried once with fallback. `install go`, `format`, `mkfs`
  stay refused.
- New Spindle command `restart` (Files flushes first). Kernel: Spindle's BOOT power grant
  gains `power::RESTART_ONLY` (new right, bit 32), which allows `RESTART` and nothing else
  (`hal/acpi.rs`); BOOT genesis only, no runtime mint.
- From the Mac: `python3 tether_host.py --dir logs send "update http://MAC:8791/ota"`,
  then `send restart`. After the restart the trial passes once Spindle talks to Files.

## Tether v2 app + bridge
- `ota_push` RPC (Full Control): the app calls Files `UPDATE` (new `sex_app::files::update`,
  which also returns the refusal report). Files allows `UPDATE` from the system-installed
  Tether only (`is_system_tether`: `/bin/tether`, bound to `/data/tether`); `tether` is now
  a reserved package name so no package can become it.
- Bridge `ota_push` with `dir`: streams `sexos-update.txt`, `.sig`, `sexos-boot.img` from a
  host release folder (fs_write), then asks SexOS to apply. MCP tool `sexos_ota_push`.
  CLI: `tether-bridge call ota_push '{"dir":"/path/to/release"}'`.
  Long RPCs get floors: fs_write/hotswap 600 s, ota_push 900 s.
- Claude agent mode: `tether-bridge serve --agent claude` (or the app's Agent pill /
  `/agent claude`) runs Claude Code headless (`claude -p`, stream-json in/out, partial
  messages) behind ACP. Claude gets this bridge's MCP server (`sexos-tether`, allowed via
  `--allowedTools mcp__sexos-tether`), so it can act on SexOS within the tier the person
  set; nothing else is pre-allowed. The app remembers the choice (`agent=` in tether.cfg).
  The ACP request `tether/agent {"agent":"claude"|"antigravity"}` switches at runtime.
  Codex and Gemini drive SexOS through `tether-bridge mcp` as an MCP server.

## Known gaps
- The Mac's `claude` CLI must be signed in (`claude` then `/login`); on 2026-11-27 its OAuth
  session had expired, and Claude mode reports that error in the SexOS chat.
- Restart from Tether v2 (not Spindle) still needs a route: the app holds no power right.
- Kernel hot swap is allowed for Spindle and Shell only; Tether v2's `hotswap` RPC is
  refused by the kernel until it routes through one of them.
