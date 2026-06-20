#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

label="com.local.slj1660.printerapp"
printer_name="${SLJ1660_QUEUE_NAME:-SL_J1660_Local}"
port="${SLJ1660_IPP_PORT:-8631}"
serial="${SLJ1660_SERIAL:-}"
runtime_path="${SLJ1660_RUNTIME_PATH:-/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin}"
print_chunk_size="${SLJ1660_PRINT_CHUNK_SIZE:-16384}"
timeout_ms="${SLJ1660_TIMEOUT_MS:-30000}"
chunk_delay_ms="${SLJ1660_CHUNK_DELAY_MS:-0}"
launch_agents="$HOME/Library/LaunchAgents"
logs="$HOME/Library/Logs"
plist="$launch_agents/$label.plist"
binary="${SLJ1660_BINARY:-$PWD/target/release/slj1660}"
skip_build="${SLJ1660_SKIP_BUILD:-0}"
dry_run="${SLJ1660_IPP_DRY_RUN:-0}"
python_path="${SLJ1660_PYTHON:-}"

usage() {
  cat >&2 <<'EOF'
usage: scripts/install-local-ipp-printer.sh [--remove]

Installs a local LaunchAgent for `slj1660 serve-ipp` and registers a macOS
CUPS queue pointing at:

  ipp://127.0.0.1:8631/printers/slj1660

Environment overrides:
  SLJ1660_QUEUE_NAME       default: SL_J1660_Local
  SLJ1660_IPP_PORT        default: 8631
  SLJ1660_SERIAL          optional USB serial for multi-printer setups
  SLJ1660_IPP_DRY_RUN=1   daemon renders jobs but does not send USB data
  SLJ1660_RUNTIME_PATH    default: Homebrew + system binary paths
  SLJ1660_PYTHON          Python executable with Pillow installed
  SLJ1660_BINARY          default: target/release/slj1660
  SLJ1660_SKIP_BUILD=1    use SLJ1660_BINARY without running cargo build
  SLJ1660_PRINT_CHUNK_SIZE default: 16384
  SLJ1660_TIMEOUT_MS       default: 30000
  SLJ1660_CHUNK_DELAY_MS   default: 0
EOF
}

detect_python_with_pillow() {
  local candidates=()
  local current_python=""

  current_python="$(python3 -c 'import sys; print(sys.executable)' 2>/dev/null || true)"
  if [ -n "$current_python" ]; then
    candidates+=("$current_python")
  fi
  candidates+=(
    "$HOME/.pyenv/shims/python3"
    "/opt/homebrew/bin/python3"
    "/usr/local/bin/python3"
    "/usr/bin/python3"
    "/Applications/Xcode.app/Contents/Developer/usr/bin/python3"
  )

  local candidate
  for candidate in "${candidates[@]}"; do
    if [ -x "$candidate" ] && "$candidate" -c 'import PIL' >/dev/null 2>&1; then
      "$candidate" -c 'import sys; print(sys.executable)'
      return 0
    fi
  done

  return 1
}

remove_install() {
  launchctl bootout "gui/$(id -u)" "$plist" >/dev/null 2>&1 || true
  rm -f "$plist"
  lpadmin -x "$printer_name" >/dev/null 2>&1 || true
  echo "Removed LaunchAgent and CUPS queue: $printer_name"
}

if [ "${1:-}" = "--remove" ]; then
  remove_install
  exit 0
fi

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

if [ "$skip_build" = "1" ]; then
  if [ ! -x "$binary" ]; then
    echo "SLJ1660_BINARY is not executable: $binary" >&2
    exit 1
  fi
else
  cargo build --release
  if [ ! -x "$binary" ]; then
    echo "release binary was not produced: $binary" >&2
    exit 1
  fi
fi

mkdir -p "$launch_agents" "$logs"

if [ -z "$python_path" ]; then
  if ! python_path="$(detect_python_with_pillow)"; then
    cat >&2 <<'EOF'
Could not find a Python executable with Pillow installed.

Install Pillow for your active Python, for example:
  python3 -m pip install Pillow

Then re-run:
  scripts/install-local-ipp-printer.sh
EOF
    exit 1
  fi
fi

echo "Using Python for PDF rendering: $python_path"

program_args=$(
  cat <<EOF
    <string>$binary</string>
    <string>serve-ipp</string>
    <string>--bind</string>
    <string>127.0.0.1:$port</string>
    <string>--chunk-size</string>
    <string>$print_chunk_size</string>
    <string>--timeout-ms</string>
    <string>$timeout_ms</string>
    <string>--chunk-delay-ms</string>
    <string>$chunk_delay_ms</string>
EOF
)

if [ -n "$serial" ]; then
  program_args="$program_args
    <string>--serial</string>
    <string>$serial</string>"
fi

if [ "$dry_run" = "1" ]; then
  program_args="$program_args
    <string>--dry-run</string>"
fi

cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$label</string>
  <key>ProgramArguments</key>
  <array>
$program_args
  </array>
  <key>WorkingDirectory</key>
  <string>$PWD</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>$runtime_path</string>
    <key>SLJ1660_HOME</key>
    <string>$PWD</string>
    <key>SLJ1660_PYTHON</key>
    <string>$python_path</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>$logs/$label.out.log</string>
  <key>StandardErrorPath</key>
  <string>$logs/$label.err.log</string>
</dict>
</plist>
EOF

launchctl bootout "gui/$(id -u)" "$plist" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "$plist"
launchctl kickstart -k "gui/$(id -u)/$label"

for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1:$port/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

curl -fsS "http://127.0.0.1:$port/health" >/dev/null

lpadmin -p "$printer_name" \
  -E \
  -v "ipp://127.0.0.1:$port/printers/slj1660" \
  -m everywhere

cat <<EOF
Installed local SL-J1660 printer app.

Queue:
  $printer_name

Printer URI:
  ipp://127.0.0.1:$port/printers/slj1660

Logs:
  $logs/$label.out.log
  $logs/$label.err.log

Remove:
  scripts/install-local-ipp-printer.sh --remove
EOF
