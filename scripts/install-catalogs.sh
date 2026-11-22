#!/usr/bin/env bash
# Install the gettext catalogs used by Rust services into a package's locale tree.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
for po in "$root"/po/*.po; do
    lang=$(basename "$po" .po)
    install -d "$1/$lang/LC_MESSAGES"
    msgfmt "$po" -o "$1/$lang/LC_MESSAGES/network-manager.mo"
done
