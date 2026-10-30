#!/usr/bin/env bash
set -euo pipefail

BUILD_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/entrypoint_build.sh"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
SPEC_PATH="$ROOT_DIR/sexos_build_spec.toml"
ABI_INPUTS=(
  kernel/src/syscalls/mod.rs
  kernel/src/capability_slot.rs
  kernel/src/capability.rs
  kernel/src/boot_network_state.rs
  kernel/src/boot_network.rs
  kernel/src/pci_command.rs
  kernel/src/entropy_source.rs
  kernel/src/clock_source.rs
  kernel/src/trust_source.rs
  kernel/src/ipc_ring.rs
  crates/sex-net-config/src/lib.rs
  crates/sex-pdx/src/network_config.rs
  crates/sex-pdx/src/network_job.rs
  crates/sex-pdx/src/entropy_source.rs
  crates/sex-pdx/src/clock_source.rs
  crates/sex-pdx/src/trust_source.rs
  crates/sex-pdx/src/trust_policy.rs
  crates/sex-trust-config/src/lib.rs
  crates/sex-pdx/src/certificate_time.rs
  crates/sex-pdx/src/entropy_seed.rs
  kernel/src/boot_display_state.rs
  crates/sex-pdx/src/display_boot.rs
  kernel/src/history_recovery.rs
  crates/sex-pdx/src/lib.rs
  crates/sex-pdx/src/pagefault.rs
  crates/sex-pdx/src/display_ready.rs
  crates/sex-pdx/src/shell_ready.rs
  crates/sex-pdx/src/memory_region.rs
  crates/sex-pdx/src/paging.rs
  crates/sex-pdx/src/app_lifecycle.rs
  crates/sex-pdx/src/spindle_view.rs
  crates/sex-pdx/src/spindle_status.rs
  crates/sex-pdx/src/spindle_history.rs
  crates/sex-pdx/src/desktop_style.rs
  crates/sex-pdx/src/text_batch.rs
  crates/sex-pdx/src/bitmap_batch.rs
  crates/sex-pdx/src/document_view.rs
  crates/sex-pdx/src/document.rs
  crates/sex-pdx/src/spindle_probe.rs
  crates/sex-pdx/src/history_recovery.rs
  kernel/src/paging_state.rs
  kernel/src/app_resource_state.rs
  kernel/src/app_resources.rs
  kernel/src/fault_transaction.rs
  kernel/src/ipc/messages.rs
  kernel/src/ipc/router.rs
)

if [[ "$#" -ne 0 ]]; then
  echo "[FAIL] entrypoint takes no mode flags. deterministic single trace only."
  exit 1
fi

fail() {
  echo "[FAIL] $1"
  exit 1
}

validate_contract() {
  local contract="sexos_contract.toml"
  [[ -f "$contract" ]] || fail "missing contract: $contract"
  rg -n "^\[contract\]" "$contract" >/dev/null || fail "contract section missing"
  rg -n "^\[ipc\]" "$contract" >/dev/null || fail "ipc section missing"
  rg -n "transport\s*=\s*\"register-only\"" "$contract" >/dev/null || fail "ipc transport must be register-only"
  rg -n "legacy_struct_return\s*=\s*false" "$contract" >/dev/null || fail "legacy_struct_return must be false"
  rg -n "legacy_r9_path\s*=\s*false" "$contract" >/dev/null || fail "legacy_r9_path must be false"
  rg -n "^\[ipc\\.pdx_call\]" "$contract" >/dev/null || fail "ipc.pdx_call section missing"
  rg -n "status_reg\s*=\s*\"rax\"" "$contract" >/dev/null || fail "pdx_call status_reg must be rax"
  rg -n "value_reg\s*=\s*\"rsi\"" "$contract" >/dev/null || fail "pdx_call value_reg must be rsi"
}

validate_silk_de_gates() {
  rg -n "pub fn validate_deterministic_vectors\\(" crates/silkbar-model/src/lib.rs >/dev/null \
    || fail "silkbar-model missing validate_deterministic_vectors"
  rg -n "validate_silkbar_contract\\(\\)" servers/silkbar/src/main.rs >/dev/null \
    || fail "silkbar startup must enforce deterministic vectors gate"
  rg -n "validate_silkbar_contract\\(\\)" servers/sexdisplay/src/main.rs >/dev/null \
    || fail "sexdisplay startup must enforce deterministic vectors gate"
}

spec_get() {
  local key="$1"
  rg -n "^${key}\\s*=\\s*\"[^\"]+\"" "$SPEC_PATH" | head -n1 | sed -E 's/.*=[[:space:]]*"([^"]+)".*/\1/'
}

echo "[SEXOS ENTRYPOINT] BUILD_ROOT=$BUILD_ROOT"
[[ -f "$SPEC_PATH" ]] || fail "missing build spec: $SPEC_PATH"

# Check the architectural obligation bindings before compiling any image.
python3 scripts/check_architectural_truth.py

# Dependency platform selection must describe SexOS's bare-metal target. This
# checks compiler cfg as well as JSON; runtime authority is a separate gate.
python3 scripts/check_nonunix_target.py
python3 scripts/check_tls_platform.py

# 1) ABI guard, always.
./scripts/sexos_pipeline.sh

# 2) Contract validation before any compile/package/run step, then freeze snapshot.
validate_contract
validate_silk_de_gates

expected_contract_hash="$(spec_get contract_sha256)"
actual_contract_hash="$(sha256sum sexos_contract.toml | awk '{print $1}')"
[[ "$expected_contract_hash" = "$actual_contract_hash" ]] || fail "contract hash mismatch vs spec"

expected_abi_hash="$(spec_get abi_version_hash)"
actual_abi_hash="$(sha256sum "${ABI_INPUTS[@]}" | sha256sum | awk '{print $1}')"
[[ "$expected_abi_hash" = "$actual_abi_hash" ]] || fail "abi_version_hash mismatch vs spec"

SNAP_DIR=".sexos_snapshot"
mkdir -p "$SNAP_DIR"
rm -f "$SNAP_DIR/contract.snapshot.toml" "$SNAP_DIR/abi.snapshot.lock"
cp sexos_contract.toml "$SNAP_DIR/contract.snapshot.toml"
chmod 444 "$SNAP_DIR/contract.snapshot.toml"

# Freeze ABI snapshot once (entrypoint only).
{
  echo "build_root=$BUILD_ROOT"
  sha256sum "${ABI_INPUTS[@]}"
  sha256sum sexos_contract.toml
} > "$SNAP_DIR/abi.snapshot.lock"
chmod 444 "$SNAP_DIR/abi.snapshot.lock"

# 3) Seal environment and execute single linear trace.
export SEXOS_ENTRYPOINT_ACTIVE=1
export SEXOS_TRACE_ACTIVE=1
export SEXOS_BUILD_ROOT="$BUILD_ROOT"
export SEXOS_CONTRACT_SNAPSHOT="$ROOT_DIR/$SNAP_DIR/contract.snapshot.toml"
export SEXOS_ABI_SNAPSHOT="$ROOT_DIR/$SNAP_DIR/abi.snapshot.lock"
export SEXOS_BUILD_SPEC="$SPEC_PATH"
unset SEXOS_SEXNET_PROOF_PROFILE

./scripts/sexos_build_trace.sh "$SPEC_PATH"

# 4) Build-time verification: staged linen must carry sexfiles100 audit markers
# NOTE: grep without -q (with >/dev/null) avoids SIGPIPE on strings under pipefail
if ! strings iso_root/servers/linen | grep "linen.sexfiles100.audit.begin" > /dev/null; then
  fail "[SEXOS ENTRYPOINT] ERROR: staged linen missing sexfiles100 markers"
fi
echo "[SEXOS ENTRYPOINT] verification: staged linen markers present"

echo "[SEXOS ENTRYPOINT] success"
