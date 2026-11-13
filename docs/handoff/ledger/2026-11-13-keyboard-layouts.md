## Keyboard layouts, dead keys, AltGr and Compose (A1–A5, 2026-11-13)

Workstream A of `DAILY_DRIVER_NEXT_100_SLICES.md`. Anyone can now type their language.

**Try it.** Super+Ctrl+Space steps through US, German, French, Spanish, UK and Russian; the
panel says `Keyboard: DE  German`. In Jot, Web (page and address bar), Files names, Quil and
`std` programs' input lines: `ü ß ö ä` on a German keyboard, `é` from dead `´` then `e`, AltGr+Q
for `@`, Caps Lock for capitals only, and Compose (the Menu key, or right Alt on layouts with no
AltGr level such as US) for `o "` → `ö`, `= e` → `€`, `s s` → `ß`, `- - -` → `—`, `1 2` → `½`.

**What changed.**
- New crate `crates/sex-keymap` (no_std, no allocation): 15 layouts (us, us-intl, gb, de, fr, es,
  it, pt, se, fi, no, dk, us-dvorak, us-colemak, ru), four levels per key, dead keys as combining
  marks, Caps Lock, Compose. Tables for dead-key letters (331) and Compose sequences (715) are
  generated from Unicode decompositions by `scripts/gen_keymap_compose.py` (our own data).
- Shell keeps one `Keyboard` (`servers/silk-shell/src/keyboard_layout.rs`) and puts what each
  press typed into the key it hands the focused program. AltGr, Caps Lock and Compose never
  reach programs.
- **ABI**: `sex_pdx::app_key::Key` gains `text: Text` (`Us`, `Nothing`, `Char`). Wire bit 17 says
  the word carries text; bits 32..53 hold the scalar. Shell sends the old wire whenever the key
  types what a US keyboard types, so programs built before this keep working on US.
  `sex_app::text(key)` returns the character; `sex_app::typed` stays ASCII-only.
- Input: PS/2 right Ctrl (E0 1D) and right Alt (E0 38) now get the USB codes 0x61 and 0x64, so
  AltGr is distinct from Alt on every keyboard. Ctrl+Alt display scale also takes right Ctrl.
- `WmAction::KeyboardLayout` on Super+Ctrl+Space.

**Tests.** `cargo test -p sex-keymap` (every layout parses; US equals the old table key for key;
golden keys per layout; dead keys; Caps Lock; Compose; switching). `tests/app_input.rs` runs the
actual Shell dispatch: DE gives `z`/`ü`, dead `´`+`e` gives `é`, AltGr+Q gives `@` and AltGr itself
is not delivered, Super+Ctrl+Space is the layout action, wire round-trips with text. QEMU:
`scripts/keyboard_layout_gate.py` (types `üé@zö` in Jot on DE and checks the saved bytes).

**Next.** Shortcuts still follow key positions (Ctrl+Z is the key labelled Y on DE); the chosen
layouts come from Settings once B1 lands (A10); Spindle and the launcher still read US.
