#!/usr/bin/env bash

APP_ID="slj1660-macos-printer"
INSTALL_ROOT="${SLJ1660_INSTALL_ROOT:-$HOME/Library/Application Support/$APP_ID}"

cd "$(dirname "$0")" || exit 1

if [ -x "$INSTALL_ROOT/scripts/install-local-ipp-printer.sh" ]; then
  "$INSTALL_ROOT/scripts/install-local-ipp-printer.sh" --remove
else
  scripts/install-local-ipp-printer.sh --remove
fi
status=$?

printf '\n'
if [ "$status" -eq 0 ]; then
  printf 'SL-J1660 로컬 프린터 큐와 LaunchAgent를 제거했습니다.\n'
  printf '설치 파일 폴더는 남겨둡니다: %s\n' "$INSTALL_ROOT"
else
  printf 'SL-J1660 제거 작업이 실패했습니다. 위 로그를 확인하세요.\n'
fi

printf '창을 닫으려면 Enter를 누르세요.'
IFS= read -r _ || true
exit "$status"
