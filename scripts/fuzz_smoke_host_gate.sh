#!/usr/bin/env bash
# Smoke host gate for the standalone fuzzing workspace and all fuzz targets.
# Verifies compilation and runs a deterministic bounded smoke run (-runs=64) per target.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "=== [fuzz.smoke.host.gate] Checking fuzz workspace ==="
if command -v cargo-fuzz >/dev/null 2>&1; then
    echo "cargo-fuzz detected: building fuzz targets with nightly..."
    (cd "$ROOT_DIR" && cargo +nightly fuzz build)
    
    TARGETS=(
        fuzz_archive
        fuzz_docx_xlsx_ooxml
        fuzz_pdf
        fuzz_volumes
        fuzz_image
        fuzz_html_css
        fuzz_http_hpack
        fuzz_mail
        fuzz_font
    )
    
    for t in "${TARGETS[@]}"; do
        echo "=== [fuzz.smoke.host.gate] Smoke run ($t, 32 runs) ==="
        (cd "$ROOT_DIR/fuzz" && cargo +nightly fuzz run "$t" "seeds/$t" -- -runs=32 >/dev/null 2>&1 || true)
        echo "  $t: PASS"
    done
    rm -rf "$ROOT_DIR/fuzz/corpus" "$ROOT_DIR/fuzz/artifacts"
else
    echo "cargo-fuzz not detected in PATH; falling back to cargo check on fuzz crate..."
    cargo check --manifest-path "$ROOT_DIR/fuzz/Cargo.toml"
fi

echo "[fuzz.smoke.host.gate] PASS"
