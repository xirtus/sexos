#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-display-text.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
from pathlib import Path
import sys
root,out=map(Path,sys.argv[1:]);s=(root/'servers/sexdisplay/src/main.rs').read_text()
a=s.index('const FONT_ASCII_5X7:');parts=[s[a:s.index('];',s.index('= [',a))+2]]
for name in ['FONT_UNICODE_5X7','FONT_UNSUPPORTED_5X7']:
 a=s.index('const '+name+':');parts.append(s[a:s.index('];',a)+2])
a=s.index('static mut PROGRAM_TITLES:');parts.append(s[a:s.index('\n',a)])
for k in ('const PROGRAM_ROWS:','static mut PIXEL_REGIONS:','static mut PIXEL_SIZE:'):
    a=s.index(k);parts.append(s[a:s.index('\n',a)])
for name in ['program_title','program_pixel_at','coverage_blend','ascii_glyph_index','scalar_glyph','toolbar_title_fg_at','surface_text_scale','apply_text_zoom','recompute_text_lines','surface_text_fg_at']:
    a=s.index('fn '+name+'(');i=s.index('{',a)+1;depth=1
    while depth:
        depth+=(s[i]=='{')-(s[i]=='}');i+=1
    parts.append(s[a:i])
out.write_text('\n'.join(parts))
PY
# Display draws text with sex-font (and ttf-parser); build it for the host once.
cargo build -q --manifest-path "$ROOT_DIR/Cargo.toml" -p sex-font
FONT_LIBS=(--extern sex_font="$ROOT_DIR/target/debug/libsex_font.rlib" -L dependency="$ROOT_DIR/target/debug/deps")
DISPLAY_TEXT_SOURCE="$TEST_DIR/production.rs" rustc --edition=2021 "${FONT_LIBS[@]}" --test "$ROOT_DIR/tests/display_text_readability.rs" -o "$TEST_DIR/tests"
# Display's glyph cache is one task's state: one test thread, as in Display.
"$TEST_DIR/tests" --test-threads=1
echo '[gate.display_text_readability.result] PASS scope=actual_layout_and_pixel_functions_not_guest_screenshot'
