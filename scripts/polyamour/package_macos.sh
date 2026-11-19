#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Build a native macOS Polyamour.app bundle (Apple Silicon, Intel, or --universal).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

OUT_DIR="${OUT_DIR:-$ROOT/dist}"
APP_NAME="Polyamour"
APP_BUNDLE="$OUT_DIR/$APP_NAME.app"
CONTENTS="$APP_BUNDLE/Contents"
MACOS_DIR="$CONTENTS/MacOS"
RESOURCES_DIR="$CONTENTS/Resources"

ARCH="$(uname -m)"
UNIVERSAL=0
if [[ "${1:-}" == "--universal" ]]; then
    UNIVERSAL=1
    ARCH="universal"
fi

rm -rf "$APP_BUNDLE"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

if [[ "$UNIVERSAL" -eq 1 ]]; then
    echo "==> Building Polyamour for aarch64-apple-darwin and x86_64-apple-darwin..."
    cargo build --release -p polyamour --target aarch64-apple-darwin
    cargo build --release -p polyamour --target x86_64-apple-darwin
    lipo -create \
        target/aarch64-apple-darwin/release/polyamour \
        target/x86_64-apple-darwin/release/polyamour \
        -output "$MACOS_DIR/polyamour"
else
    echo "==> Building Polyamour ($ARCH)..."
    cargo build --release -p polyamour
    cp target/release/polyamour "$MACOS_DIR/polyamour"
fi

chmod +x "$MACOS_DIR/polyamour"

# Bundle Python bpy / mathutils / bmesh / polyamour bridge packages inside Resources/python
cp -R apps/polyamour/python "$RESOURCES_DIR/python"
find "$RESOURCES_DIR/python" -name "__pycache__" -type d -exec rm -rf {} +

cat > "$CONTENTS/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Polyamour</string>
    <key>CFBundleDisplayName</key>
    <string>Polyamour</string>
    <key>CFBundleIdentifier</key>
    <string>org.sexos.polyamour</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>polyamour</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
</dict>
</plist>
PLIST

if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null 2>&1 || true
fi

ZIP_PATH="$OUT_DIR/Polyamour-macos-$ARCH.zip"
rm -f "$ZIP_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_BUNDLE" "$ZIP_PATH"

echo "==> Packaged $APP_BUNDLE"
echo "==> Archive: $ZIP_PATH"
