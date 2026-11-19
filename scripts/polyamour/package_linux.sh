#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Build a portable Linux tarball (Wayland + X11) for Polyamour.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

ARCH="${ARCH:-$(uname -m)}"
OUT_DIR="${OUT_DIR:-$ROOT/dist}"
STAGE_NAME="polyamour-linux-$ARCH"
STAGE_DIR="$OUT_DIR/$STAGE_NAME"

echo "==> Building Polyamour for Linux ($ARCH)..."
cargo build --release -p polyamour

rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/share/polyamour" "$STAGE_DIR/share/applications"

cp target/release/polyamour "$STAGE_DIR/bin/polyamour"
chmod +x "$STAGE_DIR/bin/polyamour"

cp -R apps/polyamour/python "$STAGE_DIR/share/polyamour/python"
find "$STAGE_DIR/share/polyamour/python" -name "__pycache__" -type d -exec rm -rf {} +
cp LICENSE "$STAGE_DIR/LICENSE"

cat > "$STAGE_DIR/share/applications/polyamour.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Polyamour
Comment=Pure-Rust 3D creation suite compatible with Blender workflows and bpy scripts
Exec=polyamour %f
Terminal=false
Categories=Graphics;3DGraphics;
MimeType=application/x-blender;model/gltf-binary;model/gltf+json;model/obj;model/stl;
DESKTOP

TAR_PATH="$OUT_DIR/$STAGE_NAME.tar.gz"
rm -f "$TAR_PATH"
tar -C "$OUT_DIR" -czf "$TAR_PATH" "$STAGE_NAME"

echo "==> Packaged $STAGE_DIR"
echo "==> Archive: $TAR_PATH"
