#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

queue="${SLJ1660_QUEUE_NAME:-SL_J1660_Local}"
install_if_missing=1

usage() {
  cat >&2 <<'EOF'
usage: scripts/print-via-ipp.sh [--queue NAME] [--no-install] FILE.pdf

Sends a PDF through the local SL-J1660 IPP queue. If the queue does not exist,
the script installs the local LaunchAgent and CUPS queue first.

Environment overrides:
  SLJ1660_QUEUE_NAME       default: SL_J1660_Local
  SLJ1660_IPP_DRY_RUN=1   install/start daemon in dry-run mode when missing
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --queue)
      if [ "$#" -lt 2 ]; then
        usage
        exit 2
      fi
      queue="$2"
      shift
      ;;
    --no-install)
      install_if_missing=0
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      usage
      exit 2
      ;;
    *)
      break
      ;;
  esac
  shift
done

if [ "$#" -ne 1 ]; then
  usage
  exit 2
fi

pdf="$1"
if [ ! -f "$pdf" ]; then
  echo "PDF does not exist: $pdf" >&2
  exit 1
fi

case "$pdf" in
  *.pdf|*.PDF)
    ;;
  *)
    echo "expected a PDF file: $pdf" >&2
    exit 1
    ;;
esac

if ! lpstat -p "$queue" >/dev/null 2>&1; then
  if [ "$install_if_missing" -eq 0 ]; then
    echo "CUPS queue does not exist: $queue" >&2
    echo "Run scripts/install-local-ipp-printer.sh first." >&2
    exit 1
  fi

  echo "CUPS queue $queue is missing; installing local SL-J1660 IPP printer app..."
  SLJ1660_QUEUE_NAME="$queue" scripts/install-local-ipp-printer.sh
fi

echo "Sending $pdf to $queue..."
lp -d "$queue" "$pdf"
