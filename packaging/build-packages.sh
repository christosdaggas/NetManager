#!/bin/bash
# Build script for Network Manager - RPM, DEB, and AppImage
# Copyright (C) 2026 Christos A. Daggas
# SPDX-License-Identifier: MIT

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Get script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# App configuration
APP_NAME="network-manager"
APP_ID="com.chrisdaggas.network-manager"
VERSION=$(grep -m1 'version = ' "$PROJECT_ROOT/Cargo.toml" | sed 's/.*version = "\([^"]*\)".*/\1/' | head -1)

# Determine architecture
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
    x86_64)  DEB_ARCH="amd64"  ;;
    aarch64) DEB_ARCH="arm64"  ;;
    armv7l)  DEB_ARCH="armhf"  ;;
    *)       DEB_ARCH="$HOST_ARCH" ;;
esac
ARCH="$HOST_ARCH"

# Output directories
DIST_DIR="$PROJECT_ROOT/dist"
RPM_OUT="$DIST_DIR/rpm"
DEB_OUT="$DIST_DIR/deb"
APPIMAGE_OUT="$DIST_DIR/appimage"

# Work directories (use /tmp to keep dist clean)
DEB_WORK="/tmp/nm-deb-work-$$"
APPIMAGE_WORK="/tmp/nm-appimage-work-$$"
TOOLS_DIR="/tmp/nm-tools-$$"

echo -e "${GREEN}=== Building Network Manager Packages ===${NC}"
echo "Version: $VERSION"
echo "Project Root: $PROJECT_ROOT"
echo "Output: $DIST_DIR"

# Create output directories
mkdir -p "$RPM_OUT" "$DEB_OUT" "$APPIMAGE_OUT"

# Build release binary
echo -e "\n${YELLOW}=== Building Release Binary ===${NC}"
cd "$PROJECT_ROOT"
cargo build --release --locked

# Verify binary was built
if [ ! -f "$PROJECT_ROOT/target/release/network-manager" ]; then
    echo -e "${RED}Error: Binary not found at target/release/network-manager${NC}"
    exit 1
fi
echo -e "${GREEN}Binary built successfully${NC}"

# Validate metadata files
echo -e "\n${YELLOW}=== Validating Metadata ===${NC}"
desktop-file-validate "$PROJECT_ROOT/data/${APP_ID}.desktop" || echo "Warning: Desktop file validation had issues"
appstream-util validate-relax "$PROJECT_ROOT/data/${APP_ID}.metainfo.xml" || echo "Warning: Metainfo validation had issues"

# =============================================================================
# RPM BUILD
# =============================================================================
echo -e "\n${YELLOW}=== Building RPM Package ===${NC}"
"$PROJECT_ROOT/scripts/package-rpm.sh"

# =============================================================================
# DEB BUILD
# =============================================================================
echo -e "\n${YELLOW}=== Building DEB Package ===${NC}"

DEB_PKG_DIR="$DEB_WORK/${APP_NAME}_${VERSION}_amd64"
rm -rf "$DEB_PKG_DIR"
mkdir -p "$DEB_PKG_DIR/DEBIAN"
mkdir -p "$DEB_PKG_DIR/usr/bin"
mkdir -p "$DEB_PKG_DIR/usr/share/applications"
mkdir -p "$DEB_PKG_DIR/usr/share/metainfo"
mkdir -p "$DEB_PKG_DIR/usr/share/icons/hicolor/scalable/apps"

# Install binary
install -Dm755 "$PROJECT_ROOT/target/release/network-manager" "$DEB_PKG_DIR/usr/bin/network-manager"

# Install compiled translations
install -d "$DEB_PKG_DIR/usr/share/locale"
bash "$PROJECT_ROOT/scripts/install-catalogs.sh" "$DEB_PKG_DIR/usr/share/locale"

# Install data files
install -Dm644 "$PROJECT_ROOT/data/${APP_ID}.desktop" "$DEB_PKG_DIR/usr/share/applications/${APP_ID}.desktop"
install -Dm644 "$PROJECT_ROOT/data/${APP_ID}.metainfo.xml" "$DEB_PKG_DIR/usr/share/metainfo/${APP_ID}.metainfo.xml"
install -Dm644 "$PROJECT_ROOT/data/icons/hicolor/scalable/apps/${APP_ID}.svg" \
    "$DEB_PKG_DIR/usr/share/icons/hicolor/scalable/apps/${APP_ID}.svg"

# Calculate installed size
INSTALLED_SIZE=$(du -s "$DEB_PKG_DIR" | cut -f1)

# Create control file
cat > "$DEB_PKG_DIR/DEBIAN/control" << EOF
Package: cd-network-manager
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Installed-Size: ${INSTALLED_SIZE}
Depends: libwebkit2gtk-4.1-0, network-manager, iproute2
Recommends: bubblewrap
Maintainer: Christos A. Daggas <info@chrisdaggas.com>
Homepage: https://chrisdaggas.com
Description: Network and system profile manager for Linux
 Network Manager is a Linux-native system and network profile
 manager. It allows you to create, manage, and quickly switch between
 comprehensive network and system configuration profiles.
EOF

# Build DEB
dpkg-deb --build "$DEB_PKG_DIR" "$DEB_OUT/${APP_NAME}_${VERSION}_${DEB_ARCH}.deb"
echo -e "${GREEN}DEB package built:${NC}"
ls -la "$DEB_OUT/"

# =============================================================================
# APPIMAGE BUILD
# =============================================================================
echo -e "\n${YELLOW}=== Building AppImage ===${NC}"

# Verify a downloaded file against scripts/checksums.txt.
# Fails the build if a checksum is recorded and does not match.
verify_checksum() {
    local file="$1"
    local basename
    basename=$(basename "$file")
    local checksum_file="$PROJECT_ROOT/scripts/checksums.txt"
    local expected
    expected=$(awk -v name="$basename" '$1 == name { print $2 }' "$checksum_file" 2>/dev/null)
    if [ -n "$expected" ]; then
        local actual
        actual=$(sha256sum "$file" | awk '{print $1}')
        if [ "$actual" != "$expected" ]; then
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

# Download linuxdeploy if needed
mkdir -p "$TOOLS_DIR"
LINUXDEPLOY="$TOOLS_DIR/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
if [ ! -f "$LINUXDEPLOY" ]; then
    echo "Downloading linuxdeploy..."
    wget -q -O "$LINUXDEPLOY" "$LINUXDEPLOY_URL"
    chmod +x "$LINUXDEPLOY"
    verify_checksum "$LINUXDEPLOY"
fi

# Setup AppDir
APPDIR="$APPIMAGE_WORK/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/metainfo"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$APPDIR/usr/share/locale"

# Install binary
install -Dm755 "$PROJECT_ROOT/target/release/network-manager" "$APPDIR/usr/bin/network-manager"

# Install compiled translations
bash "$PROJECT_ROOT/scripts/install-catalogs.sh" "$APPDIR/usr/share/locale"

# Install desktop and metainfo
install -Dm644 "$PROJECT_ROOT/data/${APP_ID}.desktop" "$APPDIR/${APP_ID}.desktop"
install -Dm644 "$PROJECT_ROOT/data/${APP_ID}.desktop" "$APPDIR/usr/share/applications/${APP_ID}.desktop"
install -Dm644 "$PROJECT_ROOT/data/${APP_ID}.metainfo.xml" "$APPDIR/usr/share/metainfo/${APP_ID}.metainfo.xml"

# Install icon
install -Dm644 "$PROJECT_ROOT/data/icons/hicolor/scalable/apps/${APP_ID}.svg" "$APPDIR/${APP_ID}.svg"
install -Dm644 "$PROJECT_ROOT/data/icons/hicolor/scalable/apps/${APP_ID}.svg" \
    "$APPDIR/usr/share/icons/hicolor/scalable/apps/${APP_ID}.svg"

# Create symbolic link for .DirIcon
ln -sf "${APP_ID}.svg" "$APPDIR/.DirIcon"

# Create AppRun
cat > "$APPDIR/AppRun" << 'APPRUN_EOF'
#!/bin/bash
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${HERE}/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH}"
export XDG_DATA_DIRS="${HERE}/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export GDK_BACKEND="${GDK_BACKEND:-x11,wayland}"
exec "${HERE}/usr/bin/network-manager" "$@"
APPRUN_EOF
chmod +x "$APPDIR/AppRun"

# Build AppImage
cd "$DIST_DIR"
export ARCH=x86_64
export NO_STRIP=1
export DISABLE_COPYRIGHT_FILES_DEPLOYMENT=1

"$LINUXDEPLOY" --appdir "$APPDIR" \
    --desktop-file "$APPDIR/${APP_ID}.desktop" \
    --icon-file "$APPDIR/${APP_ID}.svg" \
    --output appimage

# Find and move the AppImage
APPIMAGE_FILE=$(find "$DIST_DIR" -maxdepth 1 -name "*.AppImage" -type f | head -1)
if [ -n "$APPIMAGE_FILE" ] && [ -f "$APPIMAGE_FILE" ]; then
    mv "$APPIMAGE_FILE" "$APPIMAGE_OUT/"
    echo -e "${GREEN}AppImage built:${NC}"
    ls -la "$APPIMAGE_OUT/"
else
    echo -e "${RED}AppImage build may have failed${NC}"
fi

# =============================================================================
# CLEANUP
# =============================================================================
echo -e "\n${YELLOW}=== Cleaning up temporary files ===${NC}"
rm -rf "$DEB_WORK" "$APPIMAGE_WORK" "$TOOLS_DIR" 2>/dev/null

# =============================================================================
# SUMMARY
# =============================================================================
echo -e "\n${GREEN}=== Build Complete ===${NC}"
echo "Output directory: $DIST_DIR"
echo ""
echo "RPM packages:"
ls -la "$RPM_OUT/" 2>/dev/null || echo "  (none)"
echo ""
echo "DEB packages:"
ls -la "$DEB_OUT/" 2>/dev/null || echo "  (none)"
echo ""
echo "AppImage:"
ls -la "$APPIMAGE_OUT/" 2>/dev/null || echo "  (none)"
