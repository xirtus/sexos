#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Static CI gate: verify core microkernel servers remain on the IPC fast path
# (syscalls 102-104: pdx_call_recv, pdx_reply_recv, sched::deadline) and that
# Window::open() stays on the one-hop OP_WINDOW_OPEN_COMBINED path.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FAIL=0

require_pattern() {
    local file="$1"
    local pattern="$2"
    local desc="$3"
    if [[ ! -f "$file" ]]; then
        echo "FAIL: missing file $file ($desc)" >&2
        FAIL=1
        return
    fi
    if ! grep -Eq "$pattern" "$file"; then
        echo "FAIL: $file missing expected fast-path contract: $desc" >&2
        FAIL=1
    else
        echo "PASS: $file — $desc"
    fi
}

# 1. Core microkernel servers: syscalls 102 (pdx_call_recv), 103 (pdx_reply_recv), 104 (sched::deadline)
require_pattern "servers/sexfiles/src/trampoline.rs" "pdx_reply_recv" "sexfiles trampoline uses pdx_reply_recv (syscall 103)"
require_pattern "servers/sexfiles/src/backends/diskfs.rs" "pdx_call_recv" "sexfiles diskfs block RPCs use pdx_call_recv (syscall 102)"
require_pattern "apps/sexdrive/src/main.rs" "pdx_reply_recv" "sexdrive request-reply loop uses pdx_reply_recv (syscall 103)"
require_pattern "servers/sexdisplay/src/main.rs" "sched::deadline\(" "sexdisplay declares deadline class (syscall 104)"
require_pattern "servers/sexdisplay/src/main.rs" "OP_WINDOW_OPEN_COMBINED" "sexdisplay handles one-hop OP_WINDOW_OPEN_COMBINED"
require_pattern "servers/silk-shell/src/main.rs" "OP_WINDOW_OPEN_COMBINED" "silk-shell dispatches one-hop OP_WINDOW_OPEN_COMBINED"
require_pattern "servers/silk-shell/src/app_custody.rs" "0x10B" "silk-shell app_custody permits OP_WINDOW_OPEN_COMBINED (0x10B)"
require_pattern "servers/sexbell/src/main.rs" "sched::deadline\(" "sexbell declares deadline class (syscall 104)"
require_pattern "servers/sexbell/src/main.rs" "pdx_reply_recv" "sexbell synchronous replies use pdx_reply_recv (syscall 103)"
require_pattern "servers/sexinput/src/main.rs" "sched::deadline\(" "sexinput declares deadline class (syscall 104)"
require_pattern "servers/sexusb/src/main.rs" "sched::deadline\(" "sexusb declares deadline class (syscall 104)"
require_pattern "servers/cufflinks/src/store.rs" "pdx_call_recv" "cufflinks Files RPCs use pdx_call_recv (syscall 102)"

# 2. One-hop Window::open() verification in crates/sex-app/src/lib.rs
python3 - "$ROOT/crates/sex-app/src/lib.rs" <<'PY'
import re
import sys
from pathlib import Path

src = Path(sys.argv[1]).read_text(encoding="utf-8")
m = re.search(r"pub fn open\(\) -> Self \{(.*?)\n    \}", src, re.S)
if not m:
    print("FAIL: could not locate Window::open() in crates/sex-app/src/lib.rs", file=sys.stderr)
    sys.exit(1)

body = m.group(1)
if "OP_WINDOW_OPEN_COMBINED" not in body:
    print("FAIL: Window::open() does not use OP_WINDOW_OPEN_COMBINED", file=sys.stderr)
    sys.exit(1)

# Extract the primary path (everything before the `if !combined` fallback block).
primary = body.split("if !combined", 1)[0]
ipc_calls = re.findall(r"\b(?:pdx_call_checked|pdx_call|call)\s*\(", primary)
if len(ipc_calls) != 1:
    print(
        f"FAIL: Window::open() primary path must make exactly 1 IPC hop, found {len(ipc_calls)}: {ipc_calls}",
        file=sys.stderr,
    )
    sys.exit(1)

print("PASS: crates/sex-app/src/lib.rs — Window::open() primary path uses 1 IPC hop (OP_WINDOW_OPEN_COMBINED)")
PY

if [[ "$FAIL" -ne 0 ]]; then
    echo "[ipc_fastpath_ci_gate] FAILED" >&2
    exit 1
fi

echo "[ipc_fastpath_ci_gate] ALL CHECKS PASSED"
