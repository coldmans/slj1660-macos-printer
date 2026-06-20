#!/usr/bin/env bash

cd "$(dirname "$0")" || exit 1

scripts/install-macos-printer.sh "$@"
status=$?

printf '\n'
if [ "$status" -eq 0 ]; then
  printf 'SL-J1660 설치 작업이 끝났습니다.\n'
else
  printf 'SL-J1660 설치가 실패했습니다. 위 로그를 확인하세요.\n'
fi

printf '창을 닫으려면 Enter를 누르세요.'
IFS= read -r _ || true
exit "$status"
