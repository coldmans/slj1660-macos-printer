#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

version="$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)"
arch="$(uname -m)"
name="slj1660-macos-printer-${version}-macos-${arch}"
dist_root="$PWD/dist"
bundle="$dist_root/$name"
archive="$dist_root/$name.tar.gz"

cargo build --release

rm -rf "$bundle" "$archive"
mkdir -p "$bundle/bin" "$bundle/scripts" "$bundle/fixtures" "$bundle/docs"

cp target/release/slj1660 "$bundle/bin/slj1660"
cp "Install SL-J1660.command" "$bundle/"
cp "Uninstall SL-J1660.command" "$bundle/"
cp README.md LICENSE THIRD_PARTY_NOTICES.md "$bundle/"
cp scripts/*.sh scripts/*.py "$bundle/scripts/"
cp -R fixtures/confirm "$bundle/fixtures/"
cp docs/macos-setup.md docs/architecture.md docs/protocol-notes.md "$bundle/docs/"

chmod +x "$bundle/bin/slj1660"
chmod +x "$bundle/Install SL-J1660.command"
chmod +x "$bundle/Uninstall SL-J1660.command"
chmod +x "$bundle"/scripts/*.sh
chmod +x "$bundle"/scripts/*.py

tar -C "$dist_root" -czf "$archive" "$name"

cat <<EOF
Built macOS bundle:
  $archive

User flow:
  1. Extract the archive.
  2. Double-click Install SL-J1660.command.
EOF
