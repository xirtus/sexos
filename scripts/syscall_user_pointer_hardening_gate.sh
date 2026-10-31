#!/usr/bin/env bash
# SYSCALL_USER_POINTER_HARDENING_V1 — static + optional-runtime gate
#
# Verifies that the four direct user-pointer dereference paths in the kernel
# (snapshot_ingest, snapshot_resolve, PDX_GET_DISPLAY_INFO, raw_print) now
# validate every touched page via read_pte_flags before copying/writing.
#
# Usage:
#   ./scripts/syscall_user_pointer_hardening_gate.sh
#   ./scripts/syscall_user_pointer_hardening_gate.sh <serial-log>
set -u
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT_DIR/kernel/src/syscalls/mod.rs"
LOG="${1:-}"
PASS=1

row() {
    if [ "$2" = "1" ]; then
        echo "[gate.syscall_ptr.row] $1 PASS"
    else
        echo "[gate.syscall_ptr.row] $1 FAIL"
        PASS=0
    fi
}

echo "== SYSCALL_USER_POINTER_HARDENING_V1 =="

# ── Static rows ─────────────────────────────────────────────────────────────

# 1. The page-level validator exists.
grep -q 'fn validate_user_bytes' "$SRC"
row validator_exists "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 2. It rejects non-canonical start addresses.
grep -q 'is_canonical_addr' "$SRC"
row validator_canonical "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 3. It rejects address+length wraparound.
grep -q 'checked_add((len as u64) - 1)' "$SRC"
row validator_overflow "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 3a. It validates the inclusive end address as canonical before page math.
grep -q 'is_canonical_addr(inclusive_end)' "$SRC"
row validator_end_canonical "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 4. It checks PTE present bit.
grep -q '(pte & 0x1) == 0' "$SRC"
row validator_present "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 5. It checks PTE user-accessible bit.
grep -q '(pte & 0x4) == 0' "$SRC"
row validator_user "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 6. It checks PTE writable bit when requested.
grep -q 'writable && (pte & 0x2) == 0' "$SRC"
row validator_writable "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 7. It checks PKRU access-disable and write-disable for the page PKEY.
grep -q 'pkru >> shift) & 1' "$SRC"
row validator_pkru_ad "$([ $? -eq 0 ] && echo 1 || echo 0)"
grep -q 'pkru >> (shift + 1)) & 1' "$SRC"
row validator_pkru_wd "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 8. snapshot_ingest validates read access to the full struct.
grep -q 'snapshot_ingest' "$SRC" && grep -A8 'unsafe fn snapshot_ingest' "$SRC" | grep -q 'validate_user_bytes(src as u64, size, false)'
row ingest_validates "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 9. snapshot_resolve validates write access to the full struct.
grep -A10 'fn snapshot_resolve' "$SRC" | grep -q 'validate_user_bytes(out_ptr as u64, size, true)'
row resolve_validates "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 10. The ambient PDX_GET_DISPLAY_INFO bridge (0xE3) is retired: slot 0 is
# selected only through the decoded caller-scoped bridge operations.
grep -q 'match decode_bridge_op(num)' "$SRC" && ! grep -q '0xE3 =>' "$SRC"
row display_info_retired "$([ $? -eq 0 ] && echo 1 || echo 0)"

# 11. Serial records enforce RAW_PRINT_MAX and validate read access.
grep -q 'RAW_PRINT_MAX' "$SRC"
row raw_print_max_constant "$([ $? -eq 0 ] && echo 1 || echo 0)"
grep -A8 'Some(BridgeOp::SerialRecord) =>' "$SRC" | grep -q 'len > RAW_PRINT_MAX'
row raw_print_max_check "$([ $? -eq 0 ] && echo 1 || echo 0)"
grep -A12 'Some(BridgeOp::SerialRecord) =>' "$SRC" | grep -q 'validate_user_bytes(arg0, len, false)'
row raw_print_validates "$([ $? -eq 0 ] && echo 1 || echo 0)"

# ── Runtime rows (optional, need serial log) ────────────────────────────────
if [ -n "$LOG" ] && [ -f "$LOG" ]; then
    FAULTS=$(grep -cE 'KERNEL PAGE FAULT|DOUBLE FAULT|panic|fault\.kill' "$LOG" || true)
    row runtime_zero_faults "$([ "$FAULTS" = "0" ] && echo 1 || echo 0)"
elif [ -n "$LOG" ]; then
    echo "[gate.syscall_ptr.row] runtime_log_missing FAIL"
    PASS=0
fi

if [ "$PASS" = "1" ]; then
    echo "[gate.syscall_ptr.result] PASS"
    exit 0
else
    echo "[gate.syscall_ptr.result] FAIL"
    exit 1
fi
