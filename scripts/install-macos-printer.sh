#!/usr/bin/env bash
set -euo pipefail

APP_ID="slj1660-macos-printer"
QUEUE_NAME="${SLJ1660_QUEUE_NAME:-SL_J1660_Local}"
LABEL="com.local.slj1660.printerapp"
INSTALL_ROOT="${SLJ1660_INSTALL_ROOT:-$HOME/Library/Application Support/$APP_ID}"
VENV_DIR="${SLJ1660_VENV_DIR:-$INSTALL_ROOT/venv}"

say() {
  printf '\n==> %s\n' "$1" >&2
}

warn() {
  printf '\n주의: %s\n' "$1" >&2
}

die() {
  printf '\n설치 중단: %s\n' "$1" >&2
  exit 1
}

canonical_dir() {
  mkdir -p "$1"
  (cd "$1" && pwd -P)
}

source_root="$(cd "$(dirname "$0")/.." && pwd -P)"
install_root="$(canonical_dir "$INSTALL_ROOT")"
install_marker="$install_root/.slj1660-install-root"

if [ "$(uname -s)" != "Darwin" ]; then
  die "이 설치기는 macOS 전용입니다."
fi

ensure_safe_install_root() {
  local home_root
  home_root="$(cd "$HOME" && pwd -P)"

  case "$install_root" in
    "/"|"/Users"|"$home_root"|"$home_root/Desktop"|"$home_root/Documents"|"$home_root/Downloads"|"$home_root/Library"|"$home_root/Library/Application Support")
      die "설치 폴더가 너무 넓습니다: $install_root"
      ;;
  esac

  if [ "$source_root" != "$install_root" ] &&
     [ ! -f "$install_marker" ] &&
     find "$install_root" -mindepth 1 -maxdepth 1 | grep -q .; then
    cat >&2 <<EOF

설치 폴더가 비어있지 않고 SL-J1660 설치 marker도 없습니다:
  $install_root

기존 사용자 파일을 지우지 않기 위해 중단합니다.
다른 SLJ1660_INSTALL_ROOT를 지정하거나 폴더를 직접 정리한 뒤 다시 실행하세요.

EOF
    exit 1
  fi
}

copy_to_install_root() {
  if [ "$source_root" = "$install_root" ]; then
    touch "$install_marker"
    return 0
  fi

  ensure_safe_install_root
  say "설치 파일을 사용자 Application Support 폴더로 복사합니다"
  rsync -a --delete \
    --exclude '.git/' \
    --exclude 'target/' \
    --exclude 'dist/' \
    --exclude '__pycache__/' \
    --exclude '*.pyc' \
    --exclude 'fixtures/captured/' \
    --exclude 'venv/' \
    --exclude '.venv/' \
    --exclude '.slj1660-install-root' \
    "$source_root/" "$install_root/"
  touch "$install_marker"

  chmod +x "$install_root/Install SL-J1660.command" 2>/dev/null || true
  chmod +x "$install_root/Uninstall SL-J1660.command" 2>/dev/null || true
  chmod +x "$install_root"/scripts/*.sh 2>/dev/null || true
  chmod +x "$install_root"/scripts/*.py 2>/dev/null || true

  exec env SLJ1660_IN_INSTALL_ROOT=1 "$install_root/scripts/install-macos-printer.sh" "$@"
}

find_brew() {
  if command -v brew >/dev/null 2>&1; then
    command -v brew
    return 0
  fi
  if [ -x /opt/homebrew/bin/brew ]; then
    printf '%s\n' /opt/homebrew/bin/brew
    return 0
  fi
  if [ -x /usr/local/bin/brew ]; then
    printf '%s\n' /usr/local/bin/brew
    return 0
  fi
  return 1
}

ensure_brew() {
  local brew_bin
  if ! brew_bin="$(find_brew)"; then
    cat >&2 <<'EOF'

Homebrew가 필요합니다.
먼저 아래 명령으로 Homebrew를 설치한 뒤 이 설치기를 다시 실행하세요.

  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

EOF
    exit 1
  fi

  export PATH="$(dirname "$brew_bin"):/opt/homebrew/bin:/usr/local/bin:$PATH"
  printf '%s\n' "$brew_bin"
}

install_brew_packages() {
  local brew_bin="$1"
  say "필수 도구를 확인합니다: libusb, poppler, ghostscript"
  HOMEBREW_NO_AUTO_UPDATE=1 "$brew_bin" install libusb poppler ghostscript
}

find_python3() {
  local candidate
  for candidate in /opt/homebrew/bin/python3 /usr/local/bin/python3 "$(command -v python3 2>/dev/null || true)"; do
    if [ -n "$candidate" ] && [ -x "$candidate" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

ensure_python_with_pillow() {
  local brew_bin="$1"
  local python3_bin

  if ! python3_bin="$(find_python3)"; then
    say "Python 3를 설치합니다"
    HOMEBREW_NO_AUTO_UPDATE=1 "$brew_bin" install python >&2
    python3_bin="$(find_python3)" || die "Python 3를 찾지 못했습니다."
  fi

  if [ ! -x "$VENV_DIR/bin/python3" ]; then
    say "전용 Python 환경을 만듭니다"
    "$python3_bin" -m venv "$VENV_DIR"
  fi

  if ! "$VENV_DIR/bin/python3" -c 'import PIL' >/dev/null 2>&1; then
    say "Pillow를 전용 Python 환경에 설치합니다"
    "$VENV_DIR/bin/python3" -m pip install --upgrade pip >&2
    "$VENV_DIR/bin/python3" -m pip install Pillow >&2
  fi

  printf '%s\n' "$VENV_DIR/bin/python3"
}

ensure_cargo() {
  local brew_bin="$1"
  if command -v cargo >/dev/null 2>&1; then
    return 0
  fi

  say "Rust/Cargo가 없어 Homebrew로 Rust를 설치합니다"
  HOMEBREW_NO_AUTO_UPDATE=1 "$brew_bin" install rust >&2
  export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
  command -v cargo >/dev/null 2>&1 || die "cargo를 찾지 못했습니다."
}

prepare_binary() {
  local brew_bin="$1"
  local bundled="$install_root/bin/slj1660"
  local release="$install_root/target/release/slj1660"

  if [ -x "$bundled" ]; then
    printf '%s\n' "$bundled"
    return 0
  fi

  ensure_cargo "$brew_bin"
  say "SL-J1660 프린터 앱을 release 모드로 빌드합니다"
  cargo build --release >&2
  [ -x "$release" ] || die "release 바이너리 생성에 실패했습니다: $release"
  printf '%s\n' "$release"
}

probe_usb() {
  local binary="$1"
  say "USB 프린터 연결을 확인합니다"
  if "$binary" list-usb | tee /tmp/slj1660-install-usb.txt | grep -q '04e8:3954'; then
    printf 'SL-J1660 USB 장치를 찾았습니다.\n'
  else
    warn "현재 USB에서 SL-J1660 VID/PID 04e8:3954를 찾지 못했습니다. 설치는 계속하지만, 출력 전 프린터 전원/USB 연결을 확인하세요."
  fi
}

install_queue() {
  local binary="$1"
  local python_path="$2"

  say "macOS 로컬 프린터 큐를 설치합니다"
  env \
    SLJ1660_BINARY="$binary" \
    SLJ1660_SKIP_BUILD=1 \
    SLJ1660_PYTHON="$python_path" \
    SLJ1660_QUEUE_NAME="$QUEUE_NAME" \
    "$install_root/scripts/install-local-ipp-printer.sh"
}

print_finish() {
  cat <<EOF

설치 완료.

macOS 프린터 이름:
  $QUEUE_NAME

프린터가 안 보이면:
  scripts/doctor-macos-printer.sh

삭제:
  ./Uninstall\ SL-J1660.command

로그:
  $HOME/Library/Logs/$LABEL.out.log
  $HOME/Library/Logs/$LABEL.err.log

EOF
}

copy_to_install_root "$@"
cd "$install_root"

say "Samsung SL-J1660 macOS 프린터 설치를 시작합니다"
brew_bin="$(ensure_brew)"
export PATH="$(dirname "$brew_bin"):/opt/homebrew/bin:/usr/local/bin:$PATH"
install_brew_packages "$brew_bin"
python_path="$(ensure_python_with_pillow "$brew_bin")"
binary="$(prepare_binary "$brew_bin")"
probe_usb "$binary"
install_queue "$binary" "$python_path"
print_finish

if [ "${SLJ1660_OPEN_SETTINGS:-1}" = "1" ]; then
  open "x-apple.systempreferences:com.apple.Print-Scan-Settings.extension" >/dev/null 2>&1 || true
fi
