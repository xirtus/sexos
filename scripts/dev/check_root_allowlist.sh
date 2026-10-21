#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

strict=0
include_untracked=0
for arg in "$@"; do
  case "$arg" in
    --strict) strict=1 ;;
    --include-untracked) include_untracked=1 ;;
  esac
done

# Allowlist is intentionally conservative for migration: only top-level files are checked.
# Directories are ignored here.
allowlist=(
  "AGENTS.md" "ARCHITECTURE.md" "CLAUDE.md" "HANDOFF.md" "HEARTBEAT.md" "IDENTITY.md" "LICENSE"
  "SOUL.md" "TOOLS.md" "USER.md"
  ".gitignore" ".codex" "Dockerfile"
  "Cargo.toml" "Cargo.lock" "Makefile" "rust-toolchain.toml" "x86_64-sex.json"
  "limine.cfg" "sexos_build_spec.toml" "sexos_contract.toml"
  "BOOTAA64.EFI" "BOOTIA32.EFI" "BOOTRISCV64.EFI" "BOOTX64.EFI"
  "limine-bios-cd.bin" "limine-bios-pxe.bin" "limine-bios.sys" "limine-uefi-cd.bin"
  "initrd.img" "sexos-v1.0.0.iso" "sexos-visible-checkpoint.iso" "sexos-x17r1-bundle.tar.gz"
  "dev.sh" "build_payload.sh"
  "CONTRIBUTING.md" "HERMES.md" "README.md" "SECURITY.md"
  "aarch64-sex.json" "boot_mac.sh" "skills-lock.json"
  "boot_phase20.sh" "bx.sh" "qemuX-autotest.sh" "qemuX-kbd-only.sh" "qemuX-kbd.sh" "qemuX-ps2.sh" "qemuX.sh" "run_qemu.sh" "sexos_final_boss.sh" "surgical_finality_v2.sh" "verify_fix.sh"
)

declare -A ok
for f in "${allowlist[@]}"; do
  ok["$f"]=1
done

unknown=()
check_stream() {
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    if [[ -z "${ok[$name]:-}" ]]; then
      unknown+=("$name")
    fi
  done
}

# Strict CI mode: tracked root files only.
check_stream < <(git ls-files -- . ':(top,glob)*' | awk -F/ 'NF==1' | sort -u)

# Optional local hygiene mode: also include untracked root files.
if [[ $include_untracked -eq 1 ]]; then
  check_stream < <(find . -maxdepth 1 -type f -printf '%f\n' | sort -u)
fi

if [[ ${#unknown[@]} -eq 0 ]]; then
  echo "root-allowlist: clean"
  exit 0
fi

echo "root-allowlist: found ${#unknown[@]} unallowlisted root files"
printf '%s\n' "${unknown[@]}"

if [[ $strict -eq 1 ]]; then
  exit 1
fi
