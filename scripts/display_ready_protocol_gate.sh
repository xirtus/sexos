#!/usr/bin/env bash
# Host-only protocol tests plus startup wiring checks; no image build route.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-display-ready.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/display_ready_protocol.rs -o "$TEST_DIR/display-ready-tests"
"$TEST_DIR/display-ready-tests"
python3 - <<'PY'
from pathlib import Path
import re
import tomllib
protocol = Path('crates/sex-pdx/src/display_ready.rs').read_text()
contract = tomllib.loads(Path('sexos_contract.toml').read_text())['ipc']['display_ready']
def constant(name):
    return int(re.search(r'pub const ' + name + r': u\d+ = (0x[0-9a-fA-F]+|[0-9]+);', protocol)[1], 0)
assert contract['opcode'] == constant('OP_DISPLAY_READY') != 0
assert contract['version'] == constant('DISPLAY_READY_VERSION')
assert contract['publisher_pd'] == constant('DISPLAY_PD_ID') == 1
assert contract['arg0'] == 'protocol_version'
assert contract['reserved_arg1'] == contract['reserved_arg2'] == 0
assert contract['scope'] == 'boot-only'
print('[display.ready.contract] PASS')
display = Path('servers/sexdisplay/src/main.rs').read_text()
shell_source = Path('servers/silk-shell/src/main.rs').read_text()
shell = shell_source.split('pub extern "C" fn _start()', 1)[1]
bar = Path('servers/silkbar/src/main.rs').read_text()
init = Path('kernel/src/init.rs').read_text()
assert display.index('[sexdisplay.ready]') < display.index('ready_publisher.poll()')
assert shell.index('wait_for_display()') < shell.index('sex_rt::heap_init()')
assert shell.index('wait_for_display()') < shell.index('OP_REGISTER_WM,')
assert 'pop_startup_message().or_else(|| pdx_try_listen_raw(0))' in shell
# Snapshot requests are asynchronous; the single event consumer always drains
# the retained startup FIFO before receiving a new ordinary message.
guard = shell.index('if !startup_messages_pending() {')
guard_end = shell.index('} // End startup backlog bypass.')
assert guard < shell.index('linen_paint_surface();') < guard_end
assert guard_end < shell.index('let msg = match pop_startup_message()')
paint = shell_source.split('unsafe fn linen_paint_surface() {', 1)[1].split('\n}', 1)[0]
assert 'if !startup_messages_pending() && (!LINEN_REMOTE_FETCHED || !LINEN_REMOTE_REAL)' in paint
assert shell_source.count('linen_fetch_remote_snapshot();') == 1
assert shell_source.count('pdx_try_listen_raw(0)') == 1
assert shell_source.count('pdx_listen_raw(0)') == 1
assert re.search(r'^(?:unsafe )?fn linen_sync_reply\b', shell_source, re.M) is None
assert 'pdx_listen' not in Path('servers/silk-shell/src/linen_async.rs').read_text()
assert 'let startup_inbox =' not in shell
assert 'startup_inbox.pop().or_else(|| sex_pdx::pdx_try_listen_raw(0))' in bar
assert bar.index('wait_for_display()') < bar.index('let contract_err =')
send = bar.split('fn send_update_status', 1)[1].split('fn send_update(', 1)[0]
assert send.index('pdx_call_checked(') < send.index('result.is_ok()') < send.index('[bootgraph.edge.send')
assert 'if send_update_status(update) != 0' in bar
grants = init[init.index('[bootgraph.phase25.begin]'):init.index('[bootgraph.phase25.complete]')]
assert '(sex_pdx::SLOT_SHELL, silkshell_id,' in grants
assert '(sex_pdx::SLOT_SILKBAR, silkbar_id,' in grants
assert 'assert_eq!(sexdisp_id, sex_pdx::display_ready::DISPLAY_PD_ID' in grants
print('[display.ready.startup_wiring] PASS')
PY
echo '[display.ready.protocol.gate] PASS (host and source; runtime ordering requires QEMU)'
