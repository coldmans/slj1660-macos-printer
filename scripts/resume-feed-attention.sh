#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

serial="${SLJ1660_SERIAL:-}"
timeout_ms="${SLJ1660_TIMEOUT_MS:-30000}"
fixture="fixtures/confirm/tray-empty-or-open-resume.http"
dry_run=0
yes=0

usage() {
  cat >&2 <<'EOF'
usage: scripts/resume-feed-attention.sh [--dry-run] [--yes]

Sends the Windows-captured LEDM resume request for the SL-J1660
trayEmptyOrOpen/feed-attention state over endpoint 0x0a.

This is the software equivalent of pressing the blinking resume/continue
button in the captured Windows session. If the printer still has a buffered
job, this can cause that page to feed or print.

Environment overrides:
  SLJ1660_SERIAL      optional USB serial for multi-printer setups
  SLJ1660_TIMEOUT_MS  default: 30000
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      dry_run=1
      ;;
    --yes)
      yes=1
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

file_size() {
  wc -c < "$1" | tr -d '[:space:]'
}

run_cmd() {
  if [ "$dry_run" -eq 1 ]; then
    printf '+'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

cat <<EOF
About to send SL-J1660 feed-attention resume over LEDM/status endpoint 0x0a.

Printer serial: ${serial:-<auto>}
Fixture: $fixture
Timeout: ${timeout_ms}ms

This may resume a buffered job and can physically feed or print paper.
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

cmd=(
  cargo run -- send-raw "$fixture"
  --interface 3
  --endpoint 0x0a
  --chunk-size "$(file_size "$fixture")"
  --timeout-ms "$timeout_ms"
)
if [ -n "$serial" ]; then
  cmd+=(--serial "$serial")
fi

run_cmd "${cmd[@]}"
