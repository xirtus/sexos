#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "${SEXOS_TRACE_ACTIVE:-0}" != "1" ]]; then
  echo "[FAIL] sexos_build_trace.sh is sealed. invoke via scripts/entrypoint_build.sh"
  exit 1
fi

if [[ "$#" -ne 1 ]]; then
  echo "[FAIL] usage: scripts/sexos_build_trace.sh <sexos_build_spec.toml>"
  exit 1
fi
SPEC="$1"
[[ -f "$SPEC" ]] || { echo "[FAIL] missing build spec: $SPEC"; exit 1; }

spec_get() {
  local key="$1"
  rg -n "^${key}\\s*=\\s*\"[^\"]+\"" "$SPEC" | head -n1 | sed -E 's/.*=[[:space:]]*"([^"]+)".*/\1/'
}

TARGET="$(spec_get target)"
[[ -n "$TARGET" ]] || { echo "[FAIL] missing target in spec"; exit 1; }

while IFS= read -r forbidden; do
  if env | rg -n "^${forbidden}=" >/dev/null; then
    echo "[FAIL] forbidden branching env detected: ${forbidden}"
    exit 1
  fi
done < <(sed -n '/^vars = \[/,/^\]/p' "$SPEC" | rg -o '"[^"]+"' | tr -d '"')

while IFS= read -r prefix; do
  if env | rg -n "^${prefix}" >/dev/null; then
    echo "[FAIL] forbidden conditional compilation surface: ${prefix}*"
    exit 1
  fi
done < <(sed -n '/^prefixes = \[/,/^\]/p' "$SPEC" | rg -o '"[^"]+"' | tr -d '"')

allowed_crates_raw="$(sed -n '/^crates = \[/,/^\]/p' "$SPEC" | rg -o '"[^"]+"' | tr -d '"')"
is_allowed_crate() {
  local value="$1"
  grep -Fxq "$value" <<<"$allowed_crates_raw"
}

run_stage() {
  local stage="$1"
  local action
  action="$(rg -n "^action\\s*=\\s*\"[^\"]+\"" "$stage" | head -n1 | sed -E 's/.*=[[:space:]]*"([^"]+)".*/\1/')"
  [[ -n "$action" ]] || { echo "[FAIL] stage missing action"; exit 1; }

  case "$action" in
    prep_iso_root)
      rm -rf iso_root 2>/dev/null || true
      mkdir -p iso_root/servers iso_root/apps iso_root/boot/limine
      ;;
    copy_limine)
      cp limine/limine-bios-cd.bin iso_root/boot/limine/
      cp limine/limine-uefi-cd.bin iso_root/boot/limine/
      cp limine/limine-bios.sys iso_root/boot/limine/
      ;;
    cargo_pkg)
      local pkg src dst extra_rustflags
      pkg="$(rg -n '^package\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      src="$(rg -n '^source_artifact\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      dst="$(rg -n '^dest_artifact\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      extra_rustflags="$(rg -n '^rustflags\s*=\s*"[^"]*"' "$stage" | sed -E 's/.*"([^"]*)".*/\1/' || true)"
      is_allowed_crate "$pkg" || { echo "[FAIL] package not in whitelist: $pkg"; exit 1; }
      RUSTFLAGS="-C link-arg=-Tkernel/linker.ld ${extra_rustflags:-}" cargo build --locked \
        -Z build-std=core,compiler_builtins,alloc \
        -Z build-std-features=compiler-builtins-mem \
        --package "$pkg" \
        --target "$TARGET" \
        --release
      cp "$src" "$dst"
      ;;
    cargo_manifest)
      local manifest src dst extra_rustflags
      manifest="$(rg -n '^manifest\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      src="$(rg -n '^source_artifact\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      dst="$(rg -n '^dest_artifact\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      extra_rustflags="$(rg -n '^rustflags\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/' || true)"
      is_allowed_crate "$manifest" || { echo "[FAIL] manifest not in whitelist: $manifest"; exit 1; }
      RUSTFLAGS="-C relocation-model=pic -C link-arg=-pie ${extra_rustflags:-}" cargo build --locked \
        -Z build-std=core,compiler_builtins,alloc \
        -Z build-std-features=compiler-builtins-mem \
        --manifest-path "$manifest" \
        --target "$TARGET" \
        --release
      cp "$src" "$dst"
      ;;
    copy_file)
      local src dst
      src="$(rg -n '^src\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      dst="$(rg -n '^dst\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
      cp "$src" "$dst"
      ;;
    package_iso)
      rm -f sexos-v1.0.0.iso
      xorriso -as mkisofs -R -r -J \
        -b boot/limine/limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table \
        --efi-boot boot/limine/limine-uefi-cd.bin -efi-boot-part --efi-boot-image --protective-msdos-label \
        iso_root -o sexos-v1.0.0.iso
      ;;
    limine_bios_install)
      ./limine/limine bios-install sexos-v1.0.0.iso
      ;;
    *)
      echo "[FAIL] undeclared/unsupported action in spec: $action"
      exit 1
      ;;
  esac
}

# Split and execute each [[stage]] block in order declared.
tmpdir="$(mktemp -d)"
awk -v d="$tmpdir" '
  BEGIN { n=0; f="" }
  /^\[\[stage\]\]/ { n++; f=sprintf("%s/stage_%03d.toml", d, n); print > f; next }
  { if (f != "") print >> f }
' "$SPEC"

stage_count="$(ls -1 "$tmpdir"/stage_*.toml 2>/dev/null | wc -l | tr -d ' ')"
[[ "$stage_count" -gt 0 ]] || { echo "[FAIL] no stages declared in spec"; exit 1; }

echo "[SEXOS TRACE] deterministic sequence start"
for stage in "$tmpdir"/stage_*.toml; do
  sid="$(rg -n '^id\s*=\s*"[^"]+"' "$stage" | sed -E 's/.*"([^"]+)".*/\1/')"
  echo "[TRACE] stage=$sid"
  run_stage "$stage"
done

echo "[SEXOS TRACE] deterministic sequence complete"
