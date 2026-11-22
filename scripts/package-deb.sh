#!/usr/bin/env bash
# DEB packaging script for Network Manager
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

echo "=== Network Manager DEB Package ==="

cargo build --release --locked -p network-manager

# Get version from Cargo.toml
VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
echo "Version: $VERSION"

# Create dist directory
mkdir -p dist/deb

# Install cargo-deb if needed
if ! cargo deb --version &>/dev/null; then
    echo "Installing cargo-deb..."
    cargo install cargo-deb
fi

# Generate DEB
echo "Generating DEB package..."
cargo deb -p network-manager --no-build --output dist/deb/

echo "=== DEB packaging complete ==="
