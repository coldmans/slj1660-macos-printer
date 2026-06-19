#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v brew >/dev/null 2>&1; then
  echo "Homebrew is required: https://brew.sh" >&2
  exit 1
fi

brew install libusb ghostscript poppler

if ! python3 -c 'import PIL' >/dev/null 2>&1; then
  python3 -m pip install Pillow
fi

cargo build

echo "Setup complete. Try: cargo run -- list-usb"
