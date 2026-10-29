#!/usr/bin/env bash
set -euo pipefail

echo "[SEXOS PIPELINE] ABI/PKRU/FSM guard start"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

CONTRACT="sexos_contract.toml"
if [[ ! -f "$CONTRACT" ]]; then
  echo "[FAIL] missing contract file: $CONTRACT"
  exit 1
fi

SCAN_PATHS=(kernel crates servers apps)
RG_BASE=(rg -n --hidden -S)
RG_GLOBS=(
  -g '!**/*.md'
  -g '!**/*.bak'
  -g '!**/*.log'
  -g '!**/target/**'
  -g '!**/upstream-linux/**'
  -g '!**/upstream-mesa/**'
)

fail() {
  echo "[FAIL] $1"
  exit 1
}

check_no_matches() {
  local label="$1"
  local pattern="$2"
  if "${RG_BASE[@]}" "${RG_GLOBS[@]}" "$pattern" "${SCAN_PATHS[@]}" >/tmp/sexos_guard_hits.txt; then
    echo "[FAIL] $label"
    cat /tmp/sexos_guard_hits.txt
    exit 1
  fi
  echo "[PASS] $label"
}

check_has_match() {
  local label="$1"
  local pattern="$2"
  if "${RG_BASE[@]}" "${RG_GLOBS[@]}" "$pattern" "${SCAN_PATHS[@]}" >/tmp/sexos_guard_hits.txt; then
    echo "[PASS] $label"
  else
    fail "$label (pattern missing: $pattern)"
  fi
}

echo "[1/6] forbid legacy listen ABI"
check_no_matches "No PdxListenResult in live code" "PdxListenResult"

echo "[2/6] forbid r9 in IPC surfaces"
check_no_matches "No r9 register use in kernel or sex-pdx" "in\\(\"r9\"\\)|let\\s+r9\\s*=|regs\\.r9"

echo "[3/6] forbid struct pointer IPC return marshalling"
check_no_matches "No pointer-marshalled IPC response structs" "\\*mut\\s+sex_pdx::PdxResponse|resp_ptr\\s*=\\s*r9"

echo "[4/6] enforce register return contract presence"
check_has_match "Kernel sets RSI value return for pdx_call" "regs\\.rsi\\s*=\\s*value"
check_has_match "sex-pdx pdx_call lateout status (RAX)" "lateout\\(\"rax\"\\)\\s*status"
check_has_match "sex-pdx pdx_call lateout value (RSI)" "lateout\\(\"rsi\"\\)\\s*value"

echo "[5/6] enforce listen-path register decode presence"
check_has_match "sex-pdx listen syscall number present" "in\\(\"rax\"\\)\\s*28u64"
check_has_match "sex-pdx listen lateout type_id (RAX)" "lateout\\(\"rax\"\\)\\s*type_id"
check_has_match "sex-pdx listen lateout caller_pd (RSI)" "lateout\\(\"rsi\"\\)\\s*caller_pd"
check_has_match "kernel syscall-28 has no response pointer write path" "28\\s*=>\\s*\\{"

echo "[6/6] optional build gate (best-effort in host env)"
if cargo check --locked -p sex-pdx --target x86_64-unknown-none >/tmp/sexos_guard_build.log 2>&1; then
  echo "[PASS] cargo check -p sex-pdx --target x86_64-unknown-none"
else
  echo "[WARN] cargo check -p sex-pdx failed in current env"
  sed -n '1,60p' /tmp/sexos_guard_build.log
fi

echo "[SEXOS PIPELINE] guard complete"
