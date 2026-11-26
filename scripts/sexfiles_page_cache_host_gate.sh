#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Host gate for sexfiles page cache, multi-stream adaptive read-ahead, and IPC v2
# endpoint request/reply dispatch.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-sexfiles-page-cache.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT

# Extract the pure (non-asm) wire format & dispatch logic from production
# crates/sex-pdx/src/endpoint.rs and servers/sexfiles/src/ipc_v2.rs so it runs
# on any host architecture (x86_64 or aarch64).
python3 - "$ROOT_DIR" "$TEST_DIR/ipc_v2_host.rs" <<'PY'
import pathlib, sys

root = pathlib.Path(sys.argv[1])
endpoint_src = (root / "crates/sex-pdx/src/endpoint.rs").read_text()
ipc_v2_src = (root / "servers/sexfiles/src/ipc_v2.rs").read_text()

# Keep everything in endpoint.rs up to `fn raw(` (all pure constants, types, and encode/decode helpers).
marker = '#[cfg(target_arch = "x86_64")]\n#[inline]\nfn raw('
cut = endpoint_src.index(marker) if marker in endpoint_src else endpoint_src.index("#[inline]\nfn raw(")
pure_endpoint = "\n".join(
    line for line in endpoint_src[:cut].splitlines() if not line.startswith("//!")
)

# Extract DispatchResult and dispatch_message from servers/sexfiles/src/ipc_v2.rs.
d_start = ipc_v2_src.index("/// Result of dispatching")
d_end = ipc_v2_src.index("/// Initializes `sexfiles`'s")
dispatch_struct = ipc_v2_src[d_start:d_end]

fn_start = ipc_v2_src.index("/// Pure IPC v2 message decoder")
fn_end = ipc_v2_src.index("/// Serves an IPC v2 endpoint")
dispatch_fn = ipc_v2_src[fn_start:fn_end]

out = "\n".join([
    pure_endpoint,
    "use core::sync::atomic::{AtomicU64, Ordering};",
    "static REQUESTS_SERVED: AtomicU64 = AtomicU64::new(0);",
    "static BAD_MESSAGES: AtomicU64 = AtomicU64::new(0);",
    dispatch_struct,
    dispatch_fn,
])
pathlib.Path(sys.argv[2]).write_text(out)
PY

echo "[sexfiles-page-cache-gate] Compiling and running host unit tests..."
SEXFILES_IPC_V2_HOST_SRC="$TEST_DIR/ipc_v2_host.rs" rustc --edition=2021 --test \
  "$ROOT_DIR/tests/sexfiles_page_cache.rs" -o "$TEST_DIR/page-cache-tests"
"$TEST_DIR/page-cache-tests" --nocapture

echo "[gate.sexfiles_page_cache_host.result] PASS"
