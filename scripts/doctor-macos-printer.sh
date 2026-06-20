#!/usr/bin/env bash
set -u

APP_ID="slj1660-macos-printer"
QUEUE_NAME="${SLJ1660_QUEUE_NAME:-SL_J1660_Local}"
LABEL="com.local.slj1660.printerapp"
PORT="${SLJ1660_IPP_PORT:-8631}"
INSTALL_ROOT="${SLJ1660_INSTALL_ROOT:-$HOME/Library/Application Support/$APP_ID}"

cd "$(dirname "$0")/.." 2>/dev/null || true

ok() {
  printf '[OK] %s\n' "$1"
}

warn() {
  printf '[WARN] %s\n' "$1"
}

fail() {
  printf '[FAIL] %s\n' "$1"
}

find_binary() {
  for candidate in \
    "${SLJ1660_BINARY:-}" \
    "$INSTALL_ROOT/bin/slj1660" \
    "$INSTALL_ROOT/target/release/slj1660" \
    "$PWD/target/release/slj1660"
  do
    if [ -n "${candidate:-}" ] && [ -x "$candidate" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

printf 'SL-J1660 macOS printer doctor\n\n'

if [ "$(uname -s)" = "Darwin" ]; then
  ok "macOS detected"
else
  fail "this project is macOS-only"
fi

if command -v brew >/dev/null 2>&1 || [ -x /opt/homebrew/bin/brew ] || [ -x /usr/local/bin/brew ]; then
  ok "Homebrew found"
else
  fail "Homebrew not found"
fi

for tool in pdftoppm gs curl lpadmin lpstat launchctl; do
  if command -v "$tool" >/dev/null 2>&1; then
    ok "$tool found"
  else
    fail "$tool not found"
  fi
done

if [ -x "$INSTALL_ROOT/venv/bin/python3" ]; then
  if "$INSTALL_ROOT/venv/bin/python3" -c 'import PIL' >/dev/null 2>&1; then
    ok "installer Python venv has Pillow"
  else
    fail "installer Python venv exists but Pillow import failed"
  fi
else
  warn "installer Python venv not found at $INSTALL_ROOT/venv"
fi

if binary="$(find_binary)"; then
  ok "slj1660 binary found: $binary"
  if "$binary" list-usb | grep -q '04e8:3954'; then
    ok "Samsung SL-J1660 USB VID/PID 04e8:3954 detected"
  else
    warn "SL-J1660 USB device not detected; check power, USB cable, and printer cover"
  fi
else
  fail "slj1660 binary not found; rerun Install SL-J1660.command"
fi

if launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1; then
  ok "LaunchAgent is loaded: $LABEL"
else
  fail "LaunchAgent is not loaded: $LABEL"
fi

if curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
  ok "local IPP server health check passed"
else
  fail "local IPP server health check failed: http://127.0.0.1:$PORT/health"
fi

if lpstat -p "$QUEUE_NAME" >/dev/null 2>&1; then
  ok "CUPS queue exists: $QUEUE_NAME"
else
  fail "CUPS queue is missing: $QUEUE_NAME"
fi

printf '\nLogs:\n'
printf '  %s\n' "$HOME/Library/Logs/$LABEL.out.log"
printf '  %s\n' "$HOME/Library/Logs/$LABEL.err.log"

printf '\nIf the printer prepares but does not feed paper, check paper, cartridge door, and the resume/start button state.\n'
