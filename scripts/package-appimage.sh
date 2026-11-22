#!/usr/bin/env bash
# AppImage packaging script for Network Manager
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

echo "=== Network Manager AppImage Package ==="

cargo build --release --locked -p network-manager

# Get version from Cargo.toml
VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
APP_NAME="Network_Manager"
echo "Version: $VERSION"

# Create dist and build directories
mkdir -p dist/appimage
mkdir -p build/appimage

# Create AppDir structure
APPDIR="build/appimage/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/metainfo"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$APPDIR/usr/share/icons/hicolor/symbolic/apps"
mkdir -p "$APPDIR/usr/share/network-manager"

# Copy binary
cp target/release/network-manager "$APPDIR/usr/bin/"

# Copy desktop file
cp data/com.chrisdaggas.network-manager.desktop "$APPDIR/usr/share/applications/"
cp data/com.chrisdaggas.network-manager.desktop "$APPDIR/"

# Copy metainfo
cp data/com.chrisdaggas.network-manager.metainfo.xml "$APPDIR/usr/share/metainfo/"

# Copy icons
cp data/icons/hicolor/scalable/apps/com.chrisdaggas.network-manager.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
cp data/icons/hicolor/scalable/apps/com.chrisdaggas.network-manager.svg "$APPDIR/com.chrisdaggas.network-manager.svg"
if [[ -f "data/icons/hicolor/symbolic/apps/com.chrisdaggas.network-manager-symbolic.svg" ]]; then
    cp data/icons/hicolor/symbolic/apps/com.chrisdaggas.network-manager-symbolic.svg "$APPDIR/usr/share/icons/hicolor/symbolic/apps/"
fi

# Create AppRun
cat > "$APPDIR/AppRun" << 'APPRUN'
#!/bin/bash
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH:-}"
export XDG_DATA_DIRS="${HERE}/usr/share:${XDG_DATA_DIRS:-/usr/share}"
exec "${HERE}/usr/bin/network-manager" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

# Verify a downloaded file against scripts/checksums.txt.
# Fails the build if a checksum is recorded and does not match.
verify_checksum() {
    local file="$1"
    local basename
    basename=$(basename "$file")
    local checksum_file="$PROJECT_DIR/scripts/checksums.txt"
    local expected
    expected=$(awk -v name="$basename" '$1 == name { print $2 }' "$checksum_file" 2>/dev/null)
    if [[ -n "$expected" ]]; then
        local actual
        actual=$(sha256sum "$file" | awk '{print $1}')
        if [[ "$actual" != "$expected" ]]; then
            echo "ERROR: SHA-256 checksum mismatch for $basename"
            echo "  Expected: $expected"
            echo "  Actual:   $actual"
            echo "Remove or update the entry in scripts/checksums.txt if you intentionally changed versions."
            exit 1
        fi
        echo "✓ Checksum verified for $basename"
    else
        echo "WARNING: No SHA-256 checksum recorded for $basename in scripts/checksums.txt"
        echo "         Add the checksum to enable verification and protect against supply-chain attacks."
    fi
}

# Download appimagetool if needed
APPIMAGETOOL="build/appimage/appimagetool"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
if [[ ! -x "$APPIMAGETOOL" ]]; then
    echo "Downloading appimagetool..."
    wget -q -O "$APPIMAGETOOL" "$APPIMAGETOOL_URL"
    chmod +x "$APPIMAGETOOL"
    verify_checksum "$APPIMAGETOOL"
fi

# Create AppImage
echo "Creating AppImage..."
APPIMAGE_NAME="${APP_NAME}-${VERSION}-x86_64.AppImage"
ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" "dist/appimage/${APPIMAGE_NAME}"

if [[ -f "dist/appimage/${APPIMAGE_NAME}" ]]; then
    chmod +x "dist/appimage/${APPIMAGE_NAME}"
    echo "✓ AppImage created:"
    ls -lh "dist/appimage/${APPIMAGE_NAME}"
else
    echo "✗ AppImage creation failed"
    exit 1
fi

echo "=== AppImage packaging complete ==="
