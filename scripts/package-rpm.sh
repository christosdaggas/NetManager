#!/usr/bin/env bash
# RPM packaging script for Network Manager (Fedora)
#
# Builds the RPM from packaging/rpm/network-manager.spec using a source
# tarball of the project files (tracked and untracked, honouring .gitignore)
# and writes the result to dist/rpm/.
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

SPEC="packaging/rpm/network-manager.spec"
NAME="network-manager"

echo "=== Network Manager RPM Package ==="

for cmd in git rpmbuild rpmspec; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "✗ '$cmd' is required (install: sudo dnf install git rpm-build)"
        exit 1
    fi
done

# The spec and Cargo.toml must agree on the version.
VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
SPEC_VERSION=$(rpmspec -q --srpm --qf '%{version}' "$SPEC")
if [[ "$VERSION" != "$SPEC_VERSION" ]]; then
    echo "✗ Version mismatch: Cargo.toml has $VERSION, $SPEC has $SPEC_VERSION"
    exit 1
fi
echo "Version: $VERSION"

# The spec builds with --locked so the package uses the pinned dependencies.
if [[ ! -f Cargo.lock ]]; then
    echo "✗ Cargo.lock is missing; run 'cargo generate-lockfile' first"
    exit 1
fi

TOPDIR="$(mktemp -d)"
trap 'rm -rf "$TOPDIR"' EXIT
mkdir -p "$TOPDIR"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Source tarball: every project file git would publish, skipping files that
# are deleted in the working tree but not yet removed from the index.
echo "Creating source tarball..."
git ls-files --cached --others --exclude-standard -z \
    | while IFS= read -r -d '' f; do [[ -e "$f" ]] && printf '%s\0' "$f"; done \
    | tar --null -T - \
          --transform "s,^,${NAME}-${VERSION}/," \
          -czf "$TOPDIR/SOURCES/${NAME}-${VERSION}.tar.gz"

echo "Building RPM..."
rpmbuild --define "_topdir $TOPDIR" -bb "$SPEC"

mkdir -p dist/rpm
while IFS= read -r -d '' rpm; do
    output="dist/rpm/$(basename "$rpm")"
    cp "$rpm" "$output"
    echo "✓ Created $output"
done < <(find "$TOPDIR/RPMS" -name "*.rpm" -type f -print0)
echo "=== RPM packaging complete ==="
