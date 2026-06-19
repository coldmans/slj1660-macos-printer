#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$#" -ne 1 ]; then
  echo "usage: scripts/send-raw.sh fixtures/known-good.raw" >&2
  exit 2
fi

cargo run -- send-raw "$1"
