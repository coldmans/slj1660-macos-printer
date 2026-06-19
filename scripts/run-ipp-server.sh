#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cat <<'EOF'
Starting the SL-J1660 local IPP printer app.

Default printer URI:
  ipp://127.0.0.1:8631/printers/slj1660

Use --dry-run to render jobs into the spool directory without sending USB data.
EOF

exec cargo run -- serve-ipp "$@"
