#!/usr/bin/env bash
# Voyeur (system search) on the host: golden ranking and Abacus tests of the
# exact crates/voyeur sources, the sex_pdx::search provider wire protocol
# (an external provider's answer merged with a Shell built-in one), the
# launcher's arithmetic keys, and the Shell's hooks. Booted behaviour is
# scripts/voyeur_gate.py.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${VOYEUR_GATE_OUT:-}" ]]; then
    TEST_DIR="$VOYEUR_GATE_OUT"
    mkdir -p "$TEST_DIR"
else
    TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-voyeur.XXXXXX")"
    trap 'rm -rf "$TEST_DIR"' EXIT
fi
rustc --edition=2021 --test "$ROOT_DIR/crates/voyeur/src/lib.rs" -o "$TEST_DIR/voyeur-rank"
"$TEST_DIR/voyeur-rank"
rustc --edition=2021 -A dead_code --test "$ROOT_DIR/tests/voyeur_search.rs" -o "$TEST_DIR/voyeur-search"
"$TEST_DIR/voyeur-search"
rustc --edition=2021 -A dead_code --test "$ROOT_DIR/tests/launcher_search.rs" -o "$TEST_DIR/launcher-search"
"$TEST_DIR/launcher-search"
python3 - "$ROOT_DIR" <<'PY'
from pathlib import Path
import re,sys
root=Path(sys.argv[1])
s=(root/'servers/silk-shell/src/main.rs').read_text()
v=(root/'servers/silk-shell/src/voyeur.rs').read_text()
w=(root/'crates/sex-pdx/src/search.rs').read_text()
# The Shell's hooks, each once.
for hook in ['mod voyeur;','voyeur::refresh(PALETTE_QUERY.as_str());','voyeur::rows(&rows,&mut shown)',
             'voyeur::enter()','voyeur::step(','voyeur::launched(idx);','if voyeur::receive(&msg) { continue; }',
             'launcher_search::key_char_shifted(code, APP_MODIFIERS.key(code).shift)']:
    assert s.count(hook)==1,hook
# A program's words reach Voyeur only after the clipboard's and before the
# generic custody check.
loop=s[s.index('if clipboard_receive(&msg) { continue; }'):]
assert loop.index('voyeur::receive(&msg)')<loop.index('allows_operation(')
# Privacy: the only words Shell sends a provider are QUERY, CANCEL,
# ACTIVATE and REGISTERED, each built by sex_pdx::search.
sent=re.findall(r'wire::(\w+)\(',v)
assert {'query_packets','cancel','activate','registered'}<=set(sent),sent
assert 'records()' in v and 'HINT_CAP_EXTERNAL' in v, 'program answers are capped'
assert 'program_participant(' in v, 'only catalogued live programs register'
# Opcodes stay in their block and nothing else in the tree uses it.
ops=re.findall(r'pub const [A-Z]+: u64 = (0x3C[0-9A-F]);',w)
assert len(ops)==8 and len(set(ops))==8,ops
for path in list((root/'crates').rglob('*.rs'))+list((root/'servers').rglob('*.rs'))+list((root/'apps').rglob('*.rs')):
    if 'target' in path.parts or path.name=='search.rs': continue
    for line in path.read_text(errors='ignore').splitlines():
        if re.search(r'const\s+\w+\s*:\s*u64\s*=\s*0x3[cC][0-7]\s*;',line): raise SystemExit(f'opcode collision: {path}: {line}')
print('[voyeur.host] PASS scope=ranking_goldens_abacus_wire_protocol_two_provider_merge_shell_hooks_not_booted_pixels')
PY
