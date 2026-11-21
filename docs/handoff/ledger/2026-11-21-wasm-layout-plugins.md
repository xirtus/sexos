# Wasm layout plugins (S4 follow-up), 2026-11-21

`silk layout WS plugin:FILE.wasm` in Spindle tiles a workspace with a WebAssembly layout plugin;
`silk layout WS plugin:spiral` uses the built-in sample.

- **The plugin runs in its own run, never in the Shell.** Spindle has Files prepare
  `/bin/wasm layout @FILE` and hands the Shell the ticket; the Shell starts that run hidden (no
  window), or reuses the run it already has for the same plugin. The Shell links no interpreter.
- **The Shell never waits on it.** On a retile it keeps the workspace's previous layout and sends
  the run the tile count and work area as a plain call; the rectangles come back as the run's own
  calls and are applied once the whole answer is in and checked.
- **Checked whole:** one rectangle per window, each inside the work area and at least the minimum
  window size. Anything else is refused with a logged reason and the previous layout stays. An
  answer for an older generation (the workspace retiled again meanwhile) is dropped.
- **Bounded guests:** each `layout(n, w, h) -> ptr` call is a fresh wasmi instance with 2 000 000
  fuel and at most 1 MiB of memory; the guest may import nothing. An endless loop ends as
  `reason=fuel`, a trap as `reason=trap`.
- **Lifecycle:** one run per plugin (at most four), closed when no workspace uses it (forced after
  5 s); a run that ends leaves its workspaces on their previous layouts. A named layout, by `silk`
  or a chord, ends a workspace's plugin.
- **Confinement:** the run may reach the plugin file (Files' `@` grants are whole files, so read
  and write) and the `wasm` program's own folder `/data/wasm`; no other files, no network; its
  program row holds what every program row holds. The guest reaches nothing but its own memory.
- **Sample:** `crates/sex-wasm/guests/layouts/spiral.wat`, a Fibonacci spiral (each window takes
  half of what is left, turning left, top, right, bottom), checked in with its `.wasm` and built
  into the `wasm` program.

Wire (`sex_pdx::wasm_layout`, new opcodes):
- `OP_LAYOUT_ASK` 0x310, Shell to plugin run: generation, count, w << 32 | h.
- `OP_LAYOUT_RECT` 0x311, plugin run to Shell: generation << 32 | index << 16 | count, x << 32 | y, w << 32 | h.
- `OP_LAYOUT_FAIL` 0x312, plugin run to Shell: generation, reason (read, module, fuel, trap, result).
- `sex_pdx::silk_ctl::Request::Plugin { workspace, ticket, key }` (kind 10), with
  `layout_plugin` (the words) and `plugin_key` (FNV-1a of the absolute path or built-in name).

Code:
- Shell: `servers/silk-shell/src/layout_plugin.rs` (pure) and `layout_plugin_glue.rs`; `silk_ctl.rs` `Act::Plugin`.
- Host: `crates/sex-wasm/src/layout.rs` (`LayoutPlugin`), `apps/wasm/src/layout_mode.rs`.
- Spindle: `silk_command.rs` and `fs_commands.rs` (`submit_layout_plugin`).

Serial records:
- `[shell.layout.plugin] ws= key= gen= n= ok=0|1 applied=|reason= kept=`
- `[shell.layout.plugin.rect] ws= gen= i= x= y= w= h=`, `[shell.layout.plugin.tile]`, `[shell.layout.plugin.ask]`
- `[shell.layout.plugin.run] key= state=starting|running|refused|timeout|ended|stopping|forced|stopped`
- `[wasm.layout.ready] what= ok= reason=`, `[wasm.layout.answer] gen= n= w= h= ok= reason= sent=`
- `[spindle.silk.plugin] ws= what= key= prepare=1`

Tests: `tests/layout_plugin.rs` and `tests/silk_ctl.rs` (host gate), `crates/sex-wasm/tests/layout.rs`.
Gate: `scripts/wasm_layout_gate.py` (CI): three calculators tiled by the sample plugin exactly as the
spiral says, then a plugin with rectangles outside the work area and one that loops forever are
refused and the windows keep the previous layout.

Not yet: read-only file grants (Files has none, so the run could write its own module), plugin
settings or parameters, gaps the Shell adds itself (the plugin owns its gaps), and keeping a
workspace's plugin across a Shell restart.
