#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

serial="${SLJ1660_SERIAL:-}"
timeout_ms="${SLJ1660_TIMEOUT_MS:-30000}"
print_chunk_size="${SLJ1660_PRINT_CHUNK_SIZE:-16227}"
dry_run=0
yes=0

usage() {
  cat >&2 <<'EOF'
usage: scripts/replay-confirmed-text.sh [--yes] [--dry-run]

Replays the first physically verified SL-J1660 macOS print sequence:
  1. acknowledge low-ink continue over LEDM/status endpoint 0x0a
  2. acknowledge cartridge-refilled alert over LEDM/status endpoint 0x0a
  3. replay Windows-captured text-only PCL3GUI raw stream over print endpoint 0x08
  4. acknowledge single-cartridge-mode alert over LEDM/status endpoint 0x0a

Environment overrides:
  SLJ1660_SERIAL
  SLJ1660_TIMEOUT_MS
  SLJ1660_PRINT_CHUNK_SIZE
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --yes)
      yes=1
      ;;
    --dry-run)
      dry_run=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

required_files=(
  "fixtures/confirm/lowink-continue.http"
  "fixtures/confirm/cartridge-refilled-ok.http"
  "fixtures/captured-confirmed/text-only.raw"
  "fixtures/confirm/single-cartridge-ok.http"
)

for path in "${required_files[@]}"; do
  if [ ! -f "$path" ]; then
    echo "missing required fixture: $path" >&2
    exit 1
  fi
done

run_cmd() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [ "$dry_run" -eq 0 ]; then
    "$@"
  fi
}

file_size() {
  wc -c < "$1" | tr -d '[:space:]'
}

send_ledm_request() {
  local path="$1"
  local requested_chunk_size="${2:-}"
  local chunk_size

  if [ -n "$requested_chunk_size" ]; then
    chunk_size="$requested_chunk_size"
  else
    chunk_size="$(file_size "$path")"
  fi

  local cmd=(
    cargo run -- send-raw "$path"
    --interface 3
    --endpoint 0x0a
    --chunk-size "$chunk_size"
    --timeout-ms "$timeout_ms"
  )
  if [ -n "$serial" ]; then
    cmd+=(--serial "$serial")
  fi

  run_cmd "${cmd[@]}"
}

send_print_raw() {
  local cmd=(
    cargo run -- send-raw fixtures/captured-confirmed/text-only.raw
    --chunk-size "$print_chunk_size"
    --timeout-ms "$timeout_ms"
  )
  if [ -n "$serial" ]; then
    cmd+=(--serial "$serial")
  fi

  run_cmd "${cmd[@]}"
}

cat <<EOF
About to send the confirmed SL-J1660 text-page replay sequence.

Printer serial: ${serial:-<auto>}
Print raw chunk size: $print_chunk_size
Timeout: ${timeout_ms}ms

This can physically print and consume ink/paper.
EOF

if [ "$yes" -eq 0 ] && [ "$dry_run" -eq 0 ]; then
  printf 'Continue? [y/N] '
  read -r answer
  case "$answer" in
    y|Y|yes|YES)
      ;;
    *)
      echo "aborted"
      exit 0
      ;;
  esac
fi

send_ledm_request fixtures/confirm/lowink-continue.http
send_ledm_request fixtures/confirm/cartridge-refilled-ok.http
send_print_raw
send_ledm_request fixtures/confirm/single-cartridge-ok.http 1024
