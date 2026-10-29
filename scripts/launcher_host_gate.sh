#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-launcher.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/launcher.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
python3 - "$ROOT_DIR" "$TEST_DIR/production.rs" <<'PY'
from pathlib import Path
import sys,re
s=(Path(sys.argv[1])/'servers/silk-shell/src/main.rs').read_text()
parts=re.findall(r'^fn native_launcher_status\b.*?^}',s,re.M|re.S)
assert len(parts)==1
Path(sys.argv[2]).write_text(parts[0])
assert 'launcher::resolve(' in s
assert 'sex_pdx::app_lifecycle::catalog(e.slot)' in s
assert 'sex_pdx::app_lifecycle::info(e.slot)' in s
assert 'Command::FocusQuil => quil_launcher_status()' in s
assert 'quil_activation_info(true)!=Ok(active)' in s
assert 'Command::FocusSpindle => native_launcher_status(12)' in s
assert s.count('if palette_execute_selected() { toggle_command_palette(); }')==2
assert 'PALETTE_TEXT.poll(4,display_admit)' in s
assert 'palette_text_poll(); linen_text_poll();' in s
print('[launcher.host] PASS scope=exact_incarnation_policy_and_static_successor_wiring_guest_activation_separate')
PY
LAUNCHER_PRODUCTION="$TEST_DIR/production.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/launcher_authority.rs" -o "$TEST_DIR/authority"
"$TEST_DIR/authority"
